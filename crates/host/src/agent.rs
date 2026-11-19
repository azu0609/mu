use crate::{
    Result, api,
    bridge::Bridge,
    input, now,
    session::{End, Entry, Record, Session},
    skills::{self, Skill},
    tools,
};
use mu_wire::{
    Block, Event, Kind, Live, LiveEvent, Model, Node, Resume, Sender, Snapshot, Span, State, Stats, Status, TreeEntry,
};
use std::{
    mem,
    path::PathBuf,
    sync::{
        Arc, Mutex, MutexGuard,
        atomic::{AtomicBool, Ordering},
    },
    thread,
};

// Runs under the agent's lock, so it must not block. Returning false unsubscribes.
type Sink = Box<dyn FnMut(&Event) -> bool + Send>;

// Runs under the agent's lock, so it must not block.
pub type Changed = Arc<dyn Fn(&str) + Send + Sync>;

struct Inner {
    session: Session,
    skills: Vec<Skill>,
    live: Live,
    queued: Vec<input::Message>,
    cancel: Option<Arc<AtomicBool>>,
    status: Status,
    sinks: Vec<(u64, Sink)>,
    next_sink: u64,
    changed: Changed,
    bridge: Arc<Bridge>,
}

pub struct Agent {
    inner: Mutex<Inner>,
}

pub fn check(model: &Model) -> Result<()> {
    if model.name.is_empty() || model.context == Some(0) {
        return Err("A model needs a name, and a positive context size if any".into());
    }
    Ok(())
}

impl Inner {
    fn emit(&mut self, event: Event) {
        self.sinks.retain_mut(|(_, sink)| sink(&event));
        if matches!(event, Event::State(_) | Event::Snapshot(_)) {
            (self.changed)(&self.session.header().id);
        }
    }

    fn state(&self) -> State {
        let total = self.session.total_usage();
        let last = self.session.usage();
        State {
            status: self.status,
            title: self.session.header().title.clone(),
            model: self.session.model().clone(),
            stats: Stats {
                input: total.input,
                output: total.output,
                cached: total.cached,
                uncached_input: self.session.uncached_input(),
                context_used: last.input.saturating_add(last.output),
            },
            queued: self.queued.iter().map(|message| message.text.clone()).collect(),
        }
    }

    fn node(&self, (id, siblings): (usize, Vec<usize>)) -> Node {
        let blocks = self.session.node(id).blocks.clone();
        Node { id: id as u64, siblings: siblings.into_iter().map(|id| id as u64).collect(), blocks }
    }

    fn snapshot(&self, resume: Option<Resume>) -> Vec<Event> {
        let epoch = self.session.epoch();
        let branches = self.session.branches();
        let keep = resume.filter(|resume| resume.epoch == epoch).map_or(0, |resume| resume.blocks as usize);
        let (mut count, mut start) = (0, 0);
        while count < branches.len() && start < keep {
            start += self.session.node(branches[count].0).blocks.len();
            count += 1;
        }
        let (count, keep) = if start == keep { (count, keep) } else { (0, 0) };
        let mut events = vec![Event::Snapshot(Box::new(Snapshot {
            id: self.session.header().id.clone(),
            cwd: self.session.header().cwd.clone(),
            state: self.state(),
            skills: self.skills.iter().map(Into::into).collect(),
            epoch,
            keep: keep as u64,
        }))];
        if count < branches.len() {
            events.push(Event::Commit {
                nodes: branches.into_iter().skip(count).map(|branch| self.node(branch)).collect(),
            });
        }
        events.extend(self.live.blocks.iter().cloned().map(|block| Event::Live(LiveEvent::Push(block))));
        events.extend(self.live.notice.clone().map(|block| Event::Live(LiveEvent::Notice(block))));
        events
    }

    fn emit_state(&mut self) {
        let state = self.state();
        self.emit(Event::State(state));
    }

    fn feedback(&mut self, kind: Kind, text: impl Into<String>) {
        self.emit(Event::Feedback(Block::new(kind, text)));
    }

    fn save(&mut self) -> bool {
        match self.session.save() {
            Ok(()) => true,
            Err(e) => {
                self.feedback(Kind::Warning, format!("Session not saved: {e}"));
                false
            }
        }
    }

    fn commit(&mut self) {
        let nodes = self.session.branches().pop().map(|branch| self.node(branch)).into_iter().collect();
        self.live.clear();
        self.emit(Event::Commit { nodes });
    }

    fn move_to(&mut self, cursor: Option<usize>) -> Result<()> {
        if self.cancel.is_some() {
            return Err("Stop the agent before moving in the conversation tree".into());
        }
        if self.session.cursor() != cursor {
            self.session.record(Record::Cursor { node: cursor })?;
            self.save();
        }
        self.live.clear();
        self.queued.clear();
        self.status = Status::Idle;
        for event in self.snapshot(None) {
            self.emit(event);
        }
        Ok(())
    }

