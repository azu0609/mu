// mu's client core for the apps, through UniFFI. It is shaped around what the
// app does (paired hosts, their sessions, one open session at a time) rather
// than mirroring the wire, and keeps every rule in mu-client: the app only
// draws what it is told.
//
// Listeners are called on the core's own threads; hop to the UI's before
// touching its state.
use mu_client::{
    Change, Client, Endpoint, HostView, Invite, ShellEvent, Update, View,
    hosts::{Hosts, Paired},
};
use std::{
    collections::HashMap,
    fmt,
    future::Future,
    path::PathBuf,
    sync::{Arc, Mutex},
};
use tokio::runtime::{Handle, Runtime};

uniffi::setup_scaffolding!();

#[uniffi::export]
pub fn version() -> String {
    env!("CARGO_PKG_VERSION").into()
}

#[derive(Debug, uniffi::Error)]
#[uniffi(flat_error)]
pub enum MuError {
    // The host no longer accepts this device: pair it again.
    Unpaired(String),
    // Anything else, worded for a person.
    Failed(String),
}

impl fmt::Display for MuError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            Self::Unpaired(message) | Self::Failed(message) => f.write_str(message),
        }
    }
}

impl std::error::Error for MuError {}

impl From<mu_client::Error> for MuError {
    fn from(error: mu_client::Error) -> Self {
        match error {
            mu_client::Error::Unpaired(message) => Self::Unpaired(message),
            mu_client::Error::Other(message) => Self::Failed(message),
        }
    }
}

// Runs `future` on the core's runtime, whatever the caller awaits it with.
async fn spawn<T: Send + 'static>(
    runtime: &Handle,
    future: impl Future<Output = mu_client::Result<T>> + Send + 'static,
) -> Result<T, MuError> {
    runtime.spawn(future).await.map_err(|e| MuError::Failed(e.to_string()))?.map_err(Into::into)
}

// A paired host. Its id is its certificate's fingerprint, so pairing with the
// same host again keeps it. Its name, icon and description are as it last
// described itself, so they are there while it is away.
#[derive(uniffi::Record)]
pub struct Host {
    pub id: String,
    pub name: String,
    pub icon: Option<String>,
    pub description: String,
    pub address: String,
}

impl From<&Paired> for Host {
    fn from(host: &Paired) -> Self {
        Self {
            id: host.remote.cert.clone(),
            name: host.name.clone(),
            icon: host.icon.clone(),
            description: host.description.clone(),
            address: host.remote.address.clone(),
        }
    }
}

// One per app: the paired hosts, and a connection to each one in use.
#[derive(uniffi::Object)]
pub struct Core {
    runtime: Runtime,
    hosts: Arc<Mutex<Hosts>>,
    // By host id. A connection re-establishes itself when it breaks.
    clients: Arc<Mutex<HashMap<String, Client>>>,
}

#[uniffi::export]
impl Core {
    // Keeps its files, the paired hosts' tokens among them, in `dir`.
    #[uniffi::constructor]
    pub fn new(dir: String) -> Result<Arc<Self>, MuError> {
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .thread_name("mu")
            .enable_all()
            .build()
            .map_err(|e| MuError::Failed(e.to_string()))?;
        let hosts = Hosts::load(PathBuf::from(dir).join("hosts.json"))?;
        Ok(Arc::new(Self { runtime, hosts: Arc::new(Mutex::new(hosts)), clients: Arc::default() }))
    }

    // In the order they were paired.
    pub fn hosts(&self) -> Vec<Host> {
        self.hosts.lock().unwrap().list().iter().map(Host::from).collect()
    }

    // Pairs with the host behind a `mu://` invite, as `device` (what the host
    // lists it as).
    pub async fn pair(&self, invite: String, device: String) -> Result<Host, MuError> {
        let invite: Invite = invite.parse().map_err(MuError::Failed)?;
        let (remote, info) =
            spawn(self.runtime.handle(), async move { mu_client::pair(&invite, &device).await }).await?;
        // The rest of how it describes itself arrives once it is watched.
        let paired = Paired { name: info.name, icon: None, description: String::new(), remote };
        self.hosts.lock().unwrap().add(paired.clone())?;
        // A connection made with an older token would keep presenting it.
        self.clients.lock().unwrap().remove(&paired.remote.cert);
        Ok(Host::from(&paired))
    }

