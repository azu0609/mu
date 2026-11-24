mod commands;
mod image;
mod input;
mod markdown;
mod remote;
mod text;
mod ui;

use crossterm::event::{
    self, Event as Input, KeyCode as Key, KeyEvent, KeyEventKind, KeyModifiers as Mod, MouseEventKind,
};
pub use mu_client::Endpoint;
use mu_client::{
    AppendShell, Block, Change, Checkout, CompleteMention, CreateSession, Edit, Event, GetTree, HostView, Kind,
    ListSkills, Model, ModelInfo, RecallQueued, SendMessage, SessionSummary, SetModel, ShellEvent, Stop, Tool,
    ToolOutput, ToolStatus, TreeEntry, View, counts,
};
use remote::{Incoming, Remote};
use std::{
    collections::HashMap,
    env,
    io::{self, Write},
    path::PathBuf,
    time::{Duration, Instant},
};

type Result<T> = std::result::Result<T, Box<dyn std::error::Error + Send + Sync>>;

enum Target {
    Node(Option<u64>),
    Edit(u64),
    Session(String),
}

struct Picker {
    title: &'static str,
    entries: Vec<(Target, String)>,
    selected: usize,
}

const SESSIONS: &str = "resume session";

fn tree(entries: &[TreeEntry]) -> Vec<(Target, String)> {
    const DEPTH: usize = 16;
    let mut siblings = HashMap::<Option<u64>, usize>::new();
    for entry in entries {
        *siblings.entry(entry.parent).or_default() += 1;
    }
    let mut seen = HashMap::<Option<u64>, usize>::new();
    let mut prefixes = HashMap::<Option<u64>, (String, usize)>::new();
    let current = entries.iter().any(|entry| entry.current);
    let mut lines = vec![(Target::Node(None), format!("{}root", if current { "  " } else { "● " }))];
    for entry in entries {
        let (prefix, depth) = prefixes.get(&entry.parent).cloned().unwrap_or_default();
        let count = siblings[&entry.parent];
        let position = seen.entry(entry.parent).or_default();
        *position += 1;
        let (line, next) = if count > 1 {
            let last = *position == count;
            let line = format!("{prefix}{}", if last { "└─ " } else { "├─ " });
            let next = if depth < DEPTH {
                (format!("{prefix}{}", if last { "     " } else { "│    " }), depth + 1)
            } else {
                (prefix, depth)
            };
            (line, next)
        } else {
            (prefix.clone(), (prefix, depth))
        };
        prefixes.insert(Some(entry.node), next);
        let label = match entry.kind {
            Some(kind) => format!("{kind:?}: {}", entry.text),
            None => "step".into(),
        };
        let target = match entry.kind {
            Some(Kind::User) => Target::Edit(entry.node),
            _ => Target::Node(Some(entry.node)),
        };
        lines.push((target, format!("{}{line}{label}", if entry.current { "● " } else { "  " })));
    }
    lines
}

// How soon a second Esc must follow to stop the session's subagents too.
const ESCAPE_AGAIN: Duration = Duration::from_secs(1);

struct Shell {
    seq: u64,
    session: Option<String>,
    block: Block,
}

fn cycle_selection(selected: usize, len: usize, up: bool) -> usize {
    if len == 0 {
        0
    } else if up {
        selected.checked_sub(1).unwrap_or(len - 1)
    } else {
        (selected + 1) % len
    }
}

// The @mention the host is completing: each edit asks again, and only the
// latest answer counts.
#[derive(Default)]
struct Mentioning {
    asked: Option<(String, usize)>,
    open: bool,
    seq: u64,
    done: u64,
}

impl Mentioning {
    fn close(&mut self) {
        self.asked = None;
        self.open = false;
        self.seq += 1;
        self.done = self.seq;
    }

    fn pending(&self) -> bool {
        self.done < self.seq
    }
}

enum Models {
    Unasked,
    Asked,
    Listed(Vec<ModelInfo>),
}

// A conversation not started yet: the host only makes one with its first message.
struct Draft {
    model: Option<Model>,
}

