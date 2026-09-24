use crate::Result;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    env, fs,
    io::{BufReader, BufWriter, Write},
    os::unix::fs::OpenOptionsExt,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

#[derive(Clone, Copy, Serialize, Deserialize, PartialEq, Debug)]
pub enum Kind {
    User,
    Agent,
    Thought,
    Call,
    Notice,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct Block {
    pub kind: Kind,
    pub text: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool: Option<Tool>,
}
impl Block {
    pub fn new(kind: Kind, text: impl Into<String>) -> Self {
        Self { kind, text: text.into(), tool: None }
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

#[derive(Serialize, Deserialize)]
pub struct Node {
    pub parent: Option<usize>,
    pub items: Vec<Value>,
    pub blocks: Vec<Block>,
    pub usage: Option<Usage>,
}

#[derive(Serialize, Deserialize)]
pub struct Session {
    pub id: String,
    pub cwd: PathBuf,
    pub instructions: String,
    pub model: String,
    pub effort: Option<String>,
    pub context: u64,
    pub nodes: Vec<Node>,
    pub cursor: Option<usize>,
}

pub fn unique_id() -> String {
    format!("{}-{}", SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_nanos(), std::process::id())
}

pub fn state_dir() -> PathBuf {
    env::var_os("XDG_STATE_HOME").map(PathBuf::from).unwrap_or_else(|| home().join(".local/state")).join("mu")
}

fn home() -> PathBuf {
    env::home_dir().unwrap_or_else(|| PathBuf::from("."))
}

/// A session is read and written by at most one `mu` process at a time. The
/// lock lives on the open file, so the kernel drops it when the process exits
/// (even on crash): there is no stale-lock state to detect or clean up.
pub struct Lock {
    _file: fs::File,
}

impl Lock {
    pub fn acquire(id: &str) -> Result<Self> {
        let dir = state_dir();
        fs::create_dir_all(&dir)?;
        let file = fs::OpenOptions::new().write(true).create(true).mode(0o600).open(dir.join(format!("{id}.lock")))?;
        match file.try_lock() {
            Ok(()) => Ok(Self { _file: file }),
            Err(fs::TryLockError::WouldBlock) => Err(format!("Session {id} is open in another mu process").into()),
            Err(error) => Err(error.into()),
        }
    }
}

impl Session {
    pub fn new(cwd: PathBuf, model: String, effort: Option<String>, context: u64, skills: &[Skill]) -> Self {
        Self {
            id: unique_id(),
            instructions: instructions(&cwd, skills),
            cwd,
            model,
            effort,
            context,
            nodes: vec![],
            cursor: None,
        }
    }
    fn ancestors(&self) -> impl Iterator<Item = usize> + '_ {
        std::iter::successors(self.cursor, |&i| self.nodes[i].parent)
    }
    pub fn path(&self) -> Vec<usize> {
        let mut path: Vec<_> = self.ancestors().collect();
        path.reverse();
        path
    }
    pub fn input(&self) -> Vec<Value> {
        self.path().iter().flat_map(|&i| self.nodes[i].items.clone()).collect()
    }
    pub fn usage(&self) -> Usage {
        self.ancestors().find_map(|i| self.nodes[i].usage).unwrap_or_default()
    }
    pub fn push(&mut self, items: Vec<Value>, blocks: Vec<Block>, usage: Option<Usage>) {
        self.nodes.push(Node { parent: self.cursor, items, blocks, usage });
        self.cursor = Some(self.nodes.len() - 1);
    }
    pub fn user(&mut self, text: String, content: String) {
        self.push(vec![json!({"role":"user", "content":content})], vec![Block::new(Kind::User, text)], None);
    }
    pub fn save(&self) -> Result<()> {
        self.save_in(&state_dir())
    }
    fn save_in(&self, dir: &Path) -> Result<()> {
        if self.nodes.is_empty() {
            return Ok(());
        }
        fs::create_dir_all(dir)?;
        let path = dir.join(format!("{}.json", self.id));
        write_json(&path, self, true)?;
        // The listing cache is disposable; its failure mustn't fail a saved turn.
        let _ = self.cache_summary(&path);
        Ok(())
    }
    fn cache_summary(&self, path: &Path) -> Result<Summary> {
        let metadata = fs::metadata(path)?;
        let summary = Summary {
            modified: metadata.modified()?,
            len: metadata.len(),
            label: (!self.nodes.is_empty())
                .then(|| format!("{}  {}  [{} · {}]", self.id, self.title(), self.cwd.display(), self.model)),
        };
        let _ = write_json(&path.with_extension("meta"), &summary, false);
        Ok(summary)
    }
    pub fn load(path: &Path) -> Result<Self> {
        let s: Self = serde_json::from_reader(BufReader::new(fs::File::open(path)?))?;
        if s.cursor.is_some_and(|i| i >= s.nodes.len())
            || s.nodes.iter().enumerate().any(|(i, n)| n.parent.is_some_and(|p| p >= i))
        {
            return Err("Invalid session tree".into());
        }
        // Don't let an edited session's id become a write path.
        if s.id.is_empty() || !s.id.chars().all(|c| c.is_ascii_digit() || c == '-') {
            return Err("Invalid session id".into());
        }
        Ok(s)
    }
    pub fn last_text(&self, kind: Kind) -> Option<String> {
        self.ancestors()
            .flat_map(|i| self.nodes[i].blocks.iter().rev())
            .find(|b| b.kind == kind)
            .map(|b| b.text.clone())
    }
    pub fn title(&self) -> String {
        self.nodes
            .iter()
            .flat_map(|n| &n.blocks)
            .find(|b| b.kind == Kind::User)
            .map(|b| b.text.lines().next().unwrap_or("").chars().take(60).collect())
            .unwrap_or_else(|| "(empty)".into())
    }
    // Iterative DFS: very deep tool loops don't consume the stack. Linear
    // paths stay flat; fork guides continue through their descendants.
    pub fn tree(&self) -> Vec<(Option<usize>, String)> {
        let mut children = vec![vec![]; self.nodes.len() + 1];
        for (i, n) in self.nodes.iter().enumerate() {
            children[n.parent.map_or(0, |p| p + 1)].push(i);
        }
        let mut result = vec![];
        // (node, prefix for this line, prefix for its children, fork depth)
        let mut stack: Vec<(Option<usize>, String, String, usize)> = vec![(None, String::new(), String::new(), 0)];
        while let Some((node, line_prefix, continuation, depth)) = stack.pop() {
            let label = node
                .map(|i| {
                    self.nodes[i]
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
                    let line = format!("{continuation}{}", if last { "└─ " } else { "├─ " });
                    let next = if depth < 16 {
                        format!("{continuation}{}", if last { "     " } else { "│    " })
                    } else {
                        continuation.clone()
                    };
                    stack.push((Some(child), line, next, (depth + 1).min(16)));
                } else {
                    stack.push((Some(child), continuation.clone(), continuation.clone(), depth));
                }
            }
        }
        result
    }
}

// Small sidecars keep /resume independent of transcript/image size. Old sessions
// are indexed lazily; a changed file or broken/missing cache is read once again.
#[derive(Serialize, Deserialize)]
struct Summary {
    modified: SystemTime,
    len: u64,
    label: Option<String>,
}

fn write_json(path: &Path, value: &impl Serialize, durable: bool) -> Result<()> {
    let tmp = path.with_extension(format!("{}.tmp", path.extension().unwrap_or_default().to_string_lossy()));
    let mut file =
        BufWriter::new(fs::OpenOptions::new().write(true).create(true).truncate(true).mode(0o600).open(&tmp)?);
    serde_json::to_writer(&mut file, value)?;
    file.flush()?;
    if durable {
        file.get_ref().sync_all()?;
    }
    fs::rename(tmp, path)?;
    Ok(())
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
        .filter(|p| p.extension().is_some_and(|e| e == "json"))
        .collect();
    paths.sort_by_cached_key(|p| std::cmp::Reverse(fs::metadata(p).and_then(|m| m.modified()).ok()));
    Ok(paths
        .into_iter()
        .filter_map(|path| {
            let metadata = fs::metadata(&path).ok()?;
            let cached: Option<Summary> =
                fs::read(path.with_extension("meta")).ok().and_then(|data| serde_json::from_slice(&data).ok());
            let summary = cached
                .filter(|s| s.len == metadata.len() && Some(s.modified) == metadata.modified().ok())
                .or_else(|| Session::load(&path).ok()?.cache_summary(&path).ok())?;
            summary.label.map(|label| (path, label))
        })
        .collect())
}

fn frontmatter(text: &str, field: &str) -> Option<String> {
    let mut lines = text.lines();
    if lines.next()?.trim() != "---" {
        return None;
    }
    let front: Vec<_> = lines.take_while(|l| l.trim() != "---").collect();
    for (i, line) in front.iter().enumerate() {
        if let Some(value) = line.strip_prefix(&format!("{field}:")) {
            let value = value.trim();
            if matches!(value, ">" | "|" | ">-" | "|-") {
                return Some(
                    front[i + 1..]
                        .iter()
                        .take_while(|l| l.starts_with(' '))
                        .map(|l| l.trim())
                        .collect::<Vec<_>>()
                        .join(" "),
                );
            }
            return Some(value.trim_matches(['\'', '"']).to_string());
        }
    }
    None
}

fn discover_skills(dir: &Path, found: &mut Vec<PathBuf>, depth: usize) {
    if depth > 4 {
        return;
    }
    if dir.join("SKILL.md").is_file() {
        found.push(dir.join("SKILL.md"));
        return;
    }
    if let Ok(entries) = fs::read_dir(dir) {
        let mut entries: Vec<_> = entries.filter_map(|e| e.ok()).map(|e| e.path()).filter(|p| p.is_dir()).collect();
        entries.sort();
        for p in entries {
            discover_skills(&p, found, depth + 1);
        }
    }
}

#[derive(Clone)]
pub struct Skill {
    pub name: String,
    pub path: PathBuf,
    pub description: String,
}

pub fn skills(cwd: &Path) -> Vec<Skill> {
    skills_in(&home(), cwd)
}

fn skills_in(home: &Path, cwd: &Path) -> Vec<Skill> {
    let mut paths = vec![];
    discover_skills(&home.join(".agents/skills"), &mut paths, 0);
    for dir in cwd.ancestors().collect::<Vec<_>>().iter().rev() {
        discover_skills(&dir.join(".agents/skills"), &mut paths, 0);
    }
    let mut seen = std::collections::HashSet::new();
    paths
        .into_iter()
        .filter_map(|path| {
            // Deduplicate by target, but keep the discovered path: instructions should show
            // the symlink the user set up, not the (often store-hashed) path it points to.
            if !seen.insert(path.canonicalize().ok()?) {
                return None;
            }
            let text = fs::read_to_string(&path).ok()?;
            let name = frontmatter(&text, "name").unwrap_or_else(|| {
                path.parent().unwrap().file_name().unwrap_or_default().to_string_lossy().into_owned()
            });
            Some(Skill { name, description: frontmatter(&text, "description")?, path })
        })
        .collect()
}

fn instructions(cwd: &Path, skills: &[Skill]) -> String {
    let mut s = "You are coding agent".to_string();
    let ancestors: Vec<_> = cwd.ancestors().collect();
    let project: Vec<_> = ancestors.iter().rev().filter_map(|p| fs::read_to_string(p.join("AGENTS.md")).ok()).collect();
    if !project.is_empty() {
        s.push_str("\n\nProject Instructions:\n");
        s.push_str(&project.join("\n\n"));
    }
    let entries: Vec<_> = skills.iter().map(|s| format!("{}: {}", s.path.display(), s.description)).collect();
    if !entries.is_empty() {
        s.push_str("\n\nSkills:\n");
        s.push_str(&entries.join("\n"));
    }
    s
}
