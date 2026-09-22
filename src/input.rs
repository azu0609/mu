use crate::{Result, process, session::Skill};
use std::{
    collections::{BTreeMap, HashSet},
    env, fs,
    io::Read,
    ops::Range,
    path::{Path, PathBuf},
    process::Command,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    thread,
    time::Duration,
};

pub const COMMANDS: &[(&str, &str)] = &[
    ("/model", "<model> [effort]"),
    ("/new", "new conversation"),
    ("/resume", "resume session"),
    ("/tree", "conversation tree"),
    ("/copy", "[agent|user]"),
    ("/quit", "quit"),
];

pub struct Message {
    pub text: String,
    pub content: String,
}

fn commands(skills: &[Skill]) -> BTreeMap<String, String> {
    let mut entries = BTreeMap::new();
    // Local skills come last; built-ins always win a name collision.
    for skill in skills {
        if !skill.name.is_empty() && skill.name.chars().all(|c| c.is_alphanumeric() || "-_.".contains(c)) {
            entries.insert(format!("/{}", skill.name), skill.description.clone());
        }
    }
    entries.extend(COMMANDS.iter().map(|(name, desc)| (name.to_string(), desc.to_string())));
    entries
}

pub fn resolve(word: &str, skills: &[Skill]) -> Result<String> {
    let choices = commands(skills);
    if choices.contains_key(word) {
        return Ok(word.into());
    }
    let names: Vec<_> = choices.keys().filter(|name| name.starts_with(word)).cloned().collect();
    match names.as_slice() {
        [name] => Ok(name.clone()),
        [] => Err(format!("Unknown command or skill: {word}").into()),
        _ => Err(format!("Choose a command: {} (↑/↓ then Enter, or Tab)", names.join(", ")).into()),
    }
}

struct Mention {
    range: Range<usize>,
    path: String,
    quoted: bool,
    closed: bool,
}

fn mentions(text: &str) -> Vec<Mention> {
    let chars: Vec<_> = text.chars().collect();
    let mut found = vec![];
    let mut i = 0;
    while i < chars.len() {
        if chars[i] != '@' || (i > 0 && !chars[i - 1].is_whitespace()) || chars.get(i + 1) == Some(&'@') {
            i += 1;
            continue;
        }
        let start = i;
        i += 1;
        let quoted = chars.get(i) == Some(&'"');
        if quoted {
            i += 1;
        }
        let mut path = String::new();
        let mut closed = !quoted;
        while i < chars.len() {
            let c = chars[i];
            if quoted && c == '"' {
                i += 1;
                closed = true;
                break;
            }
            if !quoted && c.is_whitespace() {
                break;
            }
            if quoted && c == '\\' && chars.get(i + 1).is_some_and(|c| matches!(c, '"' | '\\')) {
                i += 1;
            }
            path.push(chars[i]);
            i += 1;
        }
        found.push(Mention { range: start..i, path, quoted, closed });
    }
    found
}

fn path(cwd: &Path, name: &str) -> PathBuf {
    if let Some(rest) = name.strip_prefix("~/") {
        return env::var_os("HOME").map(PathBuf::from).unwrap_or_default().join(rest);
    }
    cwd.join(name)
}

fn read_text(path: &Path) -> Result<String> {
    const LIMIT: u64 = 1024 * 1024;
    if !fs::metadata(path)?.is_file() {
        return Err(format!("{} is not a regular file", path.display()).into());
    }
    let mut bytes = vec![];
    fs::File::open(path)?.take(LIMIT + 1).read_to_end(&mut bytes)?;
    if bytes.len() as u64 > LIMIT {
        return Err(format!("{} exceeds 1 MiB", path.display()).into());
    }
    if bytes.contains(&0) {
        return Err(format!("{} is binary; @ mentions accept text files", path.display()).into());
    }
    Ok(String::from_utf8(bytes)?)
}

