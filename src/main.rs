mod api;
mod commands;
mod counts;
mod input;
mod instructions;
mod process;
mod session;
mod skills;
mod tools;
mod ui;

use crossterm::event::{
    self, Event as Input, KeyCode as Key, KeyEvent, KeyEventKind, KeyModifiers as Mod, MouseEventKind,
};
use session::{Block, Kind, Model, Record, Session};
use std::{
    env,
    io::{self, Write},
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    thread,
    time::Duration,
};

type Result<T> = std::result::Result<T, Box<dyn std::error::Error + Send + Sync>>;

pub fn home() -> PathBuf {
    env::home_dir().unwrap_or_else(|| PathBuf::from("."))
}

struct Worker {
    cancel: Arc<AtomicBool>,
    handle: thread::JoinHandle<()>,
}

enum Target {
    Node(Option<usize>),
    Session(PathBuf),
}

struct Picker {
    title: &'static str,
    entries: Vec<(Target, String)>,
    selected: usize,
}

struct App {
    session: Session,
    editor: ui::Editor,
    live: Vec<Block>,
    live_notice: Option<Block>,
    feedback: Vec<Block>,
    queued: Vec<input::Message>,
    skills: Vec<skills::Skill>,
    completion: Option<input::Menu>,
    dismissed: bool,
    files: input::FileSearch,
    worker: Option<Worker>,
    tx: mpsc::Sender<api::Event>,
    rx: mpsc::Receiver<api::Event>,
    picker: Option<Picker>,
    expanded: bool,
    scroll: usize,
    scroll_layout: Option<ui::ScrollAnchor>,
    transcript_static: Option<ui::CachedLines>,
    quitting: bool,
    title_status: ui::TitleStatus,
}

fn cache_miss_text() -> &'static str {
    "Cache miss · previously cached prefix was not read"
}

impl App {
    fn reset_view(&mut self) {
        self.live.clear();
        self.live_notice = None;
        self.feedback.clear();
        self.queued.clear();
        self.scroll = 0;
        self.scroll_layout = None;
        self.transcript_static = None;
        self.title_status = ui::TitleStatus::Ready;
    }

    fn drain_queue(&mut self) {
        for message in self.queued.drain(..) {
            self.session.user(message.text, message.attachments);
        }
    }

    fn recall_queued(&mut self) {
        if !self.editor.chars.is_empty() {
            self.notice("Clear the input before restoring a queued message.");
            return;
        }
        if let Some(message) = self.queued.pop() {
            self.editor.insert(&message.text);
            self.notice("Restored newest queued message to input.");
        } else {
            self.notice("No queued messages.");
        }
    }

    fn notice(&mut self, text: impl Into<String>) {
        self.push_feedback(Kind::Notice, text);
    }

    fn warn(&mut self, text: impl Into<String>) {
        self.push_feedback(Kind::Warning, text);
    }

    fn push_feedback(&mut self, kind: Kind, text: impl Into<String>) {
        self.feedback.push(Block::new(kind, text));
        if self.feedback.len() > 20 {
            self.feedback.remove(0);
        }
    }

    fn save(&mut self) -> bool {
        match self.session.save() {
            Ok(()) => true,
            Err(e) => {
                self.warn(format!("Session not saved: {e}"));
                false
            }
        }
    }

    fn start(&mut self) {
        if self.worker.is_some() || self.session.cursor().is_none() {
            return;
        }
        if !self.save() {
            return;
        }
        self.live.clear();
        self.live_notice = None;
        self.scroll = 0;
        self.scroll_layout = None;
        let request = api::Request {
            instructions: self.session.header().instructions.clone(),
            model: self.session.model().name.clone(),
            effort: self.session.model().effort.clone(),
            input: self.session.input(),
            cwd: self.session.header().cwd.clone(),
            previous_usage: self.session.usage(),
        };
        let cancel = Arc::new(AtomicBool::new(false));
        let flag = cancel.clone();
        let tx = self.tx.clone();
        let handle = thread::spawn(move || {
            let result = api::step(request, flag, &tx);
            // All streaming sends finish before Done; commit the finalized items.
            let _ = tx.send(api::Event::Done(result));
        });
        self.worker = Some(Worker { cancel, handle });
        self.title_status = ui::TitleStatus::Ready;
    }