struct App {
    remote: Remote,
    connected: bool,
    view: View,
    draft: Option<Draft>,
    host: HostView,
    editor: ui::Editor,
    feedback: Vec<Block>,
    completion: Option<input::Menu>,
    dismissed: bool,
    mention: Mentioning,
    models: Models,
    picker: Option<Picker>,
    shell: Option<Shell>,
    shell_seq: u64,
    expanded: bool,
    scroll: usize,
    scroll_layout: Option<ui::ScrollAnchor>,
    transcript_static: Option<ui::CachedLines>,
    images: image::Images,
    quitting: bool,
    // When an Esc stopped a session that has subagents.
    escaped: Option<Instant>,
}

impl App {
    fn model(&self) -> Option<&Model> {
        match &self.draft {
            Some(draft) => draft.model.as_ref(),
            None => Some(&self.view.state.model),
        }
    }

    fn session(&self) -> Result<&str> {
        match self.draft {
            Some(_) => Err("Nothing sent yet".into()),
            None => Ok(&self.view.id),
        }
    }

    // Back to the newest lines, where the transcript follows what arrives.
    fn follow(&mut self) {
        self.scroll = 0;
        self.scroll_layout = None;
    }

    fn reset_view(&mut self) {
        self.feedback.clear();
        self.follow();
        self.transcript_static = None;
        self.mention.close();
    }

    fn notice(&mut self, text: impl Into<String>) {
        self.push_feedback(Block::new(Kind::Notice, text));
    }

    fn warn(&mut self, text: impl Into<String>) {
        self.push_feedback(Block::new(Kind::Warning, text));
    }

    fn push_feedback(&mut self, block: Block) {
        self.feedback.push(block);
        if self.feedback.len() > 20 {
            self.feedback.remove(0);
        }
    }

    fn apply(&mut self, event: Event) {
        let change = self.view.apply(event);
        match change {
            Change::Reset { .. } => {
                self.connected = true;
                self.follow();
                self.transcript_static = None;
            }
            Change::State { started } => {
                if self.view.running() {
                    if started {
                        self.feedback.clear();
                    }
                    self.follow();
                }
            }
            Change::Committed { .. } | Change::Live(_) => (),
            Change::Feedback(block) => self.push_feedback(block),
        }
    }

    fn read(&mut self) {
        if self.draft.is_none() && self.view.has_unread() {
            self.remote.mark_read(&self.view.id);
        }
    }

    fn handle(&mut self, incoming: Incoming) {
        match incoming {
            Incoming::Event(generation, event) if generation == self.remote.generation => self.apply(event),
            Incoming::Lost(generation) if generation == self.remote.generation && self.connected => {
                self.connected = false;
                self.warn("Lost the mu host; reconnecting…");
            }
            Incoming::Host(event) => {
                self.host.apply(event);
                if let Some(picker) = self.picker.as_mut().filter(|picker| picker.title == SESSIONS) {
                    let selected = match picker.entries.get(picker.selected) {
                        Some((Target::Session(id), _)) => Some(id.clone()),
                        _ => None,
                    };
                    picker.entries = session_entries(&self.host);
                    picker.selected = picker
                        .entries
                        .iter()
                        .position(|(target, _)| matches!(target, Target::Session(id) if Some(id) == selected.as_ref()))
                        .unwrap_or(0);
                }
            }
            Incoming::Mention(seq, result) => {
                self.mention.done = self.mention.done.max(seq);
                if seq != self.mention.seq {
                    return;
                }
                match result {
                    Ok(Some(mention)) => {
                        let range = mention.start as usize..mention.end as usize;
                        let mut menu = input::Menu {
                            file: true,
                            exact_file: mention.exact_file,
                            ..input::Menu::new(range, mention.query, mention.entries)
                        };
                        menu.inherit(self.completion.as_ref());
                        self.completion = Some(menu);
                    }
                    Ok(None) => {
                        self.completion = None;
                        self.mention.open = false;
                    }
                    Err(e) => {
                        self.completion = None;
                        self.warn(format!("File search: {e}"));
                    }
                }
            }
            Incoming::Shell(seq, result) => self.shell_update(seq, result),
            Incoming::Models(result) => {
                match result {
                    Ok(models) => self.models = Models::Listed(models),
                    Err(e) => self.warn(format!("Listing models: {e}")),
                }
                self.refresh_completion();
            }
            Incoming::Image(path, result) => {
                self.images.loaded(path, result);
                self.transcript_static = None;
            }
            _ => (),
        }
    }

