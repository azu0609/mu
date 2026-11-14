mod commands;
mod counts;
mod devices;
mod input;
mod markdown;
mod remote;
mod text;
mod ui;

use crossterm::event::{
    self, Event as Input, KeyCode as Key, KeyEvent, KeyEventKind, KeyModifiers as Mod, MouseEventKind,
};
pub use devices::{devices, pair, unpair};
pub use mu_client::Endpoint;
use mu_client::{Block, Change, Event, Kind, Model, SessionList, View};
use remote::{Incoming, Remote};
use std::{
    env,
    io::{self, Write},
    time::{Duration, Instant},
};

type Result<T> = std::result::Result<T, Box<dyn std::error::Error + Send + Sync>>;

enum Target {
    Node(Option<usize>),
    Session(String),
}

struct Picker {
    title: &'static str,
    entries: Vec<(Target, String)>,
    selected: usize,
}

// Kept current while it is open.
const SESSIONS: &str = "resume session";

fn cycle_selection(selected: usize, len: usize, up: bool) -> usize {
    if len == 0 {
        0
    } else if up {
        selected.checked_sub(1).unwrap_or(len - 1)
    } else {
        (selected + 1) % len
    }
}

struct App {
    remote: Remote,
    connected: bool,
    view: View,
    sessions: SessionList,
    editor: ui::Editor,
    feedback: Vec<Block>,
    completion: Option<input::Menu>,
    dismissed: bool,
    // The input the host was last asked to complete; answers to older requests are ignored.
    mention_key: Option<(String, usize)>,
    mention_open: bool,
    mention_seq: u64,
    mention_done: u64,
    picker: Option<Picker>,
    expanded: bool,
    scroll: usize,
    scroll_layout: Option<ui::ScrollAnchor>,
    transcript_static: Option<ui::CachedLines>,
    quitting: bool,
}

impl App {
    fn reset_view(&mut self) {
        self.feedback.clear();
        self.scroll = 0;
        self.scroll_layout = None;
        self.transcript_static = None;
        self.close_mention();
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
        // On screen and not running: whatever the agent did has been seen.
        if matches!(change, Change::Reset { .. } | Change::State { .. }) && !self.view.running() {
            self.remote.mark_read(&self.view.id);
        }
        match change {
            Change::Reset { .. } => {
                self.connected = true;
                self.scroll = 0;
                self.scroll_layout = None;
                self.transcript_static = None;
            }
            Change::State { started } => {
                if self.view.running() {
                    if started {
                        self.feedback.clear();
                    }
                    self.scroll = 0;
                    self.scroll_layout = None;
                }
            }
            Change::Committed { .. } | Change::Live(_) | Change::LiveNotice => (),
            Change::Feedback(block) => self.push_feedback(block),
        }
    }

    fn handle(&mut self, incoming: Incoming) {
        match incoming {
            Incoming::Event(generation, event) if generation == self.remote.generation => self.apply(event),
            Incoming::Lost(generation) if generation == self.remote.generation && self.connected => {
                self.connected = false;
                self.warn("Lost the mu host; reconnecting…");
            }
            Incoming::Sessions(event) => {
                self.sessions.apply(event);
                if let Some(picker) = self.picker.as_mut().filter(|picker| picker.title == SESSIONS) {
                    let selected = match picker.entries.get(picker.selected) {
                        Some((Target::Session(id), _)) => Some(id.clone()),
                        _ => None,
                    };
                    picker.entries = session_entries(&self.sessions);
                    picker.selected = picker
                        .entries
                        .iter()
                        .position(|(target, _)| matches!(target, Target::Session(id) if Some(id) == selected.as_ref()))
                        .unwrap_or(0);
                }
            }
            Incoming::Mention(seq, result) => {
                self.mention_done = self.mention_done.max(seq);
                if seq != self.mention_seq {
                    return;
                }
                match result {
                    Ok(Some(mention)) => {
                        let mut menu = input::Menu {
                            range: mention.range,
                            query: mention.query,
                            file: true,
                            exact_file: mention.exact_file,
                            entries: mention
                                .entries
                                .into_iter()
                                .map(|entry| input::Entry { text: entry.text, label: entry.label })
                                .collect(),
                            selected: 0,
                            explicit: false,
                        };
                        menu.inherit(self.completion.as_ref());
                        self.completion = Some(menu);
                    }
                    Ok(None) => {
                        self.completion = None;
                        self.mention_open = false;
                    }
                    Err(e) => {
                        self.completion = None;
                        self.warn(format!("File search: {e}"));
                    }
                }
            }
            _ => (),
        }
    }