pub fn prepare(text: String, cwd: &Path, skill: Option<&Skill>) -> Result<Message> {
    let mut content = text.clone();
    let mut seen = HashSet::new();
    // (label, path, keep): skills keep the discovered path so the model sees the same
    // possibly symlinked path as in the instructions; @ file mentions are shown resolved.
    let mut attachments: Vec<(String, PathBuf, bool)> = vec![];
    if let Some(skill) = skill {
        attachments.push((format!("Skill: /{}", skill.name), skill.path.clone(), true));
    }
    for mention in mentions(&text) {
        if !mention.closed {
            return Err("Unclosed @\"file name\" mention".into());
        }
        if mention.path.is_empty() {
            continue;
        }
        attachments.push(("File".into(), path(cwd, &mention.path), false));
    }
    for (kind, path, keep) in attachments {
        let canonical = path.canonicalize().map_err(|e| format!("{}: {e}", path.display()))?;
        if !seen.insert(canonical.clone()) {
            continue;
        }
        let body = read_text(&path).map_err(|e| format!("{}: {e}", path.display()))?;
        if content.len() + body.len() > 4 * 1024 * 1024 {
            return Err("Attached context exceeds 4 MiB".into());
        }
        let shown = if keep { &path } else { &canonical };
        content.push_str(&format!("\n\n{kind}: {}\n{body}", shown.display()));
    }
    Ok(Message { text, content })
}

pub struct Entry {
    pub text: String,
    pub label: String,
}
pub struct Menu {
    pub range: Range<usize>,
    pub query: String,
    pub file: bool,
    pub exact_file: bool,
    pub entries: Vec<Entry>,
    pub selected: usize,
    pub explicit: bool,
}
impl Menu {
    pub fn replacement(&self) -> Option<String> {
        self.entries.get(self.selected).map(|e| e.text.clone())
    }
}

fn score(candidate: &str, query: &str) -> Option<usize> {
    let candidate = candidate.to_lowercase();
    let query = query.to_lowercase();
    if candidate.starts_with(&query) {
        return Some(0);
    }
    if candidate.rsplit('/').next()?.starts_with(&query) {
        return Some(1);
    }
    let mut rest = candidate.as_str();
    let mut gaps = 2;
    for c in query.chars() {
        let pos = rest.find(c)?;
        gaps += pos;
        rest = &rest[pos + c.len_utf8()..];
    }
    Some(gaps)
}

fn file_entry(name: String) -> Entry {
    let directory = name.ends_with('/');
    let token = if name.chars().any(|c| c.is_whitespace() || matches!(c, '"' | '\\')) || name.starts_with('@') {
        let mut quoted = serde_json::to_string(&name).unwrap();
        if directory {
            quoted.pop();
        } // Keep the quoted path open while navigating.
        quoted
    } else {
        name.clone()
    };
    Entry { text: format!("@{token}{}", if directory { "" } else { " " }), label: name }
}

pub fn menu(text: &str, cursor: usize, cwd: &Path, skills: &[Skill], files: &[String]) -> Option<Menu> {
    let prefix: String = text.chars().take(cursor).collect();
    let trimmed = prefix.trim_start();
    let start = prefix.chars().count() - trimmed.chars().count();
    if trimmed.starts_with('/') && !trimmed.chars().any(char::is_whitespace) {
        let end = text.chars().skip(start).take_while(|c| !c.is_whitespace()).count() + start;
        let entries = commands(skills)
            .into_iter()
            .filter(|(name, _)| name.starts_with(trimmed))
            .map(|(name, desc)| Entry { text: format!("{name} "), label: format!("{name}  {desc}") })
            .collect();
        return Some(Menu {
            range: start..end,
            query: trimmed.into(),
            file: false,
            exact_file: false,
            entries,
            selected: 0,
            explicit: false,
        });
    }
    let mention = mentions(&prefix).pop()?;
    if mention.range.end != cursor || (mention.quoted && mention.closed) {
        return None;
    }
    let range = mentions(text).into_iter().find(|m| m.range.start == mention.range.start)?.range;
    let query = &mention.path;
    let mut matches =
        if query.starts_with('/') || query.starts_with("~/") || query.starts_with("./") || query.starts_with("../") {
            let (parent, leaf) = query.rsplit_once('/').unwrap_or(("", query));
            let parent = format!("{parent}/");
            fs::read_dir(path(cwd, &parent))
                .ok()
                .into_iter()
                .flatten()
                .filter_map(|e| e.ok())
                .take(4096)
                .filter_map(|e| {
                    let name = e.file_name().into_string().ok()?;
                    if !name.starts_with(leaf) || name.chars().any(char::is_control) {
                        return None;
                    }
                    Some((0, format!("{parent}{name}{}", if e.path().is_dir() { "/" } else { "" })))
                })
                .collect::<Vec<_>>()
        } else {
            files
                .iter()
                .take(if query.is_empty() { 40 } else { usize::MAX })
                .filter_map(|name| Some((score(name, query)?, name.clone())))
                .collect()
        };
    matches.sort_unstable();
    let entries = matches.into_iter().take(40).map(|(_, name)| file_entry(name)).collect();
    Some(Menu {
        range,
        query: query.clone(),
        file: true,
        exact_file: !query.is_empty() && path(cwd, query).is_file(),
        entries,
        selected: 0,
        explicit: false,
    })
}

