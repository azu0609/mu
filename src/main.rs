mod api;
mod input;
mod process;
mod session;
mod tools;
mod ui;

use crossterm::event::{
    self, Event as Input, KeyCode as Key, KeyEvent, KeyEventKind, KeyModifiers as Mod, MouseEventKind,
};
use session::{Block, Kind, Session};
use std::{
    env,
    io::{self, Write},
    path::PathBuf,
    process::{Command, Stdio},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    thread,
    time::Duration,
};

type Result<T> = std::result::Result<T, Box<dyn std::error::Error + Send + Sync>>;

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
    lock: session::Lock,
    editor: ui::Editor,
    live: Vec<Block>,
    notices: Vec<Block>,
    queued: Vec<input::Message>,
    skills: Vec<session::Skill>,
    completion: Option<input::Menu>,
    dismissed: bool,
    files: input::FileSearch,
    worker: Option<Worker>,
    tx: mpsc::Sender<api::Event>,
    rx: mpsc::Receiver<api::Event>,
    picker: Option<Picker>,
    expanded: bool,
    scroll: usize,
    context: u64,
    quitting: bool,
}
impl App {
    fn reset_view(&mut self) {
        self.live.clear();
        self.notices.clear();
        self.queued.clear();
        self.scroll = 0;
    }
    fn drain_queue(&mut self) {
        for message in self.queued.drain(..) {
            self.session.user(message.text, message.content);
        }
    }
    fn notice(&mut self, text: impl Into<String>) {
        self.notices.push(Block::new(Kind::Notice, text));
        if self.notices.len() > 20 {
            self.notices.remove(0);
        }
    }
    fn save(&mut self) -> bool {
        match self.session.save() {
            Ok(()) => true,
            Err(e) => {
                self.notice(format!("Session not saved: {e}"));
                false
            }
        }
    }
    fn start(&mut self) {
        if self.worker.is_some() || self.session.cursor.is_none() {
            return;
        }
        if !self.save() {
            return;
        }
        self.live.clear();
        self.scroll = 0;
        let request = api::Request {
            instructions: self.session.instructions.clone(),
            model: self.session.model.clone(),
            effort: self.session.effort.clone(),
            input: self.session.input(),
            cwd: self.session.cwd.clone(),
        };
        let cancel = Arc::new(AtomicBool::new(false));
        let flag = cancel.clone();
        let tx = self.tx.clone();
        let handle = thread::spawn(move || {
            let result = api::step(request, flag, &tx);
            // All streaming sends finish before Done; the UI can commit its live blocks.
            let _ = tx.send(api::Event::Done(result));
        });
        self.worker = Some(Worker { cancel, handle });
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
            Ok(step) => {
                if step.usage.cache_miss(self.session.usage()) {
                    self.live.push(Block::new(Kind::Notice, "cache miss · previously cached prefix was not read"));
                }
                self.session.push(step.items, std::mem::take(&mut self.live), Some(step.usage));
                let mut again = step.again;
                if !cancelled && !self.quitting {
                    again |= !self.queued.is_empty();
                    self.drain_queue();
                }
                if cancelled {
                    self.notice("Stopped. Tool side effects are not undone. Enter to continue.");
                }
                if again && !cancelled && !self.quitting {
                    self.start();
                } else {
                    self.save();
                }
            }
            Err(e) => {
                self.notices.append(&mut self.live);
                self.notice(e.to_string());
                if !self.queued.is_empty() {
                    self.notice("Steering still queued. Send a message to retry, /new to discard.");
                }
            }
        }
    }
    fn command(&mut self, text: &str) -> Result<()> {
        let args: Vec<_> = text.split_whitespace().collect();
        let name = args.first().copied().unwrap_or("");
        if name == "/quit" {
            self.quitting = true;
            self.stop();
            return Ok(());
        }
        if name == "/copy" {
            let kind = match args.get(1).copied() {
                None | Some("agent") => Kind::Agent,
                Some("user") => Kind::User,
                _ => return Err("Usage: /copy [agent|user]".into()),
            };
            let text = self.session.last_text(kind).ok_or("Nothing to copy")?;
            copy(&text)?;
            self.notice("Copied.");
            return Ok(());
        }
        if self.worker.is_some() {
            return Err("Stop the agent with Esc before changing sessions or models".into());
        }
        match name {
            "/model" => {
                if !(2..=3).contains(&args.len()) {
                    return Err("Usage: /model <model> [effort]".into());
                }
                self.session.model = args[1].into();
                self.session.effort = args.get(2).map(|s| (*s).into());
                self.save();
            }
            "/new" => {
                self.skills = session::skills(&self.session.cwd);
                let session = Session::new(
                    self.session.cwd.clone(),
                    self.session.model.clone(),
                    self.session.effort.clone(),
                    &self.skills,
                );
                self.lock = session::Lock::acquire(&session.id)?;
                self.session = session;
                self.files = input::FileSearch::default();
                self.reset_view();
            }
            "/tree" => {
                let entries: Vec<_> = self
                    .session
                    .tree()
                    .into_iter()
                    .map(|(i, label)| {
                        (Target::Node(i), format!("{}{label}", if i == self.session.cursor { "● " } else { "  " }))
                    })
                    .collect();
                let selected = entries
                    .iter()
                    .position(|(t, _)| matches!(t, Target::Node(i) if *i == self.session.cursor))
                    .unwrap_or(0);
                self.picker = Some(Picker { title: "conversation tree", entries, selected });
            }
            "/resume" => {
                let entries: Vec<_> =
                    session::sessions()?.into_iter().map(|(p, label)| (Target::Session(p), label)).collect();
                if entries.is_empty() {
                    return Err("No saved sessions".into());
                }
                self.picker = Some(Picker { title: "resume session", entries, selected: 0 });
            }
            _ => {
                return Err("Commands: /model <model> [effort], /new, /resume, /tree, /copy [agent|user], /quit".into());
            }
        }
        Ok(())
    }
    fn submit(&mut self) {
        if self.editor.text().trim().is_empty() {
            return;
        }
        if let Err(e) = self.send_input() {
            self.notice(e.to_string());
        }
        self.scroll = 0;
    }
    fn send_input(&mut self) -> Result<()> {
        let mut text = self.editor.text();
        let mut skill = None;
        if text.trim_start().starts_with('/') {
            let word = text.split_whitespace().next().unwrap();
            let name = input::resolve(word, &self.skills)?;
            text = format!("{}{}", name, &text.trim_start()[word.len()..]);
            if input::COMMANDS.iter().any(|(command, _)| *command == name) {
                let result = self.command(&text);
                self.editor.clear();
                return result;
            }
            skill = self.skills.iter().rev().find(|s| format!("/{}", s.name) == name);
        }
        // Snapshot all attachments before clearing the draft or touching the queue.
        let message = input::prepare(text, &self.session.cwd, skill)?;
        self.editor.clear();
        if !message.text.trim().is_empty() {
            self.queued.push(message);
        }
        if self.worker.is_none() {
            self.notices.clear();
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
        let mut menu =
            input::menu(&self.editor.text(), self.editor.cursor, &self.session.cwd, &self.skills, &self.files.files);
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
                self.files.start(&self.session.cwd);
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
                    if let Some(node) = i.map(|i| &self.session.nodes[i])
                        && let Some(user) = node.blocks.iter().find(|b| b.kind == Kind::User)
                    {
                        cursor = node.parent;
                        draft = Some(user.text.clone());
                    }
                    let changed = self.session.cursor != cursor;
                    self.session.cursor = cursor;
                    if let Some(text) = draft {
                        self.editor.clear();
                        self.editor.insert(&text);
                    }
                    changed
                }
                Target::Session(path) => {
                    // Strict single reader/writer: take ownership before reading the session.
                    let id = path.file_stem().map(|stem| stem.to_string_lossy().into_owned()).unwrap_or_default();
                    if id != self.session.id {
                        let lock = session::Lock::acquire(&id)?;
                        self.session = Session::load(&path)?;
                        self.lock = lock;
                    }
                    self.skills = session::skills(&self.session.cwd);
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
    fn key(&mut self, key: KeyEvent) {
        if key.kind == KeyEventKind::Release {
            return;
        }
        let ctrl = key.modifiers.contains(Mod::CONTROL);
        if ctrl && key.code == Key::Char('o') {
            self.expanded = !self.expanded;
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
                        self.notice(e.to_string());
                    }
                }
                _ => (),
            }
            return;
        }
        if let Some(menu) = &mut self.completion {
            match key.code {
                Key::Up | Key::Down => {
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
                Key::Enter if !key.modifiers.intersects(Mod::ALT | Mod::SHIFT) => {
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
            Key::PageUp => self.scroll = self.scroll.saturating_add(10),
            Key::PageDown => self.scroll = self.scroll.saturating_sub(10),
            Key::Enter if key.modifiers.intersects(Mod::ALT | Mod::SHIFT) => self.editor.insert("\n"),
            Key::Char('j') if ctrl => self.editor.insert("\n"),
            Key::Enter => self.submit(),
            Key::Left => self.editor.cursor = self.editor.cursor.saturating_sub(1),
            Key::Right => self.editor.cursor = (self.editor.cursor + 1).min(self.editor.chars.len()),
            Key::Home => self.editor.home(),
            Key::End => self.editor.end(),
            Key::Char('a') if ctrl => self.editor.home(),
            Key::Char('e') if ctrl => self.editor.end(),
            Key::Char('u') if ctrl => self.editor.clear(),
            Key::Char('w') if ctrl => self.editor.word_backspace(),
            Key::Backspace => self.editor.backspace(),
            Key::Delete => self.editor.delete(),
            Key::Char(c) if !ctrl && !key.modifiers.contains(Mod::ALT) => self.editor.insert(&c.to_string()),
            _ => (),
        }
    }
    fn run(&mut self) -> Result<()> {
        let _terminal = ui::Terminal::enter()?;
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
                    api::Event::Done(result) => self.finish(result),
                }
                dirty = true;
            }
            if let Some(result) = self.files.poll() {
                if let Err(e) = result {
                    self.notice(format!("File search: {e}"));
                }
                self.refresh_completion();
                dirty = true;
            }
            if self.quitting && self.worker.is_none() {
                break;
            }
            if dirty {
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
                        MouseEventKind::ScrollUp => self.scroll = self.scroll.saturating_add(3),
                        MouseEventKind::ScrollDown => self.scroll = self.scroll.saturating_sub(3),
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
    for (program, args, available) in [
        ("wl-copy", vec![], env::var_os("WAYLAND_DISPLAY").is_some()),
        ("xclip", vec!["-selection", "clipboard"], env::var_os("DISPLAY").is_some()),
    ] {
        if !available {
            continue;
        }
        if let Ok(mut child) =
            Command::new(program).args(args).stdin(Stdio::piped()).stdout(Stdio::null()).stderr(Stdio::null()).spawn()
        {
            let result = child.stdin.take().unwrap().write_all(text.as_bytes());
            if child.wait()?.success() && result.is_ok() {
                return Ok(());
            }
        }
    }
    use base64::{Engine, engine::general_purpose::STANDARD};
    // OSC 52 works over SSH too, if enabled by the terminal.
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
            "mu · µ · 無\n\nMU_BASE_URL=http://127.0.0.1:8317/v1 MU_API_KEY=… MU_MODEL=… mu\n\n/model <model> [effort]  /new  /resume  /tree  /copy [agent|user]  /quit\nCtrl+O expand · Esc stop · Alt+Enter newline · PgUp/PgDn scroll"
        );
        return Ok(());
    }
    let (tx, rx) = mpsc::channel();
    let cwd = env::current_dir()?;
    let skills = session::skills(&cwd);
    let session =
        Session::new(cwd, env::var("MU_MODEL").unwrap_or_else(|_| "gpt-5".into()), env::var("MU_EFFORT").ok(), &skills);
    let lock = session::Lock::acquire(&session.id)?;
    let mut app = App {
        session,
        lock,
        skills,
        completion: None,
        dismissed: false,
        files: input::FileSearch::default(),
        editor: ui::Editor::default(),
        live: vec![],
        notices: vec![],
        queued: vec![],
        worker: None,
        tx,
        rx,
        picker: None,
        expanded: false,
        scroll: 0,
        context: env::var("MU_CONTEXT").ok().and_then(|s| s.parse().ok()).filter(|&n| n > 0).unwrap_or(128_000),
        quitting: false,
    };
    app.run()
}
