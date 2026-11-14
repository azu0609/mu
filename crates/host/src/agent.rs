use crate::{
    Result, api, config, input,
    session::{self, End, Record, Session},
    skills::{self, Skill},
};
use mu_wire::{
    self as wire, Block, Event, Kind, Mention, Model, Resume, Snapshot, Span, State, Stats, Status, TreeEntry,
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

// Told a session's id whenever what a session list shows of it may have
// changed. Runs under the agent's lock, so it must not block.
pub type Changed = Arc<dyn Fn(&str) + Send + Sync>;

struct Inner {
    session: Session,
    skills: Vec<Skill>,
    live: Vec<Block>,
    live_notice: Option<Block>,
    queued: Vec<input::Message>,
    // Present while a step is running.
    cancel: Option<Arc<AtomicBool>>,
    status: Status,
    sinks: Vec<(u64, Sink)>,
    next_sink: u64,
    changed: Changed,
}

// One session and the loop driving it. Every client of the session shares it.
pub struct Agent {
    inner: Mutex<Inner>,
    // Separate from `inner`: indexing a large tree must not stall the stream.
    files: Mutex<Option<Arc<Vec<String>>>>,
}

// Whatever the client leaves out comes from the host's environment. An empty
// effort is a choice, not an omission: the model takes none.
pub fn default_model(name: Option<String>, effort: Option<String>, context: Option<u64>) -> Result<Model> {
    Ok(Model {
        name: match name {
            Some(name) => name,
            None => config::var("MU_MODEL")?.unwrap_or_else(|| "gpt-6-luna".into()),
        },
        effort: match effort {
            Some(effort) => Some(effort).filter(|effort| !effort.is_empty()),
            None => Some(config::var("MU_EFFORT")?.unwrap_or_else(|| "xhigh".into())),
        },
        context: match context.filter(|&n| n > 0) {
            Some(context) => context,
            None => config::var("MU_CONTEXT")?.and_then(|s| s.parse().ok()).filter(|&n| n > 0).unwrap_or(272_000),
        },
    })
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

    // A reset, then the transcript along the current path, then the step in
    // progress. A client resuming on the same path only gets the blocks it lacks.
    fn snapshot(&self, resume: Option<Resume>) -> Vec<Event> {
        let epoch = self.session.epoch();
        let blocks: Vec<_> =
            self.session.path().iter().flat_map(|&i| self.session.node(i).blocks.iter().cloned()).collect();
        let keep =
            resume.filter(|resume| resume.epoch == epoch && resume.blocks <= blocks.len()).map_or(0, |r| r.blocks);
        let mut events = vec![Event::Snapshot(Box::new(Snapshot {
            id: self.session.header().id.clone(),
            cwd: self.session.header().cwd.clone(),
            state: self.state(),
            skills: self
                .skills
                .iter()
                .map(|skill| wire::Skill { name: skill.name.clone(), description: skill.description.clone() })
                .collect(),
            epoch,
            keep,
        }))];
        if blocks.len() > keep {
            events.push(Event::Commit { blocks: blocks[keep..].to_vec() });
        }
        events.extend(self.live.iter().cloned().map(Event::LivePush));
        events.extend(self.live_notice.clone().map(Event::LiveNotice));
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

    // Publish the node the session just pushed.
    fn commit(&mut self) {
        let blocks = self.session.cursor().map(|i| self.session.node(i).blocks.clone()).unwrap_or_default();
        self.live.clear();
        self.live_notice = None;
        self.emit(Event::Commit { blocks });
    }

    fn drain_queue(&mut self) {
        for message in mem::take(&mut self.queued) {
            self.session.user(message.text, message.attachments);
            self.commit();
        }
    }
}

impl Agent {
    fn wrap(session: Session, skills: Vec<Skill>, changed: Changed) -> Arc<Self> {
        Arc::new(Self {
            inner: Mutex::new(Inner {
                skills,
                live: vec![],
                live_notice: None,
                queued: vec![],
                cancel: None,
                status: session.status(),
                session,
                sinks: vec![],
                next_sink: 0,
                changed,
            }),
            files: Mutex::new(None),
        })
    }

    pub fn new(cwd: PathBuf, model: Model, changed: Changed) -> Arc<Self> {
        let skills = skills::skills(&cwd);
        let session = Session::new(cwd, model, &skills);
        Self::wrap(session, skills, changed)
    }

    pub fn load(id: &str, changed: Changed) -> Result<Arc<Self>> {
        let session = Session::load(id)?;
        let skills = skills::skills(&session.header().cwd);
        Ok(Self::wrap(session, skills, changed))
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

    // Nobody is watching, nothing is running, and everything is on disk.
    pub fn disposable(&self) -> bool {
        let inner = self.inner();
        inner.cancel.is_none() && inner.sinks.is_empty() && inner.session.persisted()
    }

    // The sink receives a snapshot first, with no event lost or repeated after it.
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

    // Starts a step, or queues the message as steering while one is running.
    pub fn send(self: &Arc<Self>, text: String, skill: Option<&str>) -> Result<()> {
        if text.trim().is_empty() {
            return Err("Empty message".into());
        }
        let (cwd, skill) = {
            let inner = self.inner();
            let skill = match skill {
                // The last discovered skill wins among equal names.
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
        let message = input::prepare(text, &cwd, skill.as_ref())?;
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
        inner.live_notice = None;
        let request = api::Request {
            instructions: inner.session.header().instructions.clone(),
            model: inner.session.model().name.clone(),
            effort: inner.session.model().effort.clone(),
            input: inner.session.input(),
            cwd: inner.session.header().cwd.clone(),
            previous_usage: inner.session.usage(),
        };
        let cancel = Arc::new(AtomicBool::new(false));
        inner.cancel = Some(cancel.clone());
        inner.status = Status::Running;
        let agent = self.clone();
        let started = session::now();
        thread::spawn(move || {
            let result = api::step(request, cancel, &|event| agent.live(event));
            // All streaming events are applied before the finalized items are committed.
            agent.finish(result, started);
        });
        true
    }

    fn live(&self, event: api::Event) {
        let mut inner = self.inner();
        match event {
            api::Event::Push(block) => {
                inner.live.push(block.clone());
                inner.emit(Event::LivePush(block));
            }
            api::Event::Delta(index, text) => {
                if let Some(block) = inner.live.get_mut(index) {
                    block.text.push_str(&text);
                }
                inner.emit(Event::LiveDelta { index, text });
            }
            api::Event::Set(index, block) => {
                if let Some(slot) = inner.live.get_mut(index) {
                    *slot = block.clone();
                }
                inner.emit(Event::LiveSet { index, block });
            }
            api::Event::ToolOutput(index, output) => {
                if let Some(tool) = inner.live.get_mut(index).and_then(|block| block.tool.as_mut()) {
                    tool.output = output.clone();
                }
                inner.emit(Event::LiveOutput { index, output });
            }
            api::Event::CacheMiss => {
                let block = Block::new(Kind::Warning, CACHE_MISS);
                inner.live_notice = Some(block.clone());
                inner.emit(Event::LiveNotice(block));
            }
        }
    }

    fn finish(self: &Arc<Self>, result: Result<api::Step>, started: u64) {
        let mut inner = self.inner();
        let cancelled = inner.cancel.take().is_some_and(|cancel| cancel.load(Ordering::Relaxed));
        match result {
            Ok(mut step) => {
                // Keep the marker with this response, ahead of its output, not in transient feedback.
                if step.usage.cache_miss(inner.session.usage()) {
                    step.entries.insert(0, Record::Notice { text: CACHE_MISS.into() });
                }
                let again = !cancelled && (step.again || !inner.queued.is_empty());
                let end = if cancelled {
                    Some(End::Stopped)
                } else if again {
                    None
                } else {
                    Some(End::Finished)
                };
                inner.session.push(step.entries, Some(step.usage), Span { started, finished: session::now() }, end);
                inner.commit();
                if !cancelled {
                    inner.drain_queue();
                }
                if !(again && self.start(&mut inner)) {
                    inner.save();
                    inner.status = if cancelled {
                        Status::Idle
                    } else if again {
                        // The next step could not start (for example, the session failed to save).
                        Status::Failed
                    } else {
                        Status::Finished
                    };
                }
            }
            Err(e) => {
                // What streamed of the step is gone; the reason stays in the transcript.
                let end = if cancelled { End::Stopped } else { End::Failed { error: e.to_string() } };
                inner.status = if cancelled { Status::Idle } else { Status::Failed };
                inner.session.push(vec![], None, Span { started, finished: session::now() }, Some(end));
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

    // Marks every step so far as seen. Saved right away, so session lists, which
    // read the log, agree.
    pub fn mark_read(&self) -> Result<()> {
        let mut inner = self.inner();
        if inner.session.mark_read()? {
            inner.session.save()?;
            (inner.changed)(&inner.session.header().id);
        }
        Ok(())
    }

    // Takes the newest queued message back for editing.
    pub fn recall(&self) -> Option<String> {
        let mut inner = self.inner();
        let message = inner.queued.pop()?;
        inner.emit_state();
        Some(message.text)
    }

    pub fn set_model(&self, model: Model) -> Result<()> {
        let mut inner = self.inner();
        if inner.cancel.is_some() {
            return Err("Stop the agent before changing models".into());
        }
        if model.name.is_empty() || model.context == 0 {
            return Err("A model needs a name and a positive context size".into());
        }
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
            .map(|(node, label)| TreeEntry { node, label, current: node == cursor })
            .collect()
    }

    // Checking out a user message moves to its parent and returns its text as a draft.
    pub fn checkout(&self, node: Option<usize>) -> Result<Option<String>> {
        let mut inner = self.inner();
        if inner.cancel.is_some() {
            return Err("Stop the agent before moving in the conversation tree".into());
        }
        if node.is_some_and(|i| !inner.session.is_node(i)) {
            return Err("Invalid session node".into());
        }
        let mut cursor = node;
        let mut draft = None;
        if let Some(node) = node.map(|i| inner.session.node(i))
            && let Some(user) = node.blocks.iter().find(|block| block.kind == Kind::User)
        {
            cursor = node.parent;
            draft = Some(user.text.clone());
        }
        if inner.session.cursor() != cursor {
            inner.session.record(Record::Cursor { node: cursor })?;
            inner.save();
        }
        inner.live.clear();
        inner.live_notice = None;
        inner.queued.clear();
        inner.status = Status::Idle;
        for event in inner.snapshot(None) {
            inner.emit(event);
        }
        Ok(draft)
    }

    // Completes the @file mention at the cursor (a character offset), if there is one.
    // `refresh` rebuilds the file index; a failed build is reported once and then stays empty.
    pub fn complete(&self, text: &str, cursor: usize, refresh: bool) -> Result<Option<Mention>> {
        let Some((range, query)) = input::mention_at(text, cursor) else {
            return Ok(None);
        };
        let cwd = self.inner().session.header().cwd.clone();
        let files = if input::is_path(&query) {
            Arc::default()
        } else {
            let mut files = self.files.lock().unwrap();
            if refresh || files.is_none() {
                let built = input::index(&cwd);
                *files = Some(Arc::new(built.as_ref().cloned().unwrap_or_default()));
                built?;
            }
            files.clone().unwrap()
        };
        Ok(Some(input::complete(range, query, &cwd, &files)))
    }
}

const CACHE_MISS: &str = "Cache miss · previously cached prefix was not read";
