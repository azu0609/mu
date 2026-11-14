use crate::{Result, home, instructions, skills::Skill};
use mu_wire::{Block, Kind, Model, Span, Status, Tool, ToolOutput, ToolStatus};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    env, fs,
    io::{self, BufRead, BufReader, BufWriter, Read, Seek, SeekFrom, Write},
    os::unix::fs::OpenOptionsExt,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

// Milliseconds since the Unix epoch.
pub fn now() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_millis() as u64
}

pub fn block_from_item(item: &Value, partial: bool) -> Option<Block> {
    if item["role"] == "user"
        && let Some(text) = item["content"].as_str()
    {
        return Some(Block::new(Kind::User, text));
    }
    let texts = |field: &str, typ: &str| {
        item[field]
            .as_array()
            .map(|a| {
                a.iter().filter(|v| v["type"] == typ).filter_map(|v| v["text"].as_str()).collect::<Vec<_>>().join("\n")
            })
            .unwrap_or_default()
    };
    match item["type"].as_str()? {
        "message" => {
            let mut text = texts("content", "output_text");
            if let Some(content) = item["content"].as_array() {
                for part in content {
                    if let Some(refusal) = part["refusal"].as_str() {
                        text.push_str(refusal);
                    }
                }
            }
            Some(Block::new(Kind::Agent, text))
        }
        "reasoning" => {
            let mut text = texts("summary", "summary_text");
            if text.is_empty() {
                text = texts("content", "reasoning_text");
            }
            Some(Block::new(Kind::Thought, text))
        }
        "function_call" => {
            let name = item["name"].as_str().unwrap_or("tool");
            let raw = item["arguments"].as_str().unwrap_or("");
            let args: Value = serde_json::from_str(raw).unwrap_or(Value::Null);
            let text = if name == "bash" {
                args["command"]
                    .as_str()
                    .map(str::to_owned)
                    .or_else(|| partial.then(|| command_prefix(raw)).flatten())
                    .unwrap_or_else(|| if partial || raw.is_empty() { "bash …" } else { raw }.to_owned())
            } else {
                format!("{name} {raw}")
            };
            let mut block = Block::new(Kind::Call, text);
            block.tool = Some(Tool {
                call_id: item["call_id"].as_str().unwrap_or("").into(),
                status: ToolStatus::Pending,
                output: ToolOutput::default(),
            });
            Some(block)
        }
        _ => None,
    }
}

// Read only a top-level command, skipping complete fields before it. A partial
// JSON string is display-only: execution still uses the final, validated args.
fn command_prefix(raw: &str) -> Option<String> {
    let mut rest = raw.trim_start().strip_prefix('{')?;
    loop {
        let mut key = serde_json::Deserializer::from_str(rest).into_iter::<String>();
        let name = key.next()?.ok()?;
        rest = rest[key.byte_offset()..].trim_start().strip_prefix(':')?.trim_start();
        if name == "command" {
            return string_prefix(rest);
        }
        let mut value = serde_json::Deserializer::from_str(rest).into_iter::<serde::de::IgnoredAny>();
        value.next()?.ok()?;
        rest = rest[value.byte_offset()..].trim_start().strip_prefix(',')?;
    }
}

fn string_prefix(raw: &str) -> Option<String> {
    match serde_json::Deserializer::from_str(raw).into_iter::<String>().next()? {
        Ok(text) => return Some(text),
        Err(e) if e.is_eof() => (),
        Err(_) => return None,
    }
    // Supply the missing quote and let serde decode. On an unfinished escape,
    // withhold it from the preview (and its high surrogate, if paired).
    let mut prefix = raw.to_owned();
    loop {
        prefix.push('"');
        if let Ok(text) = serde_json::from_str(&prefix) {
            return Some(text);
        }
        prefix.truncate(prefix.rfind('\\')?);
    }
}

fn summary(status: ToolStatus) -> Option<String> {
    Some(match status {
        ToolStatus::Exited(0) | ToolStatus::Error => return None,
        ToolStatus::Pending => "pending".into(),
        ToolStatus::Running => "running".into(),
        ToolStatus::Exited(code) => format!("exit {code}"),
        ToolStatus::TimedOut(ms) => format!("timed out after {ms} ms; process group killed"),
        ToolStatus::Cancelled => "cancelled; process group killed".into(),
    })
}

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

