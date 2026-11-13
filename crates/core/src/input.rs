use crate::{Result, image, process, session::Record, skills::Skill};
use std::{
    collections::HashSet,
    env, fs,
    io::Read,
    ops::Range,
    path::{Path, PathBuf},
    process::Command,
    sync::{Arc, atomic::AtomicBool},
    time::Duration,
};

pub struct Message {
    pub text: String,
    pub attachments: Vec<Record>,
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
        return env::home_dir().unwrap_or_default().join(rest);
    }
    cwd.join(name)
}

const TEXT_LIMIT: u64 = 1024 * 1024;
const CONTEXT_LIMIT: usize = 4 * 1024 * 1024;
const IMAGE_COUNT_LIMIT: usize = 8;

fn read_file(path: &Path, limit: u64) -> Result<Vec<u8>> {
    if !fs::metadata(path)?.is_file() {
        return Err(format!("{} is not a regular file", path.display()).into());
    }
    let mut bytes = vec![];
    fs::File::open(path)?.take(limit + 1).read_to_end(&mut bytes)?;
    if bytes.len() as u64 > limit {
        return Err(format!("{} exceeds {} MiB", path.display(), limit / (1024 * 1024)).into());
    }
    Ok(bytes)
}

fn text_content(bytes: Vec<u8>) -> Result<String> {
    if bytes.len() as u64 > TEXT_LIMIT {
        return Err("Text file exceeds 1 MiB".into());
    }
    if bytes.contains(&0) {
        return Err("Expected text or a PNG, JPEG, GIF or WebP image".into());
    }
    Ok(String::from_utf8(bytes)?)
}

fn read_attachment(path: PathBuf) -> Result<Record> {
    let bytes = read_file(&path, image::LIMIT)?;
    if let Some(image) = image::content(&bytes) {
        return Ok(Record::ImageAttachment { path, image });
    }
    Ok(Record::Attachment { label: "File".into(), path, text: text_content(bytes)? })
}

pub fn prepare(text: String, cwd: &Path, skill: Option<&Skill>) -> Result<Message> {
    let mut size = text.len();
    let mut snapshots = vec![];
    let mut seen = HashSet::new();
    if let Some(skill) = skill {
        let canonical = skill.path.canonicalize().map_err(|e| format!("{}: {e}", skill.path.display()))?;
        let body =
            text_content(read_file(&skill.path, TEXT_LIMIT)?).map_err(|e| format!("{}: {e}", skill.path.display()))?;
        let label = format!("Skill: /{}", skill.name);
        size += body.len() + label.len() + skill.path.as_os_str().len() + 5;
        if size > CONTEXT_LIMIT {
            return Err("Attached context exceeds 4 MiB".into());
        }
        seen.insert(canonical);
        // Keep the discovered (possibly symlinked) skill path used in the instructions.
        snapshots.push(Record::Attachment { label, path: skill.path.clone(), text: body });
    }
    let mut images = 0;
    // Mentions are optional context; a typo or unreadable file must not prevent sending.
    for mention in mentions(&text).into_iter().filter(|m| m.closed && !m.path.is_empty()) {
        let Ok(canonical) = path(cwd, &mention.path).canonicalize() else {
            continue;
        };
        if seen.contains(&canonical) {
            continue;
        }
        let Ok(attachment) = read_attachment(canonical.clone()) else {
            continue;
        };
        match &attachment {
            Record::Attachment { label, path, text } => {
                let added = text.len() + label.len() + path.as_os_str().len() + 5;
                if size + added > CONTEXT_LIMIT {
                    continue;
                }
                size += added;
            }
            Record::ImageAttachment { .. } => {
                if images >= IMAGE_COUNT_LIMIT {
                    continue;
                }
                images += 1;
            }
            _ => unreachable!(),
        }
        seen.insert(canonical);
        snapshots.push(attachment);
    }
    Ok(Message { text, attachments: snapshots })
}

pub struct Entry {
    pub text: String,
    pub label: String,
}

pub struct Menu {
    pub range: Range<usize>,
    pub query: String,
    pub exact_file: bool,
    pub entries: Vec<Entry>,
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

// The mention being typed at the cursor: the range to replace and its query.
// Offsets are in characters.
pub fn mention_at(text: &str, cursor: usize) -> Option<(Range<usize>, String)> {
    let prefix: String = text.chars().take(cursor).collect();
    let mention = mentions(&prefix).pop()?;
    if mention.range.end != cursor || (mention.quoted && mention.closed) {
        return None;
    }
    let range = mentions(text).into_iter().find(|m| m.range.start == mention.range.start)?.range;
    Some((range, mention.path))
}

// Explicit paths list their directory; anything else searches the file index.
pub fn is_path(query: &str) -> bool {
    ["/", "~/", "./", "../"].iter().any(|p| query.starts_with(p))
}

pub fn complete(range: Range<usize>, query: String, cwd: &Path, files: &[String]) -> Menu {
    let mut matches = if is_path(&query) {
        let (parent, leaf) = query.rsplit_once('/').unwrap_or(("", &query));
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
            .filter_map(|name| Some((score(name, &query)?, name.clone())))
            .collect()
    };
    matches.sort_unstable();
    let entries = matches.into_iter().take(40).map(|(_, name)| file_entry(name)).collect();
    Menu { range, exact_file: !query.is_empty() && path(cwd, &query).is_file(), query, entries }
}

pub fn index(cwd: &Path) -> Result<Vec<String>> {
    let mut data = vec![];
    let exit = process::run(
        Command::new("rg").args(["--files", "--hidden", "--no-require-git", "-0", "-g", "!.git"]).current_dir(cwd),
        None,
        Duration::from_secs(10),
        &Arc::new(AtomicBool::new(false)),
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
}
