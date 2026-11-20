use crate::{Result, config, input::Attachment, instructions, items::block_from_item, skills::Skill};
use mu_wire::{Block, Kind, Model, Sender, Span, Status, Tool, ToolOutput, ToolStatus};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    fs,
    io::{self, BufRead, BufReader, BufWriter, Seek, SeekFrom, Write},
    mem,
    os::unix::fs::OpenOptionsExt,
    path::PathBuf,
};

#[derive(Clone, Copy, Default, Serialize, Deserialize)]
pub struct Usage {
    pub input: u64,
    pub output: u64,
    pub cached: Option<u64>,
}

impl Usage {
    pub fn from_json(v: &Value) -> Self {
        Self {
            input: v["input_tokens"].as_u64().unwrap_or(0),
            output: v["output_tokens"].as_u64().unwrap_or(0),
            cached: v["input_tokens_details"]["cached_tokens"].as_u64(),
        }
    }

    pub fn cache_miss(self, previous: Self) -> bool {
        previous.cached.unwrap_or(0) > 0 && self.cached == Some(0) && self.input >= previous.input
    }
}

// What a turn is made of: all that the model is given, and the notices shown with it.
#[derive(Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Entry {
    Item { item: Value },
    AgentMessage { from: Sender, text: String },
    Attachment { label: String, path: PathBuf, text: String },
    ImageAttachment { path: PathBuf, image: Value },
    Notice { text: String },
    Tool(Tool),
    Shell { command: String, cwd: Option<PathBuf>, status: ToolStatus, output: ToolOutput },
}

impl Entry {
    fn input(&self) -> Option<Value> {
        Some(match self {
            Self::Item { item } => item.clone(),
            Self::AgentMessage { from, text } => {
                let (session, title) = (attribute(&from.session), attribute(&from.title));
                let text = format!("<agent-message session=\"{session}\" title=\"{title}\">\n{text}\n</agent-message>");
                json!({"role":"user", "content":text})
            }
            Self::Attachment { label, path, text } => {
                json!({"role":"user", "content":format!("{label}: {}\n{text}", path.display())})
            }
            Self::ImageAttachment { path, image } => {
                json!({"role":"user", "content":[{"type":"input_text", "text":format!("Image: {}", path.display())}, image]})
            }
            Self::Notice { .. } => return None,
            Self::Tool(tool) => tool_item(tool),
            Self::Shell { command, cwd, status, output } => {
                let prompt = cwd.as_ref().map(|cwd| cwd.display().to_string()).unwrap_or_default();
                let text = format!("{prompt}$ {command}\n{}", tool_text(*status, output).trim_start_matches('\n'));
                json!({"role":"user", "content":text.trim_end()})
            }
        })
    }
}

// This is both the on-disk log and the in-memory history. A node commits the
// entries since the record before them; its blocks are a disposable rendering
// cache, not a second history.
#[derive(Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Record {
    Node(Node),
    Cursor {
        node: Option<usize>,
    },
    Model(Model),
    Read,
    #[serde(untagged)]
    Entry(Entry),
}

impl Record {
    fn entry(&self) -> Option<&Entry> {
        match self {
            Self::Entry(entry) => Some(entry),
            _ => None,
        }
    }
}

#[derive(Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum End {
    Finished,
    Stopped,
    Failed { error: String },
}

const STOPPED: &str = "Stopped. Tool side effects are not undone.";

#[derive(Default, Serialize, Deserialize)]
pub struct Node {
    pub parent: Option<usize>,
    pub usage: Option<Usage>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub span: Option<Span>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub end: Option<End>,
    #[serde(skip)]
    start: usize,
    #[serde(skip)]
    pub blocks: Vec<Block>,
}

impl Node {
    pub fn status(&self) -> Status {
        match self.end {
            Some(End::Finished) => Status::Finished,
            Some(End::Failed { .. }) => Status::Failed,
            Some(End::Stopped) | None => Status::Idle,
        }
    }
}

fn attribute(value: &str) -> String {
    value.replace('&', "&amp;").replace('"', "&quot;").replace('<', "&lt;")
}

fn tool_item(tool: &Tool) -> Value {
    json!({"type":"function_call_output", "call_id":tool.call_id, "output":tool_text(tool.status, &tool.output)})
}

fn tool_text(status: ToolStatus, output: &ToolOutput) -> String {
    let mut text = output.text.clone();
    if let Some(summary) = status.summary() {
        text.push_str(&format!("\n[{summary}]"));
    }
    if output.truncated
        && let Some(log) = &output.log
    {
        text.push_str(&format!(
            "\n[Output log (temporary): {} — read needed ranges with sed, tail, or grep]",
            log.display()
        ));
    }
    text
}