    fn shell_update(&mut self, seq: u64, result: Result<ShellEvent>) {
        let Some(shell) = self.shell.as_mut().filter(|shell| shell.seq == seq) else {
            return;
        };
        let tool = shell.block.tool.as_mut().expect("a shell block has its tool");
        let (run, error) = match result {
            Ok(ShellEvent::Output(output)) => {
                tool.output = output;
                return;
            }
            Ok(ShellEvent::Done { run, status, output }) => {
                tool.status = status;
                tool.output = output;
                (Some(run), None)
            }
            Err(e) => {
                tool.status = ToolStatus::Error;
                (None, Some(e.to_string()))
            }
        };
        let shell = self.shell.take().expect("matched");
        let error = match (run, &shell.session) {
            (Some(run), Some(session)) => match self.remote.call(AppendShell { session_id: session.clone(), run }) {
                Ok(()) => return,
                Err(e) => Some(format!("Not added to the conversation: {e}")),
            },
            _ => error,
        };
        self.push_feedback(shell.block);
        if let Some(error) = error {
            self.warn(error);
        }
    }

    fn run_shell(&mut self, command: &str, append: bool) -> Result<()> {
        if command.is_empty() {
            return Err("!command runs a command and adds it to the conversation; !!command only runs it".into());
        }
        if self.shell.is_some() {
            return Err("A command is still running; Ctrl+C stops it".into());
        }
        if append && self.view.blocks.is_empty() {
            return Err("The conversation is empty: send a message first, or use !! to only run it".into());
        }
        if append && self.view.running() {
            return Err("The agent is running: wait or stop it, or use !! to only run it".into());
        }
        self.shell_seq += 1;
        self.remote.shell(self.shell_seq, &self.view.cwd, command.into())?;
        let mut block = Block::new(Kind::Shell, command);
        block.tool = Some(Tool { call_id: String::new(), status: ToolStatus::Running, output: ToolOutput::default() });
        self.shell = Some(Shell { seq: self.shell_seq, session: append.then(|| self.view.id.clone()), block });
        Ok(())
    }

    fn cancel_shell(&mut self) {
        self.remote.cancel_shell();
        if let Some(mut shell) = self.shell.take() {
            if let Some(tool) = &mut shell.block.tool {
                tool.status = ToolStatus::Cancelled;
            }
            self.push_feedback(shell.block);
        }
    }

    fn attach(&mut self, id: &str) -> Result<()> {
        self.remote.attach(id)?;
        self.view = View::default();
        self.draft = None;
        self.reset_view();
        Ok(())
    }

    fn start_draft(&mut self, cwd: PathBuf, model: Option<Model>) {
        self.remote.detach();
        let skills = self.remote.call(ListSkills { cwd: cwd.clone() });
        self.view = View { cwd, skills: skills.as_ref().cloned().unwrap_or_default(), ..View::default() };
        self.draft = Some(Draft { model });
        self.connected = true;
        self.reset_view();
        if let Err(e) = skills {
            self.warn(format!("Skills: {e}"));
        }
    }

    fn quit(&mut self) {
        self.quitting = true;
        if let Ok(id) = self.session() {
            let _ = self.remote.call(Stop { session_id: id.into(), children: false });
        }
    }

    fn stop(&mut self) {
        if self.view.running()
            && let Err(e) = self.remote.call(Stop { session_id: self.view.id.clone(), children: false })
        {
            self.warn(e.to_string());
        }
    }

    fn escape(&mut self) {
        if self.draft.is_some() {
            return;
        }
        let id = self.view.id.clone();
        if self.escaped.take().is_some_and(|at| at.elapsed() < ESCAPE_AGAIN) {
            match self.remote.call(Stop { session_id: id, children: true }) {
                Ok(()) => self.notice("Stopped its subagents too."),
                Err(e) => self.warn(e.to_string()),
            }
            return;
        }
        if !self.view.running() {
            return;
        }
        self.stop();
        if self.host.sessions.iter().any(|session| session.parent.as_ref() == Some(&id)) {
            self.escaped = Some(Instant::now());
            self.notice("Esc again to stop its subagents too.");
        }
    }