    // Forgets a host on this device only; the host still lists the device
    // until it is unpaired there.
    pub fn forget(&self, host: String) -> Result<(), MuError> {
        self.hosts.lock().unwrap().remove(&host)?;
        self.clients.lock().unwrap().remove(&host);
        Ok(())
    }

    // Latest first. To keep a list current, watch the host instead.
    pub async fn sessions(&self, host: String) -> Result<Vec<SessionSummary>, MuError> {
        let sessions = self.on(&host, |client| async move { client.sessions().await }).await?;
        Ok(sessions.into_iter().map(Into::into).collect())
    }

    // Starts a session in `cwd` on the host, with the host's default model.
    pub async fn create_session(&self, host: String, cwd: String) -> Result<String, MuError> {
        self.on(&host, |client| async move { client.create(cwd.as_ref(), None, None, None).await }).await
    }

    // Leaves alone sessions in use: running, or open in some client.
    pub async fn delete_sessions(&self, host: String) -> Result<DeletedSessions, MuError> {
        let deleted = self.on(&host, |client| async move { client.delete_sessions().await }).await?;
        Ok(DeletedSessions { deleted: deleted.sessions as i64, in_use: deleted.in_use as i64 })
    }

    // Follows the host's profile and session list until the returned handle is
    // closed. The listener hears everything first, then again on every change.
    // Remembers how the host describes itself, for `hosts` while it is away.
    pub async fn watch_host(&self, host: String, listener: Arc<dyn HostListener>) -> Result<Arc<Watch>, MuError> {
        let mut watch = self.on(&host, |client| async move { client.watch().await }).await?;
        let hosts = self.hosts.clone();
        let task = self.runtime.spawn(async move {
            let mut view = HostView::default();
            let mut lost = false;
            loop {
                match watch.next().await {
                    Update::Event(event) => {
                        lost = false;
                        view.apply(event);
                        let profile = &view.profile;
                        // Not worth interrupting the watch over: it is only for while the host is away.
                        let _ = hosts.lock().unwrap().update(&host, |paired| {
                            paired.name = profile.name.clone();
                            paired.icon = profile.icon.clone();
                            paired.description = profile.description.clone();
                        });
                        listener.on_update(HostUpdate::Host {
                            profile: profile.into(),
                            sessions: view.sessions.iter().cloned().map(Into::into).collect(),
                        });
                    }
                    Update::Lost if lost => (),
                    Update::Lost => {
                        lost = true;
                        listener.on_update(HostUpdate::Lost);
                    }
                }
            }
        });
        Ok(Arc::new(Watch { task }))
    }

    // The profile's changes reach watchers of the host. An unset or empty icon
    // goes back to the default.
    pub async fn set_icon(&self, host: String, icon: Option<String>) -> Result<(), MuError> {
        self.on(&host, |client| async move { client.set_icon(icon).await }).await
    }

    pub async fn set_description(&self, host: String, description: String) -> Result<(), MuError> {
        self.on(&host, |client| async move { client.set_description(description).await }).await
    }

    // Adds a snippet, or replaces the one with this id. Returns its id.
    pub async fn save_snippet(
        &self,
        host: String,
        id: Option<String>,
        name: String,
        command: String,
        icon: Option<String>,
    ) -> Result<String, MuError> {
        self.on(&host, |client| async move { client.save_snippet(id, name, command, icon).await }).await
    }

    pub async fn delete_snippet(&self, host: String, id: String) -> Result<(), MuError> {
        self.on(&host, |client| async move { client.delete_snippet(&id).await }).await
    }