    fn drain_queue(&mut self) {
        for message in mem::take(&mut self.queued) {
            self.session.user(message.text, message.from, message.attachments);
            self.commit();
        }
    }
}

impl Agent {
    fn wrap(session: Session, skills: Vec<Skill>, changed: Changed, bridge: Arc<Bridge>) -> Arc<Self> {
        Arc::new(Self {
            inner: Mutex::new(Inner {
                skills,
                live: Live::default(),
                queued: vec![],
                cancel: None,
                status: session.status(),
                session,
                sinks: vec![],
                next_sink: 0,
                changed,
                bridge,
            }),
        })
    }

    pub fn new(cwd: PathBuf, parent: Option<String>, model: Model, changed: Changed, bridge: Arc<Bridge>) -> Arc<Self> {
        let skills = skills::skills(&cwd);
        let session = Session::new(cwd, parent, model, &skills);
        Self::wrap(session, skills, changed, bridge)
    }

    pub fn load(id: &str, changed: Changed, bridge: Arc<Bridge>) -> Result<Arc<Self>> {
        let session = Session::load(id)?;
        let skills = skills::skills(&session.header().cwd);
        Ok(Self::wrap(session, skills, changed, bridge))
    }

    fn inner(&self) -> MutexGuard<'_, Inner> {
        self.inner.lock().unwrap()
    }

    pub fn id(&self) -> String {
        self.inner().session.header().id.clone()
    }

    pub fn status(&self) -> Status {
        self.inner().status
    }

    pub fn title(&self) -> String {
        self.inner().session.header().title.clone()
    }

    pub fn parent(&self) -> Option<String> {
        self.inner().session.header().parent.clone()
    }

    pub fn model(&self) -> Model {
        self.inner().session.model().clone()
    }

    pub fn disposable(&self) -> bool {
        let inner = self.inner();
        inner.cancel.is_none() && inner.sinks.is_empty() && inner.session.persisted()
    }

    pub fn subscribe(&self, resume: Option<Resume>, mut sink: impl FnMut(&Event) -> bool + Send + 'static) -> u64 {
        let mut inner = self.inner();
        let id = inner.next_sink;
        inner.next_sink += 1;
        if inner.snapshot(resume).iter().all(&mut sink) {
            inner.sinks.push((id, Box::new(sink)));
        }
        id
    }

    pub fn unsubscribe(&self, id: u64) {
        self.inner().sinks.retain(|(sink, _)| *sink != id);
    }

    pub fn send(self: &Arc<Self>, text: String, skill: Option<&str>, from: Option<Sender>) -> Result<()> {
        if text.trim().is_empty() {
            return Err("Empty message".into());
        }
        let (cwd, skill) = {
            let inner = self.inner();
            let skill = match skill {
                Some(name) => Some(
                    inner
                        .skills
                        .iter()
                        .rev()
                        .find(|skill| skill.name == name)
                        .cloned()
                        .ok_or_else(|| format!("Unknown skill: {name}"))?,
                ),
                None => None,
            };
            (inner.session.header().cwd.clone(), skill)
        };
        let message = input::Message { from, ..input::prepare(text, &cwd, skill.as_ref())? };
        let mut inner = self.inner();
        inner.queued.push(message);
        if inner.cancel.is_none() {
            inner.drain_queue();
            self.start(&mut inner);
        }
        inner.emit_state();
        Ok(())
    }

    fn start(self: &Arc<Self>, inner: &mut Inner) -> bool {
        if inner.cancel.is_some() || inner.session.cursor().is_none() || !inner.save() {
            return false;
        }
        inner.live.clear();
        let request = api::Request {
            instructions: inner.session.header().instructions.clone(),
            model: inner.session.model().name.clone(),
            effort: inner.session.model().effort.clone(),
            input: inner.session.input(),
            cwd: inner.session.header().cwd.clone(),
            bridge: inner.bridge.clone(),
            session: inner.session.header().id.clone(),
            previous_usage: inner.session.usage(),
        };
        let cancel = Arc::new(AtomicBool::new(false));
        inner.cancel = Some(cancel.clone());
        inner.status = Status::Running;
        let agent = self.clone();
        let started = now();
        thread::spawn(move || {
            let result = api::step(request, cancel, &|event| agent.live(event));
            agent.finish(result, started);
        });
        true
    }

    fn live(&self, event: LiveEvent) {
        let mut inner = self.inner();
        inner.live.apply(&event);
        inner.emit(Event::Live(event));
    }

    fn finish(self: &Arc<Self>, result: Result<api::Step>, started: u64) {
        let mut inner = self.inner();
        let cancelled = inner.cancel.take().is_some_and(|cancel| cancel.load(Ordering::Relaxed));
        let span = Span { started, finished: now() };
        match result {
            Ok(mut step) => {
                if step.usage.cache_miss(inner.session.usage()) {
                    step.entries.push(Entry::Notice { text: api::CACHE_MISS.into() });
                }
                let again = !cancelled && (step.again || !inner.queued.is_empty());
                let (end, status) = match (cancelled, again) {
                    (true, _) => (Some(End::Stopped), Status::Idle),
                    // Only reached as a status when the next step cannot start.
                    (false, true) => (None, Status::Failed),
                    (false, false) => (Some(End::Finished), Status::Finished),
                };
                inner.session.push(step.entries, Some(step.usage), span, end);
                inner.commit();
                if !cancelled {
                    inner.drain_queue();
                }
                if !(again && self.start(&mut inner)) {
                    inner.save();
                    inner.status = status;
                }
            }
            Err(e) => {
                let (end, status) = match cancelled {
                    true => (End::Stopped, Status::Idle),
                    false => (End::Failed { error: e.to_string() }, Status::Failed),
                };
                inner.status = status;
                inner.session.push(vec![], None, span, Some(end));
                inner.commit();
                inner.save();
                if !inner.queued.is_empty() {
                    inner.feedback(Kind::Notice, "Steering still queued. Send a message to retry, /new to discard.");
                }
            }
        }
        inner.emit_state();
    }

    pub fn stop(&self) {
        if let Some(cancel) = &self.inner().cancel {
            cancel.store(true, Ordering::Relaxed);
        }
    }

    pub fn mark_read(&self) -> Result<()> {
        let mut inner = self.inner();
        if inner.session.mark_read()? {
            inner.session.save()?;
            (inner.changed)(&inner.session.header().id);
        }
        Ok(())
    }

    pub fn append(&self, take: impl FnOnce() -> Result<tools::Run>) -> Result<()> {
        let mut inner = self.inner();
        if inner.cancel.is_some() {
            return Err("The agent is running; add the command once it stops".into());
        }
        if inner.session.cursor().is_none() {
            return Err("The conversation is empty; send a message before adding a command to it".into());
        }
        let run = take()?;
        let cwd = Some(run.cwd).filter(|cwd| cwd != &inner.session.header().cwd);
        let mut entries = vec![Entry::Shell { command: run.command, cwd, status: run.status, output: run.output }];
        entries.extend(run.images.into_iter().map(|item| Entry::Item { item }));
        inner.session.push(entries, None, run.span, None);
        inner.commit();
        inner.save();
        inner.status = inner.session.status();
        inner.emit_state();
        Ok(())
    }

    // Only while the message at `index` is still `text`: the queue may have been sent, and refilled, since the client saw it.
    pub fn recall(&self, index: u64, text: &str) -> bool {
        let Ok(index) = usize::try_from(index) else { return false };
        let mut inner = self.inner();
        if inner.queued.get(index).is_none_or(|message| message.text != text) {
            return false;
        }
        inner.queued.remove(index);
        inner.emit_state();
        true
    }

    pub fn set_model(&self, model: Model) -> Result<()> {
        let mut inner = self.inner();
        if inner.cancel.is_some() {
            return Err("Stop the agent before changing models".into());
        }
        check(&model)?;
        if &model != inner.session.model() {
            inner.session.record(Record::Model(model))?;
        }
        inner.save();
        inner.emit_state();
        Ok(())
    }

    pub fn tree(&self) -> Vec<TreeEntry> {
        let inner = self.inner();
        let cursor = inner.session.cursor();
        inner
            .session
            .tree()
            .into_iter()
            .map(|(node, label)| TreeEntry {
                node: node as u64,
                parent: inner.session.node(node).parent.map(|parent| parent as u64),
                kind: label.map(|block| block.kind),
                text: label
                    .and_then(|block| block.text.lines().find(|line| !line.trim().is_empty()))
                    .unwrap_or("")
                    .trim()
                    .chars()
                    .take(200)
                    .collect(),
                current: Some(node) == cursor,
            })
            .collect()
    }

    pub fn checkout(&self, node: Option<u64>) -> Result<()> {
        let mut inner = self.inner();
        let node = node.map(|node| inner.session.node_index(node).ok_or("Invalid session node")).transpose()?;
        inner.move_to(node)
    }

    pub fn edit(&self, node: u64) -> Result<String> {
        let mut inner = self.inner();
        let user = inner.session.node_index(node).map(|node| inner.session.node(node)).and_then(|node| {
            Some((node.parent, node.blocks.iter().find(|block| block.kind == Kind::User)?.text.clone()))
        });
        let (parent, text) = user.ok_or("Not a user message")?;
        inner.move_to(parent)?;
        Ok(text)
    }

    pub fn switch_branch(&self, node: u64) -> Result<()> {
        let mut inner = self.inner();
        let node = inner.session.node_index(node).ok_or("Invalid session node")?;
        let latest = inner.session.latest(node);
        inner.move_to(Some(latest))
    }
}