// One bounded scan per active file mention, off the UI thread. No recursive
// in-process walker, watcher, or permanent index; rg supplies ignore semantics.
#[derive(Default)]
pub struct FileSearch {
    cwd: Option<PathBuf>,
    pub files: Vec<String>,
    rx: Option<mpsc::Receiver<Result<Vec<String>>>>,
    handle: Option<thread::JoinHandle<()>>,
    cancel: Arc<AtomicBool>,
}
impl Drop for FileSearch {
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::Relaxed);
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}
impl FileSearch {
    pub fn pending(&self) -> bool {
        self.rx.is_some()
    }
    pub fn start(&mut self, cwd: &Path) {
        if self.cwd.as_deref() == Some(cwd) {
            return;
        }
        *self = Self::default();
        self.cwd = Some(cwd.into());
        let cwd = cwd.to_path_buf();
        let cancel = self.cancel.clone();
        let (tx, rx) = mpsc::channel();
        self.rx = Some(rx);
        self.handle = Some(thread::spawn(move || {
            let result = (|| -> Result<Vec<String>> {
                let mut data = vec![];
                let exit = process::run(
                    Command::new("rg")
                        .args(["--files", "--hidden", "--no-require-git", "-0", "-g", "!.git"])
                        .current_dir(cwd),
                    None,
                    Duration::from_secs(10),
                    &cancel,
                    |stderr, bytes| {
                        if !stderr {
                            if data.len() + bytes.len() > 8 * 1024 * 1024 {
                                return Err("File index exceeds 8 MiB; use @./directory/ instead".into());
                            }
                            data.extend_from_slice(bytes);
                        }
                        Ok(())
                    },
                )?;
                if exit.timed_out || exit.code > 1 {
                    return Err("File search failed; check ripgrep (rg) or use an explicit path".into());
                }
                let mut files: Vec<_> = data
                    .split(|&b| b == 0)
                    .filter_map(|b| std::str::from_utf8(b).ok())
                    .filter(|s| !s.is_empty() && !s.chars().any(char::is_control))
                    .map(str::to_string)
                    .collect();
                files.sort();
                files.dedup();
                Ok(files)
            })();
            let _ = tx.send(result);
        }));
    }
    pub fn poll(&mut self) -> Option<Result<()>> {
        let result = self.rx.as_ref()?.try_recv().ok()?;
        self.rx = None;
        Some(result.map(|files| self.files = files))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{session::unique_id, ui::Editor};

    fn skill(name: &str, path: PathBuf) -> Skill {
        Skill { name: name.into(), path, description: "a skill".into() }
    }
    #[test]
    fn command_prefixes_and_collisions() {
        let skills = vec![skill("review", PathBuf::new()), skill("quit", PathBuf::new())];
        assert_eq!(resolve("/q", &skills).unwrap(), "/quit");
        assert_eq!(resolve("/rev", &skills).unwrap(), "/review");
        assert_eq!(resolve("/resume", &skills).unwrap(), "/resume");
        assert!(resolve("/re", &skills).is_err());
        assert!(resolve("/missing", &skills).is_err());
        assert_eq!(commands(&skills)["/quit"], "quit");
    }
    #[test]
    fn mentions_and_midline_unicode_completion() {
        let m = mentions(r#"email a@b @@literal @src/無.rs @"two \"quotes\".txt""#);
        assert_eq!(m.iter().map(|m| m.path.as_str()).collect::<Vec<_>>(), ["src/無.rs", "two \"quotes\".txt"]);
        assert!(m.iter().all(|m| m.closed));
        assert!(!mentions("@\"unclosed name")[0].closed);
        let mut editor = Editor::default();
        editor.insert("無 @mnrs rest");
        editor.cursor = 7;
        let menu =
            menu(&editor.text(), editor.cursor, Path::new("/nonexistent"), &[], &["src/main.rs".into()]).unwrap();
        assert_eq!(menu.entries[0].label, "src/main.rs");
        editor.replace(menu.range, &menu.entries[0].text);
        assert_eq!(editor.text(), "無 @src/main.rs  rest");
        assert!(menu_for("hello a@b").is_none());
        assert!(menu_for("@\"finished file\"").is_none());
    }
    fn menu_for(text: &str) -> Option<Menu> {
        menu(text, text.chars().count(), Path::new("/nonexistent"), &[], &[])
    }
    #[test]
    fn symlinked_skill_attachments_keep_the_link_not_the_target() {
        let dir = env::temp_dir().join(format!("mu-input-link-test-{}", unique_id()));
        let target = dir.join("store/SKILL.md");
        fs::create_dir_all(target.parent().unwrap()).unwrap();
        fs::write(&target, "body").unwrap();
        let link = dir.join("SKILL.md");
        std::os::unix::fs::symlink(&target, &link).unwrap();
        let message = prepare("/review".into(), &dir, Some(&skill("review", link.clone()))).unwrap();
        assert!(message.content.contains(&format!("Skill: /review: {}", link.display())));
        assert!(!message.content.contains(&target.display().to_string()));
        assert!(message.content.contains("body"));
        fs::remove_dir_all(dir).unwrap();
    }
    #[test]
    fn attachments_are_bounded_snapshots_not_recursive_expansions() {
        let dir = env::temp_dir().join(format!("mu-input-test-{}", unique_id()));
        fs::create_dir(&dir).unwrap();
        let file = dir.join("two words.txt");
        let skill_path = dir.join("SKILL.md");
        fs::write(&file, "first snapshot @not-another-file").unwrap();
        fs::write(&skill_path, "---\nname: review\n---\nSkill body @also-not-a-file").unwrap();
        let text = "/review fix @\"two words.txt\" @\"two words.txt\"".to_string();
        let message = prepare(text.clone(), &dir, Some(&skill("review", skill_path.clone()))).unwrap();
        fs::write(&file, "second snapshot").unwrap();
        assert_eq!(message.text, text);
        assert_eq!(message.content.matches("first snapshot").count(), 1);
        assert!(!message.content.contains("second snapshot"));
        assert!(message.content.contains(&format!("Skill: /review: {}", skill_path.display())));
        assert!(message.content.contains("Skill body @also-not-a-file"));
        assert!(prepare("@missing".into(), &dir, None).is_err());
        assert!(prepare("@\"unfinished".into(), &dir, None).is_err());
        fs::write(&file, [0, 1, 2]).unwrap();
        assert!(prepare("@\"two words.txt\"".into(), &dir, None).is_err());
        fs::write(&file, vec![b'x'; 1024 * 1024 + 1]).unwrap();
        assert!(prepare("@\"two words.txt\"".into(), &dir, None).is_err());
        let explicit = format!("@{}/", dir.display());
        let menu = menu(&explicit, explicit.chars().count(), &dir, &[], &[]).unwrap();
        assert!(menu.entries.iter().any(|e| e.text.starts_with("@\"") && e.label.ends_with("two words.txt")));
        fs::remove_dir_all(dir).unwrap();
    }
}