    // A directory on the host, for choosing where a session works. `~` and an
    // empty path name the host's home; `<path>/..` names a directory's parent.
    pub async fn list_directory(&self, host: String, path: String) -> Result<Directory, MuError> {
        let directory = self.on(&host, |client| async move { client.list_directory(path.as_ref()).await }).await?;
        Ok(Directory {
            path: directory.path.to_string_lossy().into_owned(),
            entries: directory
                .entries
                .into_iter()
                .map(|entry| DirectoryEntry {
                    name: entry.name,
                    kind: match entry.kind {
                        mu_client::EntryKind::File => EntryKind::File,
                        mu_client::EntryKind::Directory => EntryKind::Directory,
                        mu_client::EntryKind::Repository => EntryKind::Repository,
                    },
                })
                .collect(),
        })
    }

    // The skills a session in `cwd` would have, for before there is one.
    pub async fn list_skills(&self, host: String, cwd: String) -> Result<Vec<Skill>, MuError> {
        let skills = self.on(&host, |client| async move { client.list_skills(cwd.as_ref()).await }).await?;
        Ok(skills.into_iter().map(|skill| Skill { name: skill.name, description: skill.description }).collect())
    }

    // Runs a command the user typed, in `cwd` on the host, until it ends or the
    // returned handle is closed, which kills it. The listener hears its output
    // as it goes, then how it ended: a run that an open session can add to its
    // conversation.
    pub async fn shell(
        &self,
        host: String,
        cwd: String,
        command: String,
        listener: Arc<dyn ShellListener>,
    ) -> Result<Arc<Shell>, MuError> {
        let mut run = self.on(&host, |client| async move { client.shell(cwd.as_ref(), command).await }).await?;
        let task = self.runtime.spawn(async move {
            loop {
                let update = match run.next().await {
                    Ok(Some(ShellEvent::Output(output))) => {
                        listener.on_update(ShellUpdate::Output { tool: tool(mu_client::ToolStatus::Running, &output) });
                        continue;
                    }
                    Ok(Some(ShellEvent::Done { run, status, output })) => {
                        ShellUpdate::Done { run, tool: tool(status, &output) }
                    }
                    Ok(None) => ShellUpdate::Failed { message: "The host stopped sending the command's output".into() },
                    Err(e) => ShellUpdate::Failed { message: e.to_string() },
                };
                listener.on_update(update);
                return;
            }
        });
        Ok(Arc::new(Shell { task }))
    }

    // Follows a session until the returned handle is closed. The listener
    // hears a reset first, and again after every break.
    pub async fn open_session(
        &self,
        host: String,
        session: String,
        listener: Arc<dyn SessionListener>,
    ) -> Result<Arc<Session>, MuError> {
        let connect = self.connect(&host)?;
        let id = session.clone();
        let (client, mut subscription) = spawn(self.runtime.handle(), async move {
            let client = connect.await?;
            let subscription = client.subscribe(&id).await?;
            Ok((client, subscription))
        })
        .await?;
        let task = self.runtime.spawn(async move {
            let mut view = View::default();
            // Retries keep failing while the host is away; that is one loss.
            let mut lost = false;
            loop {
                let update = match subscription.next().await {
                    Update::Event(event) => {
                        lost = false;
                        let change = view.apply(event);
                        translate(&view, change)
                    }
                    Update::Lost if lost => None,
                    Update::Lost => {
                        lost = true;
                        Some(SessionUpdate::Lost)
                    }
                };
                if let Some(update) = update {
                    listener.on_update(update);
                }
            }
        });
        Ok(Arc::new(Session { id: session, client, runtime: self.runtime.handle().clone(), task }))
    }
}