// This is both the on-disk log and the in-memory history. Nodes only index
// ranges in it; blocks are a disposable rendering cache, not a second history.
#[derive(Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Record {
    Item { item: Value },
    Attachment { label: String, path: PathBuf, text: String },
    ImageAttachment { path: PathBuf, image: Value },
    Notice { text: String },
    Tool(Tool),
    Node(Node),
    Cursor { node: Option<usize> },
    Model(Model),
    // A client showed every step before this.
    Read,
}

impl Record {
    fn is_entry(&self) -> bool {
        matches!(
            self,
            Self::Item { .. }
                | Self::Attachment { .. }
                | Self::ImageAttachment { .. }
                | Self::Notice { .. }
                | Self::Tool(_)
        )
    }

    fn input(&self) -> Option<Value> {
        Some(match self {
            Self::Item { item } => item.clone(),
            Self::Attachment { label, path, text } => {
                json!({"role":"user", "content":format!("{label}: {}\n{text}", path.display())})
            }
            Self::ImageAttachment { path, image } => {
                json!({"role":"user", "content":[{"type":"input_text", "text":format!("Image: {}", path.display())}, image]})
            }
            Self::Notice { .. } => return None,
            Self::Tool(tool) => tool_item(tool),
            _ => return None,
        })
    }
}

// How a run ended, on its last node. Stops and failures also show in the
// transcript, so clients that were not watching learn of them.
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
    fn status(&self) -> Status {
        match self.end {
            Some(End::Finished) => Status::Finished,
            Some(End::Failed { .. }) => Status::Failed,
            Some(End::Stopped) | None => Status::Idle,
        }
    }
}

fn tool_item(tool: &Tool) -> Value {
    let mut text = tool.output.text.clone();
    if let Some(summary) = summary(tool.status) {
        text.push_str(&format!("\n[{summary}]"));
    }
    if tool.output.truncated
        && let Some(log) = &tool.output.log
    {
        text.push_str(&format!(
            "\n[Output log (temporary): {} — read needed ranges with sed, tail, or grep]",
            log.display()
        ));
    }
    json!({"type":"function_call_output", "call_id":tool.call_id, "output":text})
}