    fn recall_queued(&mut self) {
        if !self.editor.chars.is_empty() {
            self.notice("Clear the input before restoring a queued message.");
            return;
        }
        let newest = match self.draft {
            Some(_) => None,
            None => self.view.state.queued.last().cloned(),
        };
        let Some(text) = newest else {
            self.notice("No queued messages.");
            return;
        };
        let recall = RecallQueued {
            session_id: self.view.id.clone(),
            index: self.view.state.queued.len() as u64 - 1,
            text: text.clone(),
        };
        match self.remote.call(recall) {
            Ok(true) => {
                self.editor.insert(&text);
                self.notice("Restored newest queued message to input.");
            }
            Ok(false) => self.notice("The newest queued message was already sent."),
            Err(e) => self.warn(e.to_string()),
        }
    }

    fn command(&mut self, command: &commands::Builtin, text: &str) -> Result<()> {
        let args: Vec<_> = text.split_whitespace().collect();
        match command.kind {
            commands::BuiltinKind::Quit => self.quit(),
            commands::BuiltinKind::Copy => {
                let kind = match args.get(1).copied() {
                    None | Some("agent") => Kind::Agent,
                    Some("user") => Kind::User,
                    _ => return Err(format!("Usage: {} {}", command.name, command.hint).into()),
                };
                let text = self.view.last_text(kind).ok_or("Nothing to copy")?;
                copy(&text)?;
                self.notice("Copied.");
            }
            commands::BuiltinKind::Model => {
                if !(2..=4).contains(&args.len()) {
                    return Err(format!("Usage: {} {}", command.name, command.hint).into());
                }
                let context = match args.get(3) {
                    Some(context) => Some(
                        counts::parse(context).ok_or("Context must be a positive token count (e.g. 128k or 1.5m)")?,
                    ),
                    None => self.model().filter(|model| model.name == args[1]).and_then(|model| model.context),
                };
                let model = Model {
                    name: args[1].into(),
                    effort: args.get(2).copied().filter(|&s| s != "-").map(Into::into),
                    context,
                };
                match &mut self.draft {
                    Some(draft) => draft.model = Some(model),
                    None => self.remote.call(SetModel { session_id: self.view.id.clone(), model })?,
                }
            }
            commands::BuiltinKind::New => self.start_draft(self.view.cwd.clone(), self.model().cloned()),
            commands::BuiltinKind::Tree => {
                let entries = self.remote.call(GetTree { session_id: self.session()?.into() })?;
                let selected = entries.iter().position(|entry| entry.current).map_or(0, |i| i + 1);
                self.picker = Some(Picker { title: "conversation tree", entries: tree(&entries), selected });
            }
            commands::BuiltinKind::Resume => {
                let entries = session_entries(&self.host);
                if entries.is_empty() {
                    return Err("No saved sessions".into());
                }
                self.picker = Some(Picker { title: SESSIONS, entries, selected: 0 });
            }
        }
        Ok(())
    }

    fn submit(&mut self) {
        if self.editor.text().trim().is_empty() {
            return;
        }
        if let Err(e) = self.send_input() {
            self.warn(e.to_string());
        }
        self.follow();
    }

    fn send_input(&mut self) -> Result<()> {
        let mut text = self.editor.text();
        if let Some((append, command)) = input::shell_command(&text) {
            self.run_shell(command, append)?;
            self.editor.clear();
            return Ok(());
        }
        let mut skill = None;
        if text.trim_start().starts_with('/') {
            let word = text.split_whitespace().next().unwrap();
            let (name, choice) = commands::resolve(word, &self.view.skills)?;
            text = format!("{}{}", name, &text.trim_start()[word.len()..]);
            match choice {
                commands::Choice::Builtin(command) => {
                    let result = self.command(command, &text);
                    self.editor.clear();
                    return result;
                }
                commands::Choice::Skill(i) => skill = Some(self.view.skills[i].name.clone()),
            }
        }
        match &self.draft {
            Some(draft) => {
                let model = draft.model.clone().ok_or("Pick a model first: /model <model> [effort|-] [context]")?;
                let id =
                    self.remote.call(CreateSession { cwd: self.view.cwd.clone(), model, text, skill, parent: None })?;
                self.editor.clear();
                self.attach(&id)
            }
            None => {
                self.remote.call(SendMessage { session_id: self.view.id.clone(), text, skill, from: None })?;
                self.editor.clear();
                Ok(())
            }
        }
    }