    fn stop(&mut self) {
        if let Some(w) = &self.worker {
            w.cancel.store(true, Ordering::Relaxed);
        }
    }

    fn finish(&mut self, result: Result<api::Step>) {
        let w = self.worker.take().unwrap();
        let cancelled = w.cancel.load(Ordering::Relaxed);
        let _ = w.handle.join();
        match result {
            Ok(mut step) => {
                // Keep the marker with this response, ahead of its output, not in transient UI feedback.
                if step.usage.cache_miss(self.session.usage()) {
                    step.entries.insert(0, Record::Notice { text: cache_miss_text().into() });
                }
                self.live.clear();
                self.live_notice = None;
                self.session.push(step.entries, Some(step.usage));
                let mut again = step.again;
                if !cancelled && !self.quitting {
                    again |= !self.queued.is_empty();
                    self.drain_queue();
                }
                if cancelled {
                    self.warn("Stopped. Tool side effects are not undone. Enter to continue.");
                }
                if again && !cancelled && !self.quitting {
                    self.start();
                } else {
                    self.save();
                }
                if self.worker.is_none() {
                    self.title_status = if cancelled {
                        ui::TitleStatus::Ready
                    } else if again && !self.quitting {
                        // The next step could not start (for example, the session failed to save).
                        ui::TitleStatus::Failed
                    } else {
                        ui::TitleStatus::Finished
                    };
                }
            }
            Err(e) => {
                self.title_status = if cancelled { ui::TitleStatus::Ready } else { ui::TitleStatus::Failed };
                self.live_notice = None;
                self.warn(e.to_string());
                if !self.queued.is_empty() {
                    self.notice("Steering still queued. Send a message to retry, /new to discard.");
                }
            }
        }
    }

    fn command(&mut self, command: &commands::Builtin, text: &str) -> Result<()> {
        let args: Vec<_> = text.split_whitespace().collect();
        if !matches!(command.kind, commands::BuiltinKind::Quit | commands::BuiltinKind::Copy) && self.worker.is_some() {
            return Err("Stop the agent with Esc before changing sessions or models".into());
        }
        match command.kind {
            commands::BuiltinKind::Quit => {
                self.quitting = true;
                self.stop();
            }
            commands::BuiltinKind::Copy => {
                let kind = match args.get(1).copied() {
                    None | Some("agent") => Kind::Agent,
                    Some("user") => Kind::User,
                    _ => return Err(format!("Usage: {} {}", command.name, command.hint).into()),
                };
                let text = self.session.last_text(kind).ok_or("Nothing to copy")?;
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
                    None => self.session.model().context,
                };
                let model = Model {
                    name: args[1].into(),
                    effort: args.get(2).copied().filter(|&s| s != "-").map(Into::into),
                    context,
                };
                if &model != self.session.model() {
                    self.session.record(Record::Model(model))?;
                }
                self.save();
            }
            commands::BuiltinKind::New => {
                self.skills = skills::skills(&self.session.header().cwd);
                let session =
                    Session::new(self.session.header().cwd.clone(), self.session.model().clone(), &self.skills);
                self.session = session;
                self.files = input::FileSearch::default();
                self.reset_view();
            }
            commands::BuiltinKind::Tree => {
                let entries: Vec<_> = self
                    .session
                    .tree()
                    .into_iter()
                    .map(|(i, label)| {
                        (Target::Node(i), format!("{}{label}", if i == self.session.cursor() { "● " } else { "  " }))
                    })
                    .collect();
                let selected = entries
                    .iter()
                    .position(|(t, _)| matches!(t, Target::Node(i) if *i == self.session.cursor()))
                    .unwrap_or(0);
                self.picker = Some(Picker { title: "conversation tree", entries, selected });
            }
            commands::BuiltinKind::Resume => {
                let entries: Vec<_> =
                    session::sessions()?.into_iter().map(|(p, label)| (Target::Session(p), label)).collect();
                if entries.is_empty() {
                    return Err("No saved sessions".into());
                }
                self.picker = Some(Picker { title: "resume session", entries, selected: 0 });
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
            let (name, choice) = commands::resolve(word, &self.skills)?;
            text = format!("{}{}", name, &text.trim_start()[word.len()..]);
            match choice {
                commands::Choice::Builtin(command) => {
                    let result = self.command(command, &text);
                    self.editor.clear();
                    return result;
                }
                commands::Choice::Skill(i) => skill = Some(&self.skills[i]),
            }
        }
        let message = input::prepare(text, &self.session.header().cwd, skill)?;
        self.editor.clear();
        if !message.text.trim().is_empty() {
            self.queued.push(message);
        }
        if self.worker.is_none() {
            self.feedback.clear();
            self.drain_queue();
            self.start();
        }
        Ok(())
    }