#[derive(Serialize, Deserialize)]
pub struct Header {
    version: u32,
    pub id: String,
    pub title: String,
    pub cwd: PathBuf,
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

pub fn unique_id() -> String {
    format!("{}-{}", SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_nanos(), std::process::id())
}

pub fn state_dir() -> PathBuf {
    env::var_os("XDG_STATE_HOME").map(PathBuf::from).unwrap_or_else(|| home().join(".local/state")).join("mu")
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
    pub fn new(cwd: PathBuf, model: Model, skills: &[Skill]) -> Self {
        let header = Header {
            version: 1,
            id: unique_id(),
            title: String::new(),
            instructions: instructions::build(&cwd, skills),
            cwd,
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

    pub fn is_node(&self, i: usize) -> bool {
        matches!(self.records.get(i), Some(Record::Node(_)))
    }

    // Nothing would be lost by dropping this session from memory.
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
        self.path().iter().flat_map(|&i| self.records[self.node(i).start..i].iter().filter_map(Record::input)).collect()
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

    pub fn push(&mut self, entries: Vec<Record>, usage: Option<Usage>, span: Span, end: Option<End>) {
        let parent = self.cursor();
        debug_assert!(entries.iter().all(Record::is_entry));
        self.records.extend(entries);
        let node = Node { parent, usage, span: Some(span), end, ..Node::default() };
        self.record(Record::Node(node)).expect("valid cursor");
    }

    // How the latest run ended, unless the path moved since.
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

    pub fn user(&mut self, text: String, attachments: Vec<Record>) {
        if self.file.is_none() && self.nodes().next().is_none() {
            self.header.title = text.lines().next().unwrap_or("").chars().take(60).collect();
        }
        let mut entries = vec![Record::Item { item: json!({"role":"user", "content":text}) }];
        entries.extend(attachments);
        let now = now();
        self.push(entries, None, Span { started: now, finished: now }, None);
    }

    // Changes whenever the current path does other than by growing, which only
    // a recorded cursor does. Records stay where they are, so it survives restarts.
    pub fn epoch(&self) -> u64 {
        self.records.iter().rposition(|r| matches!(r, Record::Cursor { .. })).map_or(0, |i| i as u64 + 1)
    }

    // Whether some step was not read yet, and now is.
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

    // Live changes and replay use the same path. Control records cannot split a
    // node, and every parent/cursor must point to an already committed node.
    pub fn record(&mut self, mut record: Record) -> Result<()> {
        if !record.is_entry() && !matches!(record, Record::Node(_)) && self.records.last().is_some_and(Record::is_entry)
        {
            return Err("Uncommitted session entries".into());
        }
        match &mut record {
            Record::Node(node) => {
                if node.parent.is_some_and(|i| !matches!(self.records.get(i), Some(Record::Node(_)))) {
                    return Err("Invalid session parent".into());
                }
                node.start = self.records.iter().rposition(|r| !r.is_entry()).map_or(0, |i| i + 1);
                let span = node.span;
                for entry in &self.records[node.start..] {
                    match entry {
                        Record::Item { item } => node.blocks.extend(block_from_item(item, false)),
                        Record::Attachment { label, path, .. } => {
                            node.blocks.push(Block::new(Kind::Notice, format!("{label}: {}", path.display())))
                        }
                        Record::ImageAttachment { path, .. } => {
                            node.blocks.push(Block::new(Kind::Notice, format!("Image: {}", path.display())))
                        }
                        Record::Notice { text } => node.blocks.push(Block::new(Kind::Warning, text)),
                        Record::Tool(tool) => {
                            if let Some(block) = node
                                .blocks
                                .iter_mut()
                                .rev()
                                .find(|b| b.tool.as_ref().is_some_and(|t| t.call_id == tool.call_id))
                            {
                                block.tool = Some(tool.clone());
                            }
                        }
                        _ => unreachable!(),
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
        let path = state_dir().join(format!("{}.jsonl", self.header.id));
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
            let committed = !record.is_entry();
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

    // Iterative DFS: very deep tool loops don't consume the stack. Linear
    // paths stay flat; fork guides continue through their descendants.
    pub fn tree(&self) -> Vec<(Option<usize>, String)> {
        struct Frame {
            node: Option<usize>,
            line_prefix: String,
            child_prefix: String,
            depth: usize,
        }

        let mut children = vec![vec![]; self.records.len() + 1];
        for (i, n) in self.nodes() {
            children[n.parent.map_or(0, |p| p + 1)].push(i);
        }
        let mut result = vec![];
        let mut stack = vec![Frame { node: None, line_prefix: String::new(), child_prefix: String::new(), depth: 0 }];
        while let Some(Frame { node, line_prefix, child_prefix, depth }) = stack.pop() {
            let label = node
                .map(|i| {
                    let blocks = &self.node(i).blocks;
                    blocks
                        .iter()
                        .find(|b| matches!(b.kind, Kind::User | Kind::Agent | Kind::Call))
                        .or_else(|| blocks.iter().find(|b| b.kind == Kind::Warning))
                        .map(|b| {
                            format!(
                                "{:?}: {}",
                                b.kind,
                                b.text.lines().next().unwrap_or("").chars().take(80).collect::<String>()
                            )
                        })
                        .unwrap_or_else(|| "step".into())
                })
                .unwrap_or_else(|| "root".into());
            result.push((node, format!("{line_prefix}{label}")));
            let siblings = &children[node.map_or(0, |i| i + 1)];
            for (position, &child) in siblings.iter().enumerate().rev() {
                if siblings.len() > 1 {
                    let last = position == siblings.len() - 1;
                    let line = format!("{child_prefix}{}", if last { "└─ " } else { "├─ " });
                    let next = if depth < 16 {
                        format!("{child_prefix}{}", if last { "     " } else { "│    " })
                    } else {
                        child_prefix.clone()
                    };
                    stack.push(Frame {
                        node: Some(child),
                        line_prefix: line,
                        child_prefix: next,
                        depth: (depth + 1).min(16),
                    });
                } else {
                    stack.push(Frame {
                        node: Some(child),
                        line_prefix: child_prefix.clone(),
                        child_prefix: child_prefix.clone(),
                        depth,
                    });
                }
            }
        }
        result
    }
}

// Ids arrive from clients; never let one name a path outside the state directory.
fn path(id: &str) -> Result<PathBuf> {
    if id.is_empty() || !id.chars().all(|c| c.is_ascii_digit() || c == '-') {
        return Err("Invalid session id".into());
    }
    Ok(state_dir().join(format!("{id}.jsonl")))
}

pub struct Summary {
    pub id: String,
    pub title: String,
    pub cwd: PathBuf,
    // How the latest run ended, if the path has not moved since.
    pub status: Status,
    // Why it failed.
    pub error: Option<String>,
    // Some step was never read; running or not.
    pub unread: bool,
    // When its latest step finished, or the log was last written for steps without times.
    pub updated: u64,
}

// A saved session, as its log's first and last lines tell it.
pub fn summary_of(id: &str) -> Option<Summary> {
    summarize(&path(id).ok()?)
}

fn summarize(path: &Path) -> Option<Summary> {
    let mut file = fs::File::open(path).ok()?;
    let mut line = String::new();
    BufReader::new(&file).read_line(&mut line).ok()?;
    if !line.ends_with('\n') {
        return None;
    }
    let header: Header = serde_json::from_str(&line).ok()?;
    if header.version != 1 {
        return None;
    }
    let tail = tail(&mut file).ok()?;
    let (node, read) = (tail.node, tail.read);
    let status = node.as_ref().filter(|_| !tail.moved).map_or(Status::Idle, Node::status);
    let error = match node.as_ref().and_then(|node| node.end.as_ref()) {
        Some(End::Failed { error }) if status == Status::Failed => Some(error.clone()),
        _ => None,
    };
    let modified = || {
        let modified = file.metadata().and_then(|m| m.modified()).ok()?;
        Some(modified.duration_since(UNIX_EPOCH).ok()?.as_millis() as u64)
    };
    Some(Summary {
        id: header.id,
        title: header.title,
        cwd: header.cwd,
        status,
        error,
        unread: node.is_some() && !read,
        updated: node.and_then(|node| node.span).map(|span| span.finished).or_else(modified).unwrap_or(0),
    })
}

struct Tail {
    node: Option<Node>,
    // A read marker follows the node.
    read: bool,
    // So does a cursor: the path moved.
    moved: bool,
}

// The log's latest node and what follows it. Control records are small and
// come last, so this rarely reads more than a few kilobytes.
fn tail(file: &mut fs::File) -> io::Result<Tail> {
    let len = file.metadata()?.len();
    let mut window = 16 * 1024;
    loop {
        let start = len.saturating_sub(window);
        file.seek(SeekFrom::Start(start))?;
        let mut bytes = vec![];
        file.take(len - start).read_to_end(&mut bytes)?;
        // Complete lines only: not an unfinished last one, nor one the window cuts into.
        let end = bytes.iter().rposition(|&b| b == b'\n').map_or(0, |i| i + 1);
        let mut lines: Vec<_> = bytes[..end].split(|&b| b == b'\n').collect();
        lines.pop();
        let complete = if start == 0 { &lines[..] } else { lines.get(1..).unwrap_or_default() };
        let (mut read, mut moved) = (false, false);
        for line in complete.iter().rev() {
            // serde writes the tag first.
            if line.starts_with(br#"{"type":"read""#) {
                read = true;
            } else if line.starts_with(br#"{"type":"cursor""#) {
                moved = true;
            } else if line.starts_with(br#"{"type":"node""#) {
                let Ok(Record::Node(node)) = serde_json::from_slice(line) else {
                    return Err(io::ErrorKind::InvalidData.into());
                };
                return Ok(Tail { node: Some(node), read, moved });
            }
        }
        if start == 0 {
            return Ok(Tail { node: None, read, moved });
        }
        window *= 4;
    }
}

// Latest first.
pub fn sessions() -> Result<Vec<Summary>> {
    let dir = &state_dir();
    if !dir.exists() {
        return Ok(vec![]);
    }
    let mut sessions: Vec<_> = fs::read_dir(dir)?
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|e| e == "jsonl"))
        .filter_map(|path| summarize(&path))
        .collect();
    sessions.sort_by_key(|summary| std::cmp::Reverse(summary.updated));
    Ok(sessions)
}