    fn attach(&mut self, id: &str) -> Result<()> {
        self.remote.attach(id)?;
        self.view = View::default();
        self.reset_view();
        Ok(())
    }

    // Leaving stops this session's agent, also on a shared host: nothing keeps working unseen.
    fn quit(&mut self) {
        self.quitting = true;
        // Asked regardless of what we last heard: a step may have started since.
        let _ = self.remote.stop(&self.view.id);
    }

    fn stop(&mut self) {
        if self.view.running()
            && let Err(e) = self.remote.stop(&self.view.id)
        {
            self.warn(e.to_string());
        }
    }

    fn recall_queued(&mut self) {
        if !self.editor.chars.is_empty() {
            self.notice("Clear the input before restoring a queued message.");
            return;
        }
        match self.remote.recall(&self.view.id) {
            Ok(Some(text)) => {
                self.editor.insert(&text);
                self.notice("Restored newest queued message to input.");
            }
            Ok(None) => self.notice("No queued messages."),
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
                    Some(context) => {
                        counts::parse(context).ok_or("Context must be a positive token count (e.g. 128k or 1.5m)")?
                    }
                    None => self.view.model.context,
                };
                let model = Model {
                    name: args[1].into(),
                    effort: args.get(2).copied().filter(|&s| s != "-").map(Into::into),
                    context,
                };
                self.remote.set_model(&self.view.id, model)?;
            }
            commands::BuiltinKind::New => {
                let model = &self.view.model;
                let id = self.remote.create(
                    &self.view.cwd,
                    Some(model.name.clone()),
                    Some(model.effort.clone().unwrap_or_default()),
                    Some(model.context),
                )?;
                self.attach(&id)?;
            }
            commands::BuiltinKind::Tree => {
                let entries: Vec<_> = self.remote.tree(&self.view.id)?;
                let selected = entries.iter().position(|entry| entry.current).unwrap_or(0);
                let entries = entries
                    .into_iter()
                    .map(|entry| {
                        (
                            Target::Node(entry.node),
                            format!("{}{}", if entry.current { "● " } else { "  " }, entry.label),
                        )
                    })
                    .collect();
                self.picker = Some(Picker { title: "conversation tree", entries, selected });
            }
            commands::BuiltinKind::Resume => {
                let entries = session_entries(&self.sessions);
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
        self.scroll = 0;
        self.scroll_layout = None;
    }

    fn send_input(&mut self) -> Result<()> {
        let mut text = self.editor.text();
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
        // The host reads the mentioned files and decides whether this starts a step or steers one.
        self.remote.send(&self.view.id, text, skill)?;
        self.editor.clear();
        Ok(())
    }

    fn complete(&mut self) {
        if let Some(menu) = &self.completion
            && let Some(text) = menu.replacement()
        {
            self.editor.replace(menu.range.clone(), &text);
        }
    }

    fn close_mention(&mut self) {
        self.mention_key = None;
        self.mention_open = false;
        self.mention_seq += 1;
        self.mention_done = self.mention_seq;
    }