    fn complete(&mut self) {
        if let Some(menu) = &self.completion
            && let Some(text) = menu.replacement()
        {
            self.editor.replace(menu.range.clone(), &text);
        }
    }

    fn refresh_completion(&mut self) {
        let text = self.editor.text();
        let cursor = self.editor.cursor;
        let shell = input::shell_command(&text).is_some();
        let menu = if self.dismissed || self.picker.is_some() {
            None
        } else {
            input::command_menu(&text, cursor, &self.view.skills)
                .or_else(|| self.model_menu(&text, cursor))
                .or_else(|| input::shell_menu(&text, cursor, &self.host.profile.snippets))
        };
        if let Some(mut menu) = menu {
            menu.inherit(self.completion.as_ref());
            self.completion = Some(menu);
            self.mention.close();
            return;
        }
        if self.dismissed || self.picker.is_some() || shell || !text.contains('@') {
            self.completion = None;
            self.mention.close();
            return;
        }
        if self.mention.asked.as_ref().is_some_and(|(t, c)| *t == text && *c == cursor) {
            return;
        }
        self.mention.seq += 1;
        let call = CompleteMention {
            cwd: self.view.cwd.clone(),
            text: text.clone(),
            cursor: cursor as u64,
            refresh: !self.mention.open,
        };
        self.remote.complete(self.mention.seq, call);
        self.mention.open = true;
        self.mention.asked = Some((text, cursor));
        // A nearby host answers before the next frame, so the menu never lags the
        // input. Only a slow answer (a first index of a large tree) arrives later.
        let deadline = Instant::now() + Duration::from_millis(50);
        while self.mention.pending() {
            match self.remote.rx.recv_timeout(deadline.saturating_duration_since(Instant::now())) {
                Ok(incoming) => self.handle(incoming),
                Err(_) => {
                    self.completion = None;
                    break;
                }
            }
        }
    }

    fn model_menu(&mut self, text: &str, cursor: usize) -> Option<input::Menu> {
        let argument = input::model_argument(text, cursor, &self.view.skills)?;
        if let Models::Unasked = self.models {
            self.remote.list_models();
            self.models = Models::Asked;
        }
        let models = match &self.models {
            Models::Listed(models) => models.as_slice(),
            _ => &[],
        };
        input::model_menu(argument, models)
    }

    fn select(&mut self) -> Result<()> {
        let Some(picker) = self.picker.take() else {
            return Ok(());
        };
        if let Some((target, _)) = picker.entries.into_iter().nth(picker.selected) {
            match target {
                Target::Node(node) => {
                    self.remote.call(Checkout { session_id: self.view.id.clone(), node })?;
                    self.reset_view();
                }
                Target::Edit(node) => {
                    let text = self.remote.call(Edit { session_id: self.view.id.clone(), node })?;
                    self.editor.clear();
                    self.editor.insert(&text);
                    self.reset_view();
                }
                Target::Session(id) => {
                    if id != self.view.id {
                        self.attach(&id)?;
                    } else {
                        self.reset_view();
                    }
                }
            }
        }
        Ok(())
    }

    fn scroll_up(&mut self, amount: usize) {
        if self.scroll == 0
            && self.picker.is_none()
            && let Ok((width, _)) = crossterm::terminal::size()
        {
            let width = width as usize;
            let total = ui::transcript_line_count(self, width);
            self.scroll_layout = Some(ui::ScrollAnchor { width, expanded: self.expanded, total });
        }
        self.scroll = self.scroll.saturating_add(amount);
    }

