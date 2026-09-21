use crate::Result;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    env, fs,
    io::{BufReader, BufWriter, Write},
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

#[derive(Clone, Copy, Serialize, Deserialize, PartialEq, Debug)]
pub enum Kind {
    User,
    Agent,
    Thought,
    Call,
    Output,
    Notice,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct Block {
    pub kind: Kind,
    pub text: String,
}
impl Block {
    pub fn new(kind: Kind, text: impl Into<String>) -> Self {
        Self { kind, text: text.into() }
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
    env::var_os("HOME").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("."))
}

impl Session {
    pub fn new(cwd: PathBuf, model: String, effort: Option<String>) -> Self {
        Self { id: unique_id(), instructions: instructions(&cwd), cwd, model, effort, nodes: vec![], cursor: None }
    }
    pub fn path(&self) -> Vec<usize> {
        let mut path = vec![];
        let mut cursor = self.cursor;
        while let Some(i) = cursor {
            path.push(i);
            cursor = self.nodes[i].parent;
        }
        path.reverse();
        path
    }
    pub fn input(&self) -> Vec<Value> {
        self.path().iter().flat_map(|&i| self.nodes[i].items.clone()).collect()
    }
    pub fn usage(&self) -> Usage {
        self.path().iter().rev().find_map(|&i| self.nodes[i].usage).unwrap_or_default()
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
        self.path()
            .iter()
            .rev()
            .flat_map(|&i| self.nodes[i].blocks.iter().rev())
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
    // Iterative DFS: very deep tool loops don't consume the stack.
    pub fn tree(&self) -> Vec<(Option<usize>, usize, String)> {
        let mut children = vec![vec![]; self.nodes.len() + 1];
        for (i, n) in self.nodes.iter().enumerate() {
            children[n.parent.map_or(0, |p| p + 1)].push(i);
        }
        let mut result = vec![(None, 0, "root".into())];
        let mut stack: Vec<_> = children[0].iter().rev().map(|&i| (i, 1)).collect();
        while let Some((i, depth)) = stack.pop() {
            let n = &self.nodes[i];
            let b = n.blocks.iter().find(|b| matches!(b.kind, Kind::User | Kind::Agent | Kind::Call));
            let label = b
                .map(|b| {
                    format!(
                        "{:?}: {}",
                        b.kind,
                        b.text.lines().next().unwrap_or("").chars().take(80).collect::<String>()
                    )
                })
                .unwrap_or_else(|| "step".into());
            result.push((Some(i), depth, label));
            stack.extend(children[i + 1].iter().rev().map(|&j| (j, depth + 1)));
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
    use std::os::unix::fs::OpenOptionsExt;
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
    let mut paths = vec![];
    discover_skills(&home().join(".agents/skills"), &mut paths, 0);
    for dir in cwd.ancestors().collect::<Vec<_>>().iter().rev() {
        discover_skills(&dir.join(".agents/skills"), &mut paths, 0);
    }
    let mut seen = std::collections::HashSet::new();
    paths
        .into_iter()
        .filter_map(|path| {
            let path = path.canonicalize().ok()?;
            if !seen.insert(path.clone()) {
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

fn instructions(cwd: &Path) -> String {
    let mut s = "You are coding agent".to_string();
    let ancestors: Vec<_> = cwd.ancestors().collect();
    let project: Vec<_> = ancestors.iter().rev().filter_map(|p| fs::read_to_string(p.join("AGENTS.md")).ok()).collect();
    if !project.is_empty() {
        s.push_str("\n\nProject Instructions:\n");
        s.push_str(&project.join("\n\n"));
    }
    let entries: Vec<_> = skills(cwd).iter().map(|s| format!("{}: {}", s.path.display(), s.description)).collect();
    if !entries.is_empty() {
        s.push_str("\n\nSkills:\n");
        s.push_str(&entries.join("\n"));
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn branches_preserve_future() {
        let mut s = Session::new(PathBuf::from("/nonexistent"), "m".into(), None);
        s.user("one".into(), "one".into());
        s.user("two".into(), "two".into());
        s.cursor = Some(0);
        s.user("other".into(), "other".into());
        assert_eq!(s.path(), vec![0, 2]);
        assert_eq!(s.nodes.len(), 3);
        s.cursor = Some(1);
        assert_eq!(s.input()[1]["content"], "two");
        assert_eq!(s.tree().len(), 4);
    }
    #[test]
    fn lazy_sessions_and_listing_cache() {
        let dir = env::temp_dir().join(format!("mu-sessions-test-{}", unique_id()));
        let mut s = Session::new(PathBuf::from("/nonexistent"), "m".into(), None);
        s.save_in(&dir).unwrap();
        assert!(!dir.exists(), "empty sessions must not create files or directories");

        s.user("hello".into(), "hello".into());
        s.save_in(&dir).unwrap();
        let path = dir.join(format!("{}.json", s.id));
        let cache = path.with_extension("meta");
        assert!(sessions_in(&dir).unwrap()[0].1.contains("hello"));
        assert_eq!(Session::load(&path).unwrap().nodes.len(), 1);

        // A valid cache is used as-is, without parsing the transcript again.
        let mut summary: Summary = serde_json::from_slice(&fs::read(&cache).unwrap()).unwrap();
        summary.label = Some("cached label".into());
        write_json(&cache, &summary, false).unwrap();
        assert_eq!(sessions_in(&dir).unwrap()[0].1, "cached label");

        // Data changed by an older version (no sidecar update) invalidates the cache.
        s.model = "different model".into();
        write_json(&path, &s, false).unwrap();
        assert!(sessions_in(&dir).unwrap()[0].1.contains("different model"));
        fs::write(&cache, "broken").unwrap();
        assert_eq!(sessions_in(&dir).unwrap().len(), 1);
        fs::remove_file(&cache).unwrap();
        assert_eq!(sessions_in(&dir).unwrap().len(), 1);
        assert!(cache.exists(), "legacy sessions should be indexed once");

        // Moving to the root isn't an empty session: its branches must survive.
        s.cursor = None;
        s.save_in(&dir).unwrap();
        assert_eq!(Session::load(&path).unwrap().nodes.len(), 1);
        assert_eq!(Session::load(&path).unwrap().cursor, None);

        // Hide old empty sessions, but don't delete user files.
        let empty = Session::new(PathBuf::from("/nonexistent"), "m".into(), None);
        let empty_path = dir.join(format!("{}.json", empty.id));
        write_json(&empty_path, &empty, false).unwrap();
        assert_eq!(sessions_in(&dir).unwrap().len(), 1);
        assert!(empty_path.exists());
        fs::remove_dir_all(dir).unwrap();
    }
    #[test]
    fn description_formats() {
        assert_eq!(frontmatter("---\ndescription: 'a skill'\n---", "description"), Some("a skill".into()));
        assert_eq!(
            frontmatter("---\ndescription: >-\n  a long\n  skill\nname: x\n---", "description"),
            Some("a long skill".into())
        );
    }
    #[test]
    fn only_warn_on_observed_cache_loss() {
        let old = Usage { input: 2000, output: 5, cached: Some(1024) };
        assert!(Usage { cached: Some(0), ..old }.cache_miss(old));
        assert!(!Usage { cached: None, ..old }.cache_miss(old));
        assert!(!Usage { input: 10, cached: Some(0), ..old }.cache_miss(old));
    }
}