    fn refresh_completion(&mut self) {
        let text = self.editor.text();
        let cursor = self.editor.cursor;
        let menu = if self.dismissed || self.picker.is_some() {
            None
        } else {
            input::command_menu(&text, cursor, &self.view.skills)
        };
        if let Some(mut menu) = menu {
            menu.inherit(self.completion.as_ref());
            self.completion = Some(menu);
            self.close_mention();
            return;
        }
        if self.dismissed || self.picker.is_some() || !text.contains('@') {
            self.completion = None;
            self.close_mention();
            return;
        }
        if self.mention_key.as_ref().is_some_and(|(t, c)| *t == text && *c == cursor) {
            return;
        }
        self.mention_seq += 1;
        // Opening a mention re-indexes the host's files; typing on within it reuses that index.
        self.remote.complete(self.mention_seq, &self.view.id, text.clone(), cursor, !self.mention_open);
        self.mention_open = true;
        self.mention_key = Some((text, cursor));
        // A nearby host answers before the next frame, so the menu never lags the
        // input. Only a slow answer (a first index of a large tree) arrives later.
        let deadline = Instant::now() + Duration::from_millis(50);
        while self.mention_done < self.mention_seq {
            match self.remote.rx.recv_timeout(deadline.saturating_duration_since(Instant::now())) {
                Ok(incoming) => self.handle(incoming),
                Err(_) => {
                    self.completion = None;
                    break;
                }
            }
        }
    }

    fn select(&mut self) -> Result<()> {
        let Some(picker) = self.picker.take() else {
            return Ok(());
        };
        if let Some((target, _)) = picker.entries.into_iter().nth(picker.selected) {
            match target {
                Target::Node(node) => {
                    // The host answers with a fresh snapshot on the session's stream.
                    let draft = self.remote.checkout(&self.view.id, node)?;
                    if let Some(text) = draft {
                        self.editor.clear();
                        self.editor.insert(&text);
                    }
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
                if self.view.running() {
                    self.stop();
                } else if !self.editor.chars.is_empty() {
                    self.editor.clear();
                } else {
                    self.quit();
                }
            }
            Key::Char('d') if ctrl && self.editor.chars.is_empty() => self.quit(),
            Key::Esc => self.stop(),
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
            // The host can speak at any time (another client may be driving this session).
            let busy = self.view.running() || self.mention_done < self.mention_seq;
            if event::poll(Duration::from_millis(if busy { 33 } else { 250 }))? {
                let before = (self.editor.text(), self.editor.cursor);
                match event::read()? {
                    Input::Key(k)
                        if k.kind != KeyEventKind::Release
                            && k.modifiers.contains(Mod::CONTROL)
                            && k.code == Key::Char('g')
                            && self.picker.is_none() =>
                    {
                        self.edit_prompt(&mut terminal)?;
                        title.clear();
                    }
                    Input::Key(k) => self.key(k),
                    Input::Paste(s) if self.picker.is_none() => self.editor.insert(&s),
                    Input::Mouse(m) => match m.kind {
                        MouseEventKind::ScrollUp => self.scroll_up(3),
                        MouseEventKind::ScrollDown => self.scroll_down(3),
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

fn session_entries(list: &SessionList) -> Vec<(Target, String)> {
    list.sessions
        .iter()
        .map(|session| {
            let label = format!(
                "{}{}{}  {}  [{}]",
                ui::marker(session.status),
                if session.unread { "• " } else { "" },
                session.id,
                session.title,
                session.cwd.display()
            );
            (Target::Session(session.id.clone()), label)
        })
        .collect()
}

fn var(name: &str) -> Option<String> {
    env::var(name).ok().filter(|value| !value.is_empty())
}

// Opens a new session in the current directory on the host at `endpoint`.
// Quitting stops the session's agent; the saved session remains.
pub fn run(endpoint: Endpoint) -> Result<()> {
    let remote = Remote::connect(endpoint)?;
    // What this terminal asks for wins over the defaults of a shared host.
    let id = remote.create(
        &env::current_dir()?,
        var("MU_MODEL"),
        var("MU_EFFORT"),
        var("MU_CONTEXT").and_then(|s| s.parse().ok()),
    )?;
    let mut app = App {
        remote,
        connected: true,
        view: View::default(),
        sessions: SessionList::default(),
        editor: ui::Editor::default(),
        feedback: vec![],
        completion: None,
        dismissed: false,
        mention_key: None,
        mention_open: false,
        mention_seq: 0,
        mention_done: 0,
        picker: None,
        expanded: false,
        scroll: 0,
        scroll_layout: None,
        transcript_static: None,
        quitting: false,
    };
    app.attach(&id)?;
    app.run()
}