    fn complete(&mut self) {
        if let Some(menu) = &self.completion
            && let Some(text) = menu.replacement()
        {
            self.editor.replace(menu.range.clone(), &text);
        }
    }

    fn refresh_completion(&mut self) {
        if self.dismissed || self.picker.is_some() {
            self.completion = None;
            self.files = input::FileSearch::default();
            return;
        }
        let mut menu = input::menu(
            &self.editor.text(),
            self.editor.cursor,
            &self.session.header().cwd,
            &self.skills,
            &self.files.files,
        );
        if let Some(new) = &mut menu {
            if let Some(old) = &self.completion
                && new.range == old.range
                && new.query == old.query
                && new.file == old.file
            {
                new.selected = old.selected.min(new.entries.len().saturating_sub(1));
                new.explicit = old.explicit;
            }
            if new.file && !["/", "~/", "./", "../"].iter().any(|p| new.query.starts_with(p)) {
                self.files.start(&self.session.header().cwd);
            } else {
                self.files = input::FileSearch::default();
            }
        } else {
            self.files = input::FileSearch::default();
        }
        self.completion = menu;
    }

    fn select(&mut self) -> Result<()> {
        let Some(picker) = self.picker.take() else {
            return Ok(());
        };
        if let Some((target, _)) = picker.entries.into_iter().nth(picker.selected) {
            let save_cursor = match target {
                Target::Node(i) => {
                    let mut cursor = i;
                    let mut draft = None;
                    if let Some(node) = i.map(|i| self.session.node(i))
                        && let Some(user) = node.blocks.iter().find(|b| b.kind == Kind::User)
                    {
                        cursor = node.parent;
                        draft = Some(user.text.clone());
                    }
                    let changed = self.session.cursor() != cursor;
                    if changed {
                        self.session.record(Record::Cursor { node: cursor })?;
                    }
                    if let Some(text) = draft {
                        self.editor.clear();
                        self.editor.insert(&text);
                    }
                    changed
                }
                Target::Session(path) => {
                    let id = path.file_stem().map(|stem| stem.to_string_lossy().into_owned()).unwrap_or_default();
                    if id != self.session.header().id {
                        self.session = Session::load(&path)?;
                    }
                    self.skills = skills::skills(&self.session.header().cwd);
                    self.files = input::FileSearch::default();
                    false
                }
            };
            self.reset_view();
            if save_cursor {
                self.save();
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
                Key::Up | Key::Char('k') => picker.selected = picker.selected.saturating_sub(1),
                Key::Down | Key::Char('j') => {
                    picker.selected = (picker.selected + 1).min(picker.entries.len().saturating_sub(1))
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
                    menu.selected = if key.code == Key::Up {
                        menu.selected.saturating_sub(1)
                    } else {
                        (menu.selected + 1).min(menu.entries.len().saturating_sub(1))
                    };
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
                if self.worker.is_some() {
                    self.stop();
                } else if !self.editor.chars.is_empty() {
                    self.editor.clear();
                } else {
                    self.quitting = true;
                }
            }
            Key::Char('d') if ctrl && self.editor.chars.is_empty() => {
                self.quitting = true;
                self.stop();
            }
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

    fn run(&mut self) -> Result<()> {
        let _terminal = ui::Terminal::enter()?;
        let mut title = String::new();
        let mut dirty = true;
        loop {
            while let Ok(event) = self.rx.try_recv() {
                match event {
                    api::Event::Push(b) => self.live.push(b),
                    api::Event::Delta(i, s) => {
                        if let Some(b) = self.live.get_mut(i) {
                            b.text.push_str(&s);
                        }
                    }
                    api::Event::Set(i, b) => {
                        if let Some(slot) = self.live.get_mut(i) {
                            *slot = b;
                        }
                    }
                    api::Event::ToolOutput(i, output) => {
                        if let Some(tool) = self.live.get_mut(i).and_then(|b| b.tool.as_mut()) {
                            tool.output = output;
                        }
                    }
                    api::Event::CacheMiss => {
                        self.live_notice = Some(Block::new(Kind::Warning, cache_miss_text()));
                    }
                    api::Event::Done(result) => self.finish(result),
                }
                dirty = true;
            }
            if let Some(result) = self.files.poll() {
                if let Err(e) = result {
                    self.warn(format!("File search: {e}"));
                }
                self.refresh_completion();
                dirty = true;
            }
            if self.quitting && self.worker.is_none() {
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
            if event::poll(Duration::from_millis(if self.worker.is_some() || self.files.pending() {
                33
            } else {
                1000
            }))? {
                let before = (self.editor.text(), self.editor.cursor);
                match event::read()? {
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

impl Drop for App {
    fn drop(&mut self) {
        if let Some(w) = self.worker.take() {
            w.cancel.store(true, Ordering::Relaxed);
            let _ = w.handle.join();
        }
    }
}

fn copy(text: &str) -> Result<()> {
    use base64::{Engine, engine::general_purpose::STANDARD};
    print!("\x1b]52;c;{}\x07", STANDARD.encode(text));
    io::stdout().flush()?;
    Ok(())
}

fn main() -> Result<()> {
    let args: Vec<_> = env::args_os().skip(1).collect();
    if args.first().is_some_and(|s| s == "--view-image") {
        if args.len() != 2 {
            return Err("Usage: view_image /path/to/image".into());
        }
        return tools::view_image(&PathBuf::from(&args[1]));
    }
    if !args.is_empty() {
        println!(
            "mu · µ · 無\n\nMU_BASE_URL=http://127.0.0.1:8317/v1 MU_API_KEY=… MU_MODEL=… MU_EFFORT=… MU_CONTEXT=… mu"
        );
        return Ok(());
    }
    let (tx, rx) = mpsc::channel();
    let cwd = env::current_dir()?;
    let skills = skills::skills(&cwd);
    let session = Session::new(
        cwd,
        Model {
            name: env::var("MU_MODEL").unwrap_or_else(|_| "gpt-6-luna".into()),
            effort: Some(env::var("MU_EFFORT").unwrap_or_else(|_| "xhigh".into())),
            context: env::var("MU_CONTEXT").ok().and_then(|s| s.parse().ok()).filter(|&n| n > 0).unwrap_or(272_000),
        },
        &skills,
    );
    let mut app = App {
        session,
        skills,
        completion: None,
        dismissed: false,
        files: input::FileSearch::default(),
        editor: ui::Editor::default(),
        live: vec![],
        live_notice: None,
        feedback: vec![],
        queued: vec![],
        worker: None,
        tx,
        rx,
        picker: None,
        expanded: false,
        scroll: 0,
        scroll_layout: None,
        transcript_static: None,
        quitting: false,
        title_status: ui::TitleStatus::Ready,
    };
    app.run()
}