impl Core {
    // Runs `call` with the connection to a paired host, on the core's runtime.
    async fn on<T, F>(&self, host: &str, call: impl FnOnce(Client) -> F + Send + 'static) -> Result<T, MuError>
    where
        T: Send + 'static,
        F: Future<Output = mu_client::Result<T>> + Send + 'static,
    {
        let connect = self.connect(host)?;
        spawn(self.runtime.handle(), async move { call(connect.await?).await }).await
    }

    // The connection to a paired host, made when first needed.
    fn connect(&self, host: &str) -> Result<impl Future<Output = mu_client::Result<Client>> + use<>, MuError> {
        let cached = self.clients.lock().unwrap().get(host).cloned();
        let hosts = self.hosts.lock().unwrap();
        let remote = hosts.get(host).ok_or_else(|| MuError::Failed("This host is not paired".into()))?.remote.clone();
        let (clients, host) = (self.clients.clone(), host.to_owned());
        Ok(async move {
            if let Some(client) = cached {
                return Ok(client);
            }
            let client = Client::connect(Endpoint::Remote(remote)).await?;
            clients.lock().unwrap().insert(host, client.clone());
            Ok(client)
        })
    }
}

fn translate(view: &View, change: Change) -> Option<SessionUpdate> {
    Some(match change {
        Change::Reset { keep } => SessionUpdate::Reset {
            keep: keep as i32,
            cwd: view.cwd.to_string_lossy().into_owned(),
            state: state(view),
            skills: view
                .skills
                .iter()
                .map(|skill| Skill { name: skill.name.clone(), description: skill.description.clone() })
                .collect(),
        },
        Change::State { started } => SessionUpdate::State { state: state(view), started },
        Change::Committed { from } => {
            SessionUpdate::Committed { blocks: view.blocks[from..].iter().map(Into::into).collect() }
        }
        Change::Live(index) => SessionUpdate::Live { index: index as i32, block: view.live.get(index)?.into() },
        Change::LiveNotice => SessionUpdate::LiveNotice { block: view.live_notice.as_ref()?.into() },
        Change::Feedback(block) => SessionUpdate::Feedback { block: (&block).into() },
    })
}

fn state(view: &View) -> SessionState {
    SessionState {
        status: view.status.into(),
        title: view.title.clone(),
        model: view.model.name.clone(),
        effort: view.model.effort.clone(),
        context: view.model.context as i64,
        context_used: view.stats.context_used as i64,
        queued: view.queued.clone(),
    }
}

// An open session. Closing it stops following the session, not the session.
#[derive(uniffi::Object)]
pub struct Session {
    id: String,
    client: Client,
    runtime: Handle,
    task: tokio::task::JoinHandle<()>,
}

#[uniffi::export]
impl Session {
    pub fn id(&self) -> String {
        self.id.clone()
    }

    // Starts a step, or queues the message as steering while one is running.
    // `skill` names one of the session's, to invoke with the message.
    pub async fn send(&self, text: String, skill: Option<String>) -> Result<(), MuError> {
        let (client, id) = (self.client.clone(), self.id.clone());
        spawn(&self.runtime, async move { client.send(&id, text, skill).await }).await
    }

    // Adds a command run in shell mode, and its result, to the conversation for
    // the agent's next step. Only while the agent is not running, and after a
    // message: a session is never just commands. A refused run can be added later.
    pub async fn append_shell(&self, run: String) -> Result<(), MuError> {
        let (client, id) = (self.client.clone(), self.id.clone());
        spawn(&self.runtime, async move { client.append_shell(&id, &run).await }).await
    }

    pub async fn stop(&self) -> Result<(), MuError> {
        let (client, id) = (self.client.clone(), self.id.clone());
        spawn(&self.runtime, async move { client.stop(&id).await }).await
    }

