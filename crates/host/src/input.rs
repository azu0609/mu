use crate::{Result, image, process, read_limited, session::Entry, skills::Skill};
use mu_wire::{self as wire, Attached, MentionEntry, Resolved, Sender, mentions};
use serde_json::Value;
use std::{
    collections::HashMap,
    env, fs,
    ops::Range,
    path::{Path, PathBuf},
    process::Command,
    sync::{Arc, atomic::AtomicBool},
    time::Duration,
};

pub struct Message {
    pub text: String,
    pub attachments: Vec<Attachment>,
    // The session it comes from, when not from the user.
    pub from: Option<Sender>,
}

// A file as it was when the message mentioning it was sent.
pub enum Attachment {
    Text { label: String, path: PathBuf, text: String },
    Image { path: PathBuf, image: Value },
}

impl From<Attachment> for Entry {
    fn from(attachment: Attachment) -> Self {
        match attachment {
            Attachment::Text { label, path, text } => Self::Attachment { label, path, text },
            Attachment::Image { path, image } => Self::ImageAttachment { path, image },
        }
    }
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

pub fn text_content(bytes: Vec<u8>) -> Result<String> {
    if bytes.len() as u64 > TEXT_LIMIT {
        return Err("Text file exceeds 1 MiB".into());
    }
    let unreadable = || "Expected text or a PNG, JPEG, GIF or WebP image".into();
    if bytes.contains(&0) {
        return Err(unreadable());
    }
    String::from_utf8(bytes).map_err(|_| unreadable())
}

fn read_attachment(path: PathBuf) -> Result<Attachment> {
    let bytes = read_limited(&path, image::LIMIT)?;
    if let Some(image) = image::content(&bytes) {
        return Ok(Attachment::Image { path, image });
    }
    Ok(Attachment::Text { label: "File".into(), path, text: text_content(bytes)? })
}

pub fn prepare(text: String, cwd: &Path, skill: Option<&Skill>) -> Result<Message> {
    let attachments = gather(&text, cwd, skill)?.attachments;
    Ok(Message { text, attachments, from: None })
}

pub fn resolve(text: &str, cwd: &Path, skill: Option<&Skill>) -> Result<Vec<Resolved>> {
    let Gathered { attachments, mentioned } = gather(text, cwd, skill)?;
    Ok(mentioned
        .into_iter()
        .map(|(path, index)| match &attachments[index] {
            Attachment::Text { path: file, text, .. } => Resolved {
                path,
                file: file.clone(),
                kind: Attached::Text {
                    lines: text.lines().count() as u32,
                    head: text.lines().take(8).map(|line| line.chars().take(40).collect::<String>() + "\n").collect(),
                },
            },
            Attachment::Image { path: file, .. } => Resolved { path, file: file.clone(), kind: Attached::Image },
        })
        .collect())
}

struct Gathered {
    attachments: Vec<Attachment>,
    // Each mention that attaches something, by its path as written, and which attachment it is.
    mentioned: Vec<(String, usize)>,
}

fn gather(text: &str, cwd: &Path, skill: Option<&Skill>) -> Result<Gathered> {
    let mut size = text.len();
    let mut snapshots = vec![];
    let mut seen = HashMap::new();
    let mut mentioned = vec![];
    if let Some(skill) = skill {
        let canonical = skill.path.canonicalize().map_err(|e| format!("{}: {e}", skill.path.display()))?;
        let body = text_content(read_limited(&skill.path, TEXT_LIMIT)?)
            .map_err(|e| format!("{}: {e}", skill.path.display()))?;
        let label = format!("Skill: /{}", skill.name);
        size += body.len() + label.len() + skill.path.as_os_str().len() + 5;
        if size > CONTEXT_LIMIT {
            return Err("Attached context exceeds 4 MiB".into());
        }
        seen.insert(canonical, 0);
        snapshots.push(Attachment::Text { label, path: skill.path.clone(), text: body });
    }
    let mut images = 0;
    // Mentions are optional context; a typo or unreadable file must not prevent sending.
    for mention in mentions(text).into_iter().filter(|m| m.closed && !m.path.is_empty()) {
        let Ok(canonical) = path(cwd, &mention.path).canonicalize() else {
            continue;
        };
        if let Some(&index) = seen.get(&canonical) {
            mentioned.push((mention.path, index));
            continue;
        }
        let Ok(attachment) = read_attachment(canonical.clone()) else {
            continue;
        };
        match &attachment {
            Attachment::Text { label, path, text } => {
                let added = text.len() + label.len() + path.as_os_str().len() + 5;
                if size + added > CONTEXT_LIMIT {
                    continue;
                }
                size += added;
            }
            Attachment::Image { .. } => {
                if images >= IMAGE_COUNT_LIMIT {
                    continue;
                }
                images += 1;
            }
        }
        seen.insert(canonical, snapshots.len());
        mentioned.push((mention.path, snapshots.len()));
        snapshots.push(attachment);
    }
    Ok(Gathered { attachments: snapshots, mentioned })
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

fn file_entry(name: String) -> MentionEntry {
    let directory = name.ends_with('/');
    let token = if name.chars().any(|c| c.is_whitespace() || matches!(c, '"' | '\\')) || name.starts_with('@') {
        let mut quoted = serde_json::to_string(&name).unwrap();
        if directory {
            // Keep the quoted path open while navigating.
            quoted.pop();
        }
        quoted
    } else {
        name.clone()
    };
    MentionEntry { text: format!("@{token}{}", if directory { "" } else { " " }), label: name }
}

pub fn mention_at(text: &str, cursor: u32) -> Option<(Range<u32>, String)> {
    let prefix: String = text.chars().take(cursor as usize).collect();
    let mention = mentions(&prefix).pop()?;
    if mention.end != cursor || (mention.quoted && mention.closed) {
        return None;
    }
    let whole = mentions(text).into_iter().find(|m| m.start == mention.start)?;
    Some((whole.start..whole.end, mention.path))
}

pub fn is_path(query: &str) -> bool {
    ["/", "~/", "./", "../"].iter().any(|p| query.starts_with(p))
}

pub fn complete(range: Range<u32>, query: String, cwd: &Path, files: &[String]) -> wire::Mention {
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
    wire::Mention {
        start: range.start,
        end: range.end,
        exact_file: !query.is_empty() && path(cwd, &query).is_file(),
        query,
        entries,
    }
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