    fn scroll_down(&mut self, amount: usize) {
        self.scroll = self.scroll.saturating_sub(amount);
        if self.scroll == 0 {
            self.scroll_layout = None;
        }
    }

    fn key(&mut self, key: KeyEvent) {
        if key.kind == KeyEventKind::Release {
            return;
        }
        let ctrl = key.modifiers.contains(Mod::CONTROL);
        if ctrl && key.code == Key::Char('o') {
            self.expanded = !self.expanded;
            self.scroll_layout = None;
            return;
        }
        if let Some(picker) = &mut self.picker {
            match key.code {
                Key::Esc => self.picker = None,
                Key::Up | Key::Down | Key::Char('k' | 'j') => {
                    picker.selected = cycle_selection(
                        picker.selected,
                        picker.entries.len(),
                        matches!(key.code, Key::Up | Key::Char('k')),
                    );
                }
                Key::Home => picker.selected = 0,
                Key::End => picker.selected = picker.entries.len().saturating_sub(1),
                Key::Enter => {
                    if let Err(e) = self.select() {
                        self.warn(e.to_string());
                    }
                }
                _ => (),
            }
            return;
        }
        if ctrl && key.code == Key::Up {
            self.recall_queued();
            return;
        }
        if let Some(menu) = &mut self.completion {
            match key.code {
                Key::Up | Key::Down if !menu.entries.is_empty() => {
                    menu.selected = cycle_selection(menu.selected, menu.entries.len(), key.code == Key::Up);
                    menu.explicit = true;
                    return;
                }
                Key::Esc => {
                    self.completion = None;
                    self.dismissed = true;
                    return;
                }
                Key::Tab => {
                    self.complete();
                    return;
                }
                Key::Enter if !key.modifiers.contains(Mod::SHIFT) => {
                    if menu.file && (!menu.exact_file || menu.explicit) && !menu.entries.is_empty() {
                        self.complete();
                        return;
                    }
                    if !menu.file && menu.explicit {
                        self.complete();
                    }
                }
                _ => (),
            }
        }
        match key.code {
            Key::Char('c') if ctrl => {
                if self.shell.is_some() {
                    self.cancel_shell();
                } else if self.view.running() {
                    self.stop();
                } else if !self.editor.chars.is_empty() {
                    self.editor.clear();
                } else {
                    self.quit();
                }
            }
            Key::Char('d') if ctrl && self.editor.chars.is_empty() => self.quit(),
            Key::Esc if self.shell.is_some() => self.cancel_shell(),
            Key::Esc => self.escape(),
            Key::PageUp => self.scroll_up(10),
            Key::PageDown => self.scroll_down(10),
            Key::Enter if key.modifiers.contains(Mod::SHIFT) => self.editor.insert("\n"),
            Key::Char('j') if ctrl => self.editor.insert("\n"),
            Key::Enter => self.submit(),
            Key::Left => self.editor.left(),
            Key::Right => self.editor.right(),
            Key::Up | Key::Down => {
                let width = crossterm::terminal::size().map(|(w, _)| w.saturating_sub(3) as usize).unwrap_or(1);
                self.editor.move_vertical(width, key.code == Key::Up);
            }
            Key::Home => self.editor.home(),
            Key::End => self.editor.end(),
            Key::Char('a') if ctrl => self.editor.home(),
            Key::Char('e') if ctrl => self.editor.end(),
            Key::Char('u') if ctrl => self.editor.clear_line(),
            Key::Char('w') if ctrl => self.editor.word_backspace(),
            Key::Backspace => self.editor.backspace(),
            Key::Delete => self.editor.delete(),
            Key::Char(c) if !ctrl && !key.modifiers.contains(Mod::ALT) => self.editor.insert(&c.to_string()),
            _ => (),
        }
    }

    fn edit_prompt(&mut self, terminal: &mut ui::Terminal) -> Result<()> {
        match terminal.suspend(|| input::edit(&self.editor.text(), &self.view.cwd))? {
            Ok(text) => {
                self.editor.clear();
                self.editor.insert(&text);
            }
            Err(e) => self.warn(format!("Prompt unchanged: {e}")),
        }
        Ok(())
    }