#[derive(Serialize, Deserialize)]
pub struct Header {
    pub version: u32,
    pub id: String,
    pub title: String,
    pub cwd: PathBuf,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent: Option<String>,
    pub instructions: String,
    #[serde(flatten)]
    model: Model,
}

pub struct Session {
    header: Header,
    records: Vec<Record>,
    file: Option<fs::File>,
    saved: usize,
    offset: u64,
}

// Lock the transcript's stable inode, not a separate sentinel. Closing the file
// releases ownership even after a crash; normal saves never rename it.
fn lock(file: &fs::File) -> Result<()> {
    match file.try_lock() {
        Ok(()) => Ok(()),
        Err(fs::TryLockError::WouldBlock) => Err("Session is open in another mu process".into()),
        Err(error) => Err(error.into()),
    }
}

fn write_line(writer: &mut impl Write, value: &impl Serialize) -> Result<()> {
    serde_json::to_writer(&mut *writer, value)?;
    writer.write_all(b"\n")?;
    Ok(())
}

impl Session {
    pub fn new(cwd: PathBuf, parent: Option<String>, model: Model, skills: &[Skill]) -> Self {
        let header = Header {
            version: 1,
            id: crate::unique_id(),
            title: String::new(),
            instructions: instructions::build(&cwd, skills),
            cwd,
            parent,
            model,
        };
        Self { header, records: vec![], file: None, saved: 0, offset: 0 }
    }

    pub fn header(&self) -> &Header {
        &self.header
    }

    pub fn model(&self) -> &Model {
        self.records
            .iter()
            .rev()
            .find_map(|r| match r {
                Record::Model(model) => Some(model),
                _ => None,
            })
            .unwrap_or(&self.header.model)
    }

    pub fn cursor(&self) -> Option<usize> {
        self.records
            .iter()
            .enumerate()
            .rev()
            .find_map(|(i, r)| match r {
                Record::Node(_) => Some(Some(i)),
                Record::Cursor { node } => Some(*node),
                _ => None,
            })
            .flatten()
    }

    pub fn node_index(&self, id: u64) -> Option<usize> {
        let i = usize::try_from(id).ok()?;
        matches!(self.records.get(i), Some(Record::Node(_))).then_some(i)
    }

    pub fn persisted(&self) -> bool {
        self.file.is_some() && self.saved == self.records.len()
    }

    pub fn node(&self, i: usize) -> &Node {
        match &self.records[i] {
            Record::Node(node) => node,
            _ => unreachable!("validated node index"),
        }
    }

    fn nodes(&self) -> impl Iterator<Item = (usize, &Node)> {
        self.records.iter().enumerate().filter_map(|(i, r)| match r {
            Record::Node(node) => Some((i, node)),
            _ => None,
        })
    }