    // Call while the session is on screen and its state says it is not
    // running: everything the agent did so far has been seen.
    pub async fn mark_read(&self) -> Result<(), MuError> {
        let (client, id) = (self.client.clone(), self.id.clone());
        spawn(&self.runtime, async move { client.mark_read(&id).await }).await
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        self.task.abort();
    }
}

// A watched host. Closing it stops watching.
#[derive(uniffi::Object)]
pub struct Watch {
    task: tokio::task::JoinHandle<()>,
}

impl Drop for Watch {
    fn drop(&mut self) {
        self.task.abort();
    }
}

#[uniffi::export(with_foreign)]
pub trait HostListener: Send + Sync {
    fn on_update(&self, update: HostUpdate);
}

#[derive(uniffi::Enum)]
pub enum HostUpdate {
    // Everything there is to show of the host, whole every time; sessions
    // latest first.
    Host { profile: HostProfile, sessions: Vec<SessionSummary> },
    // The connection broke; the host follows again once it is back.
    Lost,
}

// What a host says of itself.
#[derive(uniffi::Record)]
pub struct HostProfile {
    // The machine's hostname.
    pub name: String,
    // The home directory of the user the host runs as, for showing paths under it as `~`.
    pub home: String,
    // The rest is up to its owner. An icon's name in the app's icon set; unset
    // for the default.
    pub icon: Option<String>,
    // One line; empty when none was set.
    pub description: String,
    // Commands to offer in shell mode, in order.
    pub snippets: Vec<Snippet>,
}

impl From<&mu_client::Profile> for HostProfile {
    fn from(profile: &mu_client::Profile) -> Self {
        Self {
            name: profile.name.clone(),
            home: profile.home.to_string_lossy().into_owned(),
            icon: profile.icon.clone(),
            description: profile.description.clone(),
            snippets: profile
                .snippets
                .iter()
                .map(|snippet| Snippet {
                    id: snippet.id.clone(),
                    name: snippet.name.clone(),
                    command: snippet.command.clone(),
                    icon: snippet.icon.clone(),
                })
                .collect(),
        }
    }
}

#[derive(uniffi::Record)]
pub struct Snippet {
    pub id: String,
    pub name: String,
    pub command: String,
    pub icon: Option<String>,
}

#[derive(uniffi::Record)]
pub struct DeletedSessions {
    pub deleted: i64,
    // Left alone for being in use: running, or open in some client.
    pub in_use: i64,
}

#[derive(uniffi::Record)]
pub struct Directory {
    // Absolute and canonical.
    pub path: String,
    // Directories first, then by name. Hidden ones included.
    pub entries: Vec<DirectoryEntry>,
}

#[derive(uniffi::Record)]
pub struct DirectoryEntry {
    pub name: String,
    pub kind: EntryKind,
}

#[derive(uniffi::Enum)]
pub enum EntryKind {
    File,
    Directory,
    // A directory at the top of a git repository.
    Repository,
}

// A command running in shell mode. Closing it kills the command.
#[derive(uniffi::Object)]
pub struct Shell {
    task: tokio::task::JoinHandle<()>,
}

impl Drop for Shell {
    fn drop(&mut self) {
        self.task.abort();
    }
}

#[uniffi::export(with_foreign)]
pub trait ShellListener: Send + Sync {
    fn on_update(&self, update: ShellUpdate);
}

// Each update replaces the last; the final one is `Done` or `Failed`.
#[derive(uniffi::Enum)]
pub enum ShellUpdate {
    // The output so far, while it runs.
    Output { tool: Tool },
    // The command ended. An open session can add `run` to its conversation for
    // a while: see `Session::append_shell`.
    Done { run: String, tool: Tool },
    // The connection broke before the command ended; the host killed it.
    Failed { message: String },
}

#[uniffi::export(with_foreign)]
pub trait SessionListener: Send + Sync {
    fn on_update(&self, update: SessionUpdate);
}

// How an open session changed. A reset clears everything but the first `keep`
// blocks of the transcript; the rest of it then arrives as ordinary commits.
#[derive(uniffi::Enum)]
pub enum SessionUpdate {
    Reset { keep: i32, cwd: String, state: SessionState, skills: Vec<Skill> },
    // `started` when the agent just began running.
    State { state: SessionState, started: bool },
    // Appended to the transcript; the live step ended.
    Committed { blocks: Vec<Block> },
    // The step in progress: the block at `index` was added, or now reads `block`.
    Live { index: i32, block: Block },
    // A notice about the step in progress, like a cache miss.
    LiveNotice { block: Block },
    // Transient, not part of the session.
    Feedback { block: Block },
    // The connection broke; a reset follows once the session is back.
    Lost,
}

#[derive(uniffi::Record)]
pub struct SessionState {
    pub status: Status,
    pub title: String,
    pub model: String,
    pub effort: Option<String>,
    pub context: i64,
    // Tokens the latest response occupied in the context window.
    pub context_used: i64,
    // Messages waiting to steer the running agent, oldest first.
    pub queued: Vec<String>,
}

#[derive(uniffi::Record)]
pub struct Skill {
    pub name: String,
    pub description: String,
}

#[derive(uniffi::Record)]
pub struct SessionSummary {
    pub id: String,
    pub title: String,
    pub cwd: String,
    pub status: Status,
    // Why its latest run failed.
    pub error: Option<String>,
    // Not running, with steps no client has shown since they finished.
    pub unread: bool,
    // When its latest step finished, in milliseconds since the Unix epoch.
    pub updated: i64,
}

impl From<mu_client::SessionSummary> for SessionSummary {
    fn from(summary: mu_client::SessionSummary) -> Self {
        Self {
            id: summary.id,
            title: summary.title,
            cwd: summary.cwd.to_string_lossy().into_owned(),
            status: summary.status.into(),
            error: summary.error,
            unread: summary.unread,
            updated: summary.updated as i64,
        }
    }
}

#[derive(uniffi::Enum)]
pub enum Status {
    Idle,
    Running,
    Finished,
    Failed,
}

impl From<mu_client::Status> for Status {
    fn from(status: mu_client::Status) -> Self {
        match status {
            mu_client::Status::Idle => Self::Idle,
            mu_client::Status::Running => Self::Running,
            mu_client::Status::Finished => Self::Finished,
            mu_client::Status::Failed => Self::Failed,
        }
    }
}

#[derive(uniffi::Record)]
pub struct Block {
    pub kind: BlockKind,
    pub text: String,
    // Set for calls.
    pub tool: Option<Tool>,
    // When the step it belongs to ran; set once it is in the transcript. A
    // turn's "Worked for" runs from its first block's start to its last's finish.
    pub span: Option<Span>,
}

// Milliseconds since the Unix epoch.
#[derive(uniffi::Record)]
pub struct Span {
    pub started: i64,
    pub finished: i64,
}

impl From<&mu_client::Block> for Block {
    fn from(block: &mu_client::Block) -> Self {
        Self {
            kind: match block.kind {
                mu_client::Kind::User => BlockKind::User,
                mu_client::Kind::Agent => BlockKind::Agent,
                mu_client::Kind::Thought => BlockKind::Thought,
                mu_client::Kind::Call => BlockKind::Call,
                mu_client::Kind::Shell => BlockKind::Shell,
                mu_client::Kind::Notice => BlockKind::Notice,
                mu_client::Kind::Warning => BlockKind::Warning,
            },
            text: block.text.clone(),
            tool: block.tool.as_ref().map(|t| tool(t.status, &t.output)),
            span: block.span.map(|span| Span { started: span.started as i64, finished: span.finished as i64 }),
        }
    }
}

#[derive(uniffi::Enum)]
pub enum BlockKind {
    User,
    Agent,
    Thought,
    Call,
    // A command the user ran themselves; the text is the command.
    Shell,
    Notice,
    Warning,
}

#[derive(uniffi::Record)]
pub struct Tool {
    pub status: ToolStatus,
    // A preview when `truncated`; the full log stays on the host.
    pub output: String,
    pub truncated: bool,
}

fn tool(status: mu_client::ToolStatus, output: &mu_client::ToolOutput) -> Tool {
    Tool {
        status: match status {
            mu_client::ToolStatus::Pending => ToolStatus::Pending,
            mu_client::ToolStatus::Running => ToolStatus::Running,
            mu_client::ToolStatus::Exited(code) => ToolStatus::Exited { code },
            mu_client::ToolStatus::TimedOut(millis) => ToolStatus::TimedOut { millis: millis as i64 },
            mu_client::ToolStatus::Cancelled => ToolStatus::Cancelled,
            mu_client::ToolStatus::Error => ToolStatus::Failed,
        },
        output: output.text.clone(),
        truncated: output.truncated,
    }
}

#[derive(uniffi::Enum)]
pub enum ToolStatus {
    Pending,
    Running,
    Exited { code: i32 },
    TimedOut { millis: i64 },
    Cancelled,
    // The tool could not run at all.
    Failed,
}