    fn run(&mut self) -> Result<()> {
        let mut terminal = ui::Terminal::enter()?;
        let mut title = String::new();
        let mut dirty = true;
        loop {
            while let Ok(incoming) = self.remote.rx.try_recv() {
                self.handle(incoming);
                dirty = true;
            }
            if self.quitting {
                break;
            }
            if dirty {
                let next = ui::title(self);
                if next != title {
                    ui::set_title(&next)?;
                    title = next;
                }
                ui::draw(self)?;
                dirty = false;
            }
            let busy = self.view.running() || self.shell.is_some() || self.mention.pending();
            if event::poll(Duration::from_millis(if busy { 33 } else { 250 }))? {
                let before = (self.editor.text(), self.editor.cursor);
                match event::read()? {
                    Input::Key(k)
                        if k.kind != KeyEventKind::Release
                            && k.modifiers.contains(Mod::CONTROL)
                            && k.code == Key::Char('g')
                            && self.picker.is_none() =>
                    {
                        self.read();
                        self.edit_prompt(&mut terminal)?;
                        self.images.forget();
                        title.clear();
                    }
                    Input::Key(k) => {
                        self.read();
                        self.key(k);
                    }
                    Input::Paste(s) if self.picker.is_none() => {
                        self.read();
                        self.editor.insert(&s);
                    }
                    Input::Mouse(m) => match m.kind {
                        MouseEventKind::ScrollUp => {
                            self.read();
                            self.scroll_up(3);
                        }
                        MouseEventKind::ScrollDown => {
                            self.read();
                            self.scroll_down(3);
                        }
                        _ => (),
                    },
                    _ => (),
                }
                if before != (self.editor.text(), self.editor.cursor) {
                    self.dismissed = false;
                }
                self.refresh_completion();
                dirty = true;
            }
        }
        Ok(())
    }
}

fn copy(text: &str) -> Result<()> {
    use base64::{Engine, engine::general_purpose::STANDARD};
    print!("\x1b]52;c;{}\x07", STANDARD.encode(text));
    io::stdout().flush()?;
    Ok(())
}

// Each session, then the ones it spawned beneath it.
fn session_entries(host: &HostView) -> Vec<(Target, String)> {
    let sessions = &host.sessions;
    let mut stack: Vec<(&SessionSummary, usize)> = sessions
        .iter()
        .rev()
        .filter(|s| s.parent.as_ref().is_none_or(|parent| sessions.iter().all(|other| other.id != *parent)))
        .map(|session| (session, 0))
        .collect();
    let mut entries = vec![];
    while let Some((session, depth)) = stack.pop() {
        let label = format!(
            "{}{}{}{}  {}  [{}]",
            if depth == 0 { String::new() } else { format!("{}└ ", "  ".repeat(depth - 1)) },
            ui::marker(session.status),
            if session.unread { "• " } else { "" },
            session.id,
            session.title,
            session.cwd.display()
        );
        entries.push((Target::Session(session.id.clone()), label));
        let spawned = sessions.iter().rev().filter(|s| s.parent.as_ref() == Some(&session.id));
        stack.extend(spawned.map(|child| (child, depth + 1)));
    }
    entries
}

fn var(name: &str) -> Option<String> {
    env::var(name).ok().filter(|value| !value.is_empty())
}

pub fn run(endpoint: Endpoint) -> Result<()> {
    let remote = Remote::connect(endpoint)?;
    let model = var("MU_MODEL").map(|name| Model {
        name,
        effort: var("MU_EFFORT"),
        context: var("MU_CONTEXT").and_then(|s| counts::parse(&s)),
    });
    let mut app = App {
        remote,
        connected: true,
        view: View::default(),
        draft: None,
        host: HostView::default(),
        editor: ui::Editor::default(),
        feedback: vec![],
        completion: None,
        dismissed: false,
        mention: Mentioning::default(),
        models: Models::Unasked,
        picker: None,
        shell: None,
        shell_seq: 0,
        expanded: false,
        scroll: 0,
        scroll_layout: None,
        transcript_static: None,
        images: image::Images::new(),
        quitting: false,
        escaped: None,
    };
    app.start_draft(env::current_dir()?, model);
    app.run()
}