    fn ancestors(&self) -> impl Iterator<Item = usize> + '_ {
        std::iter::successors(self.cursor(), |&i| self.node(i).parent)
    }

    pub fn path(&self) -> Vec<usize> {
        let mut path: Vec<_> = self.ancestors().collect();
        path.reverse();
        path
    }

    pub fn input(&self) -> Vec<Value> {
        self.path()
            .iter()
            .flat_map(|&i| self.records[self.node(i).start..i].iter().filter_map(Record::entry))
            .filter_map(Entry::input)
            .collect()
    }

    pub fn usage(&self) -> Usage {
        self.ancestors().find_map(|i| self.node(i).usage).unwrap_or_default()
    }

    pub fn total_usage(&self) -> Usage {
        let mut total = Usage { cached: Some(0), ..Usage::default() };
        for usage in self.ancestors().filter_map(|i| self.node(i).usage) {
            total.input = total.input.saturating_add(usage.input);
            total.output = total.output.saturating_add(usage.output);
            total.cached = total.cached.zip(usage.cached).map(|(a, b)| a.saturating_add(b));
        }
        total
    }

    pub fn uncached_input(&self) -> u64 {
        self.ancestors()
            .filter_map(|i| self.node(i).usage)
            .fold(0u64, |total, usage| total.saturating_add(usage.input.saturating_sub(usage.cached.unwrap_or(0))))
    }

    pub fn push(&mut self, entries: Vec<Entry>, usage: Option<Usage>, span: Span, end: Option<End>) {
        let parent = self.cursor();
        self.records.extend(entries.into_iter().map(Record::Entry));
        let node = Node { parent, usage, span: Some(span), end, ..Node::default() };
        self.record(Record::Node(node)).expect("valid cursor");
    }

    pub fn status(&self) -> Status {
        self.records
            .iter()
            .rev()
            .find_map(|r| match r {
                Record::Node(node) => Some(node.status()),
                Record::Cursor { .. } => Some(Status::Idle),
                _ => None,
            })
            .unwrap_or(Status::Idle)
    }

    pub fn user(&mut self, text: String, from: Option<Sender>, attachments: Vec<Attachment>) {
        if self.file.is_none() && self.nodes().next().is_none() {
            self.header.title = text.lines().next().unwrap_or("").chars().take(60).collect();
        }
        let mut entries = vec![match from {
            Some(from) => Entry::AgentMessage { from, text },
            None => Entry::Item { item: json!({"role":"user", "content":text}) },
        }];
        entries.extend(attachments.into_iter().map(Into::into));
        let now = crate::now();
        self.push(entries, None, Span { started: now, finished: now }, None);
    }

    // Changes whenever the current path does other than by growing, which only
    // a recorded cursor does. Records stay where they are, so it survives restarts.
    pub fn epoch(&self) -> u64 {
        self.records.iter().rposition(|r| matches!(r, Record::Cursor { .. })).map_or(0, |i| i as u64 + 1)
    }

    pub fn unread_from(&self) -> u64 {
        self.records.iter().rposition(|r| matches!(r, Record::Read)).map_or(0, |i| i as u64)
    }

    pub fn mark_read(&mut self) -> Result<bool> {
        let unread = self.records.iter().rev().find_map(|r| match r {
            Record::Node(_) => Some(true),
            Record::Read => Some(false),
            _ => None,
        });
        if unread != Some(true) {
            return Ok(false);
        }
        self.record(Record::Read)?;
        Ok(true)
    }

    pub fn record(&mut self, mut record: Record) -> Result<()> {
        let uncommitted = self.records.last().is_some_and(|last| last.entry().is_some());
        if uncommitted && !matches!(record, Record::Node(_) | Record::Entry(_)) {
            return Err("Uncommitted session entries".into());
        }
        match &mut record {
            Record::Node(node) => {
                if node.parent.is_some_and(|i| !matches!(self.records.get(i), Some(Record::Node(_)))) {
                    return Err("Invalid session parent".into());
                }
                node.start = self.records.iter().rposition(|r| r.entry().is_none()).map_or(0, |i| i + 1);
                let span = node.span;
                for entry in self.records[node.start..].iter().filter_map(Record::entry) {
                    match entry {
                        Entry::Item { item } => node.blocks.extend(block_from_item(item, false)),
                        Entry::AgentMessage { from, text } => {
                            node.blocks.push(Block { from: Some(from.clone()), ..Block::new(Kind::AgentMessage, text) })
                        }
                        Entry::Attachment { label, path, .. } => {
                            node.blocks.push(Block::new(Kind::Notice, format!("{label}: {}", path.display())))
                        }
                        Entry::ImageAttachment { path, .. } => {
                            node.blocks.push(Block::new(Kind::Notice, format!("Image: {}", path.display())))
                        }
                        Entry::Notice { text } => node.blocks.push(Block::new(Kind::Warning, text)),
                        Entry::Tool(tool) => {
                            if let Some(block) = node
                                .blocks
                                .iter_mut()
                                .rev()
                                .find(|b| b.tool.as_ref().is_some_and(|t| t.call_id == tool.call_id))
                            {
                                block.tool = Some(tool.clone());
                            }
                        }
                        Entry::Shell { command, status, output, .. } => {
                            let mut block = Block::new(Kind::Shell, command);
                            block.tool = Some(Tool { call_id: String::new(), status: *status, output: output.clone() });
                            node.blocks.push(block);
                        }
                    }
                }
                match &node.end {
                    Some(End::Stopped) => node.blocks.push(Block::new(Kind::Warning, STOPPED)),
                    Some(End::Failed { error }) => node.blocks.push(Block::new(Kind::Warning, error)),
                    Some(End::Finished) | None => (),
                }
                for block in &mut node.blocks {
                    block.span = span;
                }
            }
            Record::Cursor { node } if node.is_some_and(|i| !matches!(self.records.get(i), Some(Record::Node(_)))) => {
                return Err("Invalid session cursor".into());
            }
            _ => (),
        }
        self.records.push(record);
        Ok(())
    }

    pub fn save(&mut self) -> Result<()> {
        if self.saved == self.records.len() || self.nodes().next().is_none() {
            return Ok(());
        }
        let path = config::state_dir().join(format!("{}.jsonl", self.header.id));
        if self.file.is_none() {
            fs::create_dir_all(path.parent().unwrap())?;
            let file = fs::OpenOptions::new().read(true).write(true).create_new(true).mode(0o600).open(&path)?;
            lock(&file)?;
            self.file = Some(file);
        }
        let mut file = self.file.as_ref().unwrap();
        // Retry from the last synced boundary, never duplicate a failed append.
        file.set_len(self.offset)?;
        file.seek(SeekFrom::Start(self.offset))?;
        let mut writer = BufWriter::new(file);
        if self.offset == 0 {
            write_line(&mut writer, &self.header)?;
        }
        for record in &self.records[self.saved..] {
            write_line(&mut writer, record)?;
        }
        writer.flush()?;
        drop(writer);
        let offset = file.stream_position()?;
        file.sync_all()?;
        if self.offset == 0 {
            fs::File::open(path.parent().unwrap())?.sync_all()?;
        }
        self.offset = offset;
        self.saved = self.records.len();
        Ok(())
    }

    pub fn load(id: &str) -> Result<Self> {
        let path = &path(id)?;
        let file = fs::OpenOptions::new().read(true).write(true).open(path)?;
        lock(&file)?;
        let mut reader = BufReader::new(&file);
        let mut line = vec![];
        let mut offset = reader.read_until(b'\n', &mut line)? as u64;
        if line.last() != Some(&b'\n') {
            return Err("Incomplete session header".into());
        }
        let header: Header = serde_json::from_slice(&line)?;
        if header.version != 1 || header.id != id {
            return Err("Invalid session header".into());
        }
        let mut session = Self { header, records: vec![], file: None, saved: 0, offset };
        loop {
            line.clear();
            offset += reader.read_until(b'\n', &mut line)? as u64;
            if line.last() != Some(&b'\n') {
                break;
            }
            let record: Record =
                serde_json::from_slice(&line).map_err(|e| format!("Session near byte {offset}: {e}"))?;
            let committed = record.entry().is_none();
            session.record(record)?;
            if committed {
                session.saved = session.records.len();
                session.offset = offset;
            }
        }
        drop(reader);
        // Only an unfinished tail is discarded; complete malformed records fail.
        session.records.truncate(session.saved);
        if file.metadata()?.len() != session.offset {
            file.set_len(session.offset)?;
            file.sync_all()?;
        }
        session.file = Some(file);
        Ok(session)
    }

    fn children(&self) -> Vec<Vec<usize>> {
        let mut children = vec![vec![]; self.records.len() + 1];
        for (i, node) in self.nodes() {
            children[node.parent.map_or(0, |p| p + 1)].push(i);
        }
        children
    }

    pub fn branches(&self) -> Vec<(usize, Vec<usize>)> {
        let mut children = self.children();
        self.path()
            .into_iter()
            .map(|i| (i, mem::take(&mut children[self.node(i).parent.map_or(0, |p| p + 1)])))
            .collect()
    }

    pub fn latest(&self, node: usize) -> usize {
        let mut within = vec![false; self.records.len()];
        within[node] = true;
        for (i, n) in self.nodes().filter(|&(i, _)| i > node) {
            within[i] = n.parent.is_some_and(|p| within[p]);
        }
        self.records
            .iter()
            .enumerate()
            .rev()
            .filter_map(|(i, r)| match r {
                Record::Node(_) => Some(i),
                Record::Cursor { node } => *node,
                _ => None,
            })
            .find(|&i| within[i])
            .unwrap_or(node)
    }

    // Iterative DFS: very deep tool loops don't consume the stack.
    pub fn tree(&self) -> Vec<(usize, Option<&Block>)> {
        let children = self.children();
        let mut result = vec![];
        let mut stack: Vec<_> = children[0].iter().rev().copied().collect();
        while let Some(i) = stack.pop() {
            let blocks = &self.node(i).blocks;
            let label = blocks
                .iter()
                .find(|b| matches!(b.kind, Kind::User | Kind::AgentMessage | Kind::Agent | Kind::Call | Kind::Shell))
                .or_else(|| blocks.iter().find(|b| b.kind == Kind::Warning));
            result.push((i, label));
            stack.extend(children[i + 1].iter().rev());
        }
        result
    }
}

// Ids arrive from clients; never let one name a path outside the state directory.
pub fn path(id: &str) -> Result<PathBuf> {
    if id.is_empty() || !id.chars().all(|c| c.is_ascii_digit() || c == '-') {
        return Err("Invalid session id".into());
    }
    Ok(config::state_dir().join(format!("{id}.jsonl")))
}

pub fn delete(id: &str) -> Result<bool> {
    let path = path(id)?;
    let file = match fs::File::open(&path) {
        Ok(file) => file,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(true),
        Err(e) => return Err(e.into()),
    };
    // Held until the file is gone: a mu loading it meanwhile fails to lock it.
    match file.try_lock() {
        Ok(()) => (),
        Err(fs::TryLockError::WouldBlock) => return Ok(false),
        Err(fs::TryLockError::Error(e)) => return Err(e.into()),
    }
    fs::remove_file(&path)?;
    Ok(true)
}
