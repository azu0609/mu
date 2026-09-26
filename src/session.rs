use crate::{Result, home, instructions, skills::Skill};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    env, fs,
    io::{BufRead, BufReader, BufWriter, Seek, SeekFrom, Write},
    os::unix::fs::OpenOptionsExt,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Kind {
    User,
    Agent,
    Thought,
    Call,
    Notice,
}

#[derive(Clone)]
pub struct Block {
    pub kind: Kind,
    pub text: String,
    pub tool: Option<Tool>,
}

impl Block {
    pub fn new(kind: Kind, text: impl Into<String>) -> Self {
        Self { kind, text: text.into(), tool: None }
    }

    pub fn from_item(item: &Value, partial: bool) -> Option<Block> {
        if item["role"] == "user"
            && let Some(text) = item["content"].as_str()
        {
            return Some(Block::new(Kind::User, text));
        }
        let texts = |field: &str, typ: &str| {
            item[field]
                .as_array()
                .map(|a| {
                    a.iter()
                        .filter(|v| v["type"] == typ)
                        .filter_map(|v| v["text"].as_str())
                        .collect::<Vec<_>>()
                        .join("\n")
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

#[derive(Clone, Serialize, Deserialize)]
pub struct Tool {
    pub call_id: String,
    pub status: ToolStatus,
    pub output: ToolOutput,
}

#[derive(Clone, Default, Serialize, Deserialize)]
pub struct ToolOutput {
    pub text: String,
    pub log: Option<PathBuf>,
    pub truncated: bool,
}

#[derive(Clone, Copy, Serialize, Deserialize)]
pub enum ToolStatus {
    Pending,
    Running,
    Exited(i32),
    TimedOut(u64),
    Cancelled,
    Error,
}

impl ToolStatus {
    pub fn summary(self) -> Option<String> {
        Some(match self {
            Self::Exited(0) | Self::Error => return None,
            Self::Pending => "pending".into(),
            Self::Running => "running".into(),
            Self::Exited(code) => format!("exit {code}"),
            Self::TimedOut(ms) => format!("timed out after {ms} ms; process group killed"),
            Self::Cancelled => "cancelled; process group killed".into(),
        })
    }
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
    Tool(Tool),
    Node(Node),
    Cursor { node: Option<usize> },
    Model(Model),
}

impl Record {
    fn is_entry(&self) -> bool {
        matches!(self, Self::Item { .. } | Self::Attachment { .. } | Self::Tool(_))
    }

    fn input(&self) -> Option<Value> {
        Some(match self {
            Self::Item { item } => item.clone(),
            Self::Attachment { label, path, text } => {
                json!({"role":"user", "content":format!("{label}: {}\n{text}", path.display())})
            }
            Self::Tool(tool) => tool.item(),
            _ => return None,
        })
    }
}

#[derive(Default, Serialize, Deserialize)]
pub struct Node {
    pub parent: Option<usize>,
    pub usage: Option<Usage>,
    #[serde(skip)]
    start: usize,
    #[serde(skip)]
    pub blocks: Vec<Block>,
}

impl Tool {
    fn item(&self) -> Value {
        let mut text = self.output.text.clone();
        if let Some(summary) = self.status.summary() {
            text.push_str(&format!("\n[{summary}]"));
        }
        if self.output.truncated
            && let Some(log) = &self.output.log
        {
            text.push_str(&format!(
                "\n[Output log (temporary): {} — read needed ranges with sed, tail, or grep]",
                log.display()
            ));
        }
        json!({"type":"function_call_output", "call_id":self.call_id, "output":text})
    }
}

#[derive(Clone, PartialEq, Serialize, Deserialize)]
pub struct Model {
    #[serde(rename = "model")]
    pub name: String,
    pub effort: Option<String>,
    pub context: u64,
}

#[derive(Serialize, Deserialize)]
pub struct Header {
    version: u32,
    pub id: String,
    title: String,
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

    pub fn push(&mut self, entries: Vec<Record>, usage: Option<Usage>) {
        let parent = self.cursor();
        debug_assert!(entries.iter().all(Record::is_entry));
        self.records.extend(entries);
        self.record(Record::Node(Node { parent, usage, ..Node::default() })).expect("valid cursor");
    }

    pub fn user(&mut self, text: String, attachments: Vec<Record>) {
        if self.file.is_none() && self.nodes().next().is_none() {
            self.header.title = text.lines().next().unwrap_or("").chars().take(60).collect();
        }
        let mut entries = vec![Record::Item { item: json!({"role":"user", "content":text}) }];
        entries.extend(attachments);
        self.push(entries, None);
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
                for entry in &self.records[node.start..] {
                    match entry {
                        Record::Item { item } => node.blocks.extend(Block::from_item(item, false)),
                        Record::Attachment { label, path, .. } => {
                            node.blocks.push(Block::new(Kind::Notice, format!("{label}: {}", path.display())))
                        }
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

    pub fn load(path: &Path) -> Result<Self> {
        let file = fs::OpenOptions::new().read(true).write(true).open(path)?;
        lock(&file)?;
        let mut reader = BufReader::new(&file);
        let mut line = vec![];
        let mut offset = reader.read_until(b'\n', &mut line)? as u64;
        if line.last() != Some(&b'\n') {
            return Err("Incomplete session header".into());
        }
        let header: Header = serde_json::from_slice(&line)?;
        if header.version != 1
            || path.file_stem().and_then(|s| s.to_str()) != Some(&header.id)
            || header.id.is_empty()
            || !header.id.chars().all(|c| c.is_ascii_digit() || c == '-')
        {
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

    pub fn last_text(&self, kind: Kind) -> Option<String> {
        self.ancestors().flat_map(|i| self.node(i).blocks.iter().rev()).find(|b| b.kind == kind).map(|b| b.text.clone())
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
                    self.node(i)
                        .blocks
                        .iter()
                        .find(|b| matches!(b.kind, Kind::User | Kind::Agent | Kind::Call))
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

pub fn sessions() -> Result<Vec<(PathBuf, String)>> {
    sessions_in(&state_dir())
}

fn sessions_in(dir: &Path) -> Result<Vec<(PathBuf, String)>> {
    if !dir.exists() {
        return Ok(vec![]);
    }
    let mut paths: Vec<_> = fs::read_dir(dir)?
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|e| e == "jsonl"))
        .collect();
    paths.sort_by_cached_key(|p| std::cmp::Reverse(fs::metadata(p).and_then(|m| m.modified()).ok()));
    Ok(paths
        .into_iter()
        .filter_map(|path| {
            let mut line = String::new();
            BufReader::new(fs::File::open(&path).ok()?).read_line(&mut line).ok()?;
            if !line.ends_with('\n') {
                return None;
            }
            let header: Header = serde_json::from_str(&line).ok()?;
            (header.version == 1).then(|| {
                let label = format!("{}  {}  [{}]", header.id, header.title, header.cwd.display());
                (path, label)
            })
        })
        .collect())
}
