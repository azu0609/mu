use crate::{Result, commands};
use mu_client::{MentionEntry as Entry, ModelInfo, Skill, Snippet, counts};
use std::{
    env, fs,
    io::Write,
    ops::Range,
    os::unix::fs::{DirBuilderExt, OpenOptionsExt},
    path::{Path, PathBuf},
    process::Command,
    time::{SystemTime, UNIX_EPOCH},
};

struct PromptTemp(PathBuf);

impl Drop for PromptTemp {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

pub fn edit(text: &str, cwd: &Path) -> Result<String> {
    let editor = env::var("VISUAL")
        .ok()
        .filter(|s| !s.trim().is_empty())
        .or_else(|| env::var("EDITOR").ok().filter(|s| !s.trim().is_empty()))
        .unwrap_or_else(|| "vi".into());
    let nanos = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_nanos();
    let dir = env::temp_dir().canonicalize()?.join(format!("mu-prompt-{nanos}-{}", std::process::id()));
    fs::DirBuilder::new().mode(0o700).create(&dir)?;
    let temp = PromptTemp(dir);
    let path = temp.0.join("PROMPT_EDITMSG");
    let mut file = fs::OpenOptions::new().write(true).create_new(true).mode(0o600).open(&path)?;
    file.write_all(text.as_bytes())?;
    drop(file);

    // Let editor settings contain arguments and quoting, but pass the filename
    // separately so even paths containing shell metacharacters are safe.
    let status = Command::new("sh")
        .arg("-c")
        .arg(format!("exec {editor} \"$@\""))
        .arg("mu-editor")
        .arg(&path)
        // The session's directory is the host's; it may not exist where this client runs.
        .current_dir(if cwd.is_dir() { cwd } else { Path::new(".") })
        .status()
        .map_err(|e| format!("Could not launch editor ({editor}): {e}"))?;
    if !status.success() {
        return Err(format!("Editor ({editor}) exited with {status}").into());
    }
    fs::read_to_string(&path).map_err(|e| format!("Could not read edited prompt: {e}").into())
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
    pub fn new(range: Range<usize>, query: String, entries: Vec<Entry>) -> Self {
        Self { range, query, file: false, exact_file: false, entries, selected: 0, explicit: false }
    }

    pub fn replacement(&self) -> Option<String> {
        self.entries.get(self.selected).map(|e| e.text.clone())
    }

    pub fn inherit(&mut self, old: Option<&Menu>) {
        if let Some(old) = old
            && self.range == old.range
            && self.query == old.query
            && self.file == old.file
        {
            self.selected = old.selected.min(self.entries.len().saturating_sub(1));
            self.explicit = old.explicit;
        }
    }
}

pub fn command_menu(text: &str, cursor: usize, skills: &[Skill]) -> Option<Menu> {
    let prefix: String = text.chars().take(cursor).collect();
    let trimmed = prefix.trim_start();
    let start = prefix.chars().count() - trimmed.chars().count();
    if !trimmed.starts_with('/') || trimmed.chars().any(char::is_whitespace) {
        return None;
    }
    let end = text.chars().skip(start).take_while(|c| !c.is_whitespace()).count() + start;
    let entries = commands::choices(skills)
        .into_iter()
        .filter(|(name, _)| name.starts_with(trimmed))
        .map(|(name, (_, hint))| Entry { text: format!("{name} "), label: format!("{name}  {hint}") })
        .collect();
    Some(Menu::new(start..end, trimmed.into(), entries))
}

const EFFORTS: &[&str] = &["minimal", "low", "medium", "high", "xhigh"];
const CONTEXTS: &[u64] = &[128_000, 200_000, 256_000, 400_000, 1_000_000];

// The argument of /model under the cursor, after the ones already typed.
pub struct ModelArgument {
    range: Range<usize>,
    query: String,
    before: Vec<String>,
}

pub fn model_argument(text: &str, cursor: usize, skills: &[Skill]) -> Option<ModelArgument> {
    let prefix: String = text.chars().take(cursor).collect();
    if prefix.contains('\n') {
        return None;
    }
    let (word, rest) = prefix.trim_start().split_once(char::is_whitespace)?;
    match commands::resolve(word, skills).ok()?.1 {
        commands::Choice::Builtin(command) if command.kind == commands::BuiltinKind::Model => (),
        _ => return None,
    }
    let query: String =
        rest.chars().rev().take_while(|c| !c.is_whitespace()).collect::<Vec<_>>().into_iter().rev().collect();
    let mut before: Vec<String> = rest.split_whitespace().map(Into::into).collect();
    if !query.is_empty() {
        before.pop();
    }
    let start = cursor - query.chars().count();
    let end = cursor + text.chars().skip(cursor).take_while(|c| !c.is_whitespace()).count();
    Some(ModelArgument { range: start..end, query, before })
}

fn tokens(n: u64) -> String {
    counts::compact(n).replace(".0", "")
}

pub fn model_menu(argument: ModelArgument, models: &[ModelInfo]) -> Option<Menu> {
    let query = argument.query.to_lowercase();
    let entry = |text: String, label: String| Entry { text: format!("{text} "), label };
    let entries: Vec<_> = match argument.before.as_slice() {
        [] => models
            .iter()
            .filter(|model| model.name.to_lowercase().contains(&query))
            .map(|model| {
                let context = model.context.map(|n| format!("  {} context", tokens(n))).unwrap_or_default();
                entry(model.name.clone(), format!("{}{context}", model.name))
            })
            .collect(),
        [_] => std::iter::once(entry("-".into(), "-  endpoint default".into()))
            .chain(EFFORTS.iter().map(|&effort| entry(effort.into(), effort.into())))
            .filter(|entry| entry.text.starts_with(&query))
            .collect(),
        [name, _] => {
            let known = models.iter().find(|model| model.name == *name).and_then(|model| model.context);
            let smaller = CONTEXTS.iter().copied().filter(|&n| known.is_none_or(|known| n < known));
            known
                .into_iter()
                .chain(smaller)
                .map(|n| {
                    let compact = tokens(n);
                    let text = if counts::parse(&compact) == Some(n) { compact.clone() } else { n.to_string() };
                    let label = if Some(n) == known { format!("{compact}  full context of {name}") } else { compact };
                    entry(text, label)
                })
                .filter(|entry| entry.text.to_lowercase().starts_with(&query))
                .collect()
        }
        _ => return None,
    };
    Some(Menu::new(argument.range, argument.query, entries))
}

pub fn shell_command(text: &str) -> Option<(bool, &str)> {
    let rest = text.trim_start().strip_prefix('!')?;
    Some(match rest.strip_prefix('!') {
        Some(command) => (false, command.trim()),
        None => (true, rest.trim()),
    })
}

pub fn shell_menu(text: &str, cursor: usize, snippets: &[Snippet]) -> Option<Menu> {
    let prefix: String = text.chars().take(cursor).collect();
    if prefix.contains('\n') {
        return None;
    }
    let (append, query) = shell_command(&prefix)?;
    let bang = if append { "!" } else { "!!" };
    let query = query.to_lowercase();
    let matches = |text: &str| text.to_lowercase().contains(&query);
    let line = |command: &str| command.lines().map(str::trim).collect::<Vec<_>>().join(" ↵ ");
    let entries = snippets
        .iter()
        .filter(|snippet| matches(&snippet.name) || matches(&snippet.command))
        .map(|snippet| Entry {
            text: format!("{bang}{}", snippet.command),
            label: format!("{}  {}", snippet.name, line(&snippet.command)),
        })
        .collect();
    let start = prefix.chars().count() - prefix.trim_start().chars().count();
    Some(Menu::new(start..text.chars().count(), prefix.trim_start().into(), entries))
}
