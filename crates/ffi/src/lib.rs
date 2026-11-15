// Listeners are called on the core's own threads; hop to the UI's before
// touching its state.
use mu_client::{
    Change, Client, Endpoint, HostView, Invite, Model, ShellEvent, Update, View,
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

// Offsets count characters, not UTF-16 units.
#[uniffi::export]
pub fn mentions(text: String) -> Vec<Written> {
    mu_client::mentions(&text)
        .into_iter()
        .map(|mention| Written {
            start: mention.range.start as u32,
            end: mention.range.end as u32,
            path: mention.path,
            closed: mention.closed,
        })
        .collect()
}

#[derive(Debug, uniffi::Error)]
#[uniffi(flat_error)]
pub enum MuError {
    Unpaired(String),
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

#[derive(uniffi::Object)]
pub struct Core {
    runtime: Runtime,
    hosts: Arc<Mutex<Hosts>>,
    clients: Arc<Mutex<HashMap<String, Client>>>,
}

#[uniffi::export]
impl Core {
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

    pub fn hosts(&self) -> Vec<Host> {
        self.hosts.lock().unwrap().list().iter().map(Host::from).collect()
    }

    pub async fn pair(&self, invite: String, device: String) -> Result<Host, MuError> {
        let invite: Invite = invite.parse().map_err(MuError::Failed)?;
        let (remote, info) =
            spawn(self.runtime.handle(), async move { mu_client::pair(&invite, &device).await }).await?;
        let paired = Paired { name: info.name, icon: None, description: String::new(), remote };
        self.hosts.lock().unwrap().add(paired.clone())?;
        // A connection made with an older token would keep presenting it.
        self.clients.lock().unwrap().remove(&paired.remote.cert);
        Ok(Host::from(&paired))
    }

    pub fn forget(&self, host: String) -> Result<(), MuError> {
        self.hosts.lock().unwrap().remove(&host)?;
        self.clients.lock().unwrap().remove(&host);
        Ok(())
    }

    pub async fn sessions(&self, host: String) -> Result<Vec<SessionSummary>, MuError> {
        let sessions = self.on(&host, |client| async move { client.sessions().await }).await?;
        Ok(sessions.into_iter().map(Into::into).collect())
    }

    pub async fn create_session(
        &self,
        host: String,
        cwd: String,
        model: String,
        effort: Option<String>,
        text: String,
        skill: Option<String>,
    ) -> Result<String, MuError> {
        let model = Model { name: model, effort, context: None };
        self.on(&host, |client| async move { client.create(cwd.as_ref(), model, text, skill).await }).await
    }

    pub async fn list_models(&self, host: String) -> Result<Vec<ModelInfo>, MuError> {
        let models = self.on(&host, |client| async move { client.models().await }).await?;
        Ok(models
            .into_iter()
            .map(|model| ModelInfo { name: model.name, context: model.context.map(|context| context as i64) })
            .collect())
    }

    pub async fn delete_sessions(&self, host: String) -> Result<DeletedSessions, MuError> {
        let deleted = self.on(&host, |client| async move { client.delete_sessions().await }).await?;
        Ok(DeletedSessions { deleted: deleted.sessions as i64, in_use: deleted.in_use as i64 })
    }

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

    pub async fn set_icon(&self, host: String, icon: Option<String>) -> Result<(), MuError> {
        self.on(&host, |client| async move { client.set_icon(icon).await }).await
    }

    pub async fn set_description(&self, host: String, description: String) -> Result<(), MuError> {
        self.on(&host, |client| async move { client.set_description(description).await }).await
    }

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

    pub async fn upload(&self, host: String, name: String, data: Vec<u8>) -> Result<String, MuError> {
        let path = self.on(&host, |client| async move { client.upload(&name, &data).await }).await?;
        Ok(path.to_string_lossy().into_owned())
    }

    pub async fn list_skills(&self, host: String, cwd: String) -> Result<Vec<Skill>, MuError> {
        let skills = self.on(&host, |client| async move { client.list_skills(cwd.as_ref()).await }).await?;
        Ok(skills.into_iter().map(|skill| Skill { name: skill.name, description: skill.description }).collect())
    }

    // Offsets count characters, not UTF-16 units.
    pub async fn complete_mention(
        &self,
        host: String,
        cwd: String,
        text: String,
        cursor: u32,
        refresh: bool,
    ) -> Result<Option<Mention>, MuError> {
        let mention = self
            .on(&host, move |client| async move { client.complete(cwd.as_ref(), text, cursor as usize, refresh).await })
            .await?;
        Ok(mention.map(|mention| Mention {
            start: mention.range.start as u32,
            end: mention.range.end as u32,
            query: mention.query,
            entries: mention
                .entries
                .into_iter()
                .map(|entry| MentionEntry { text: entry.text, label: entry.label })
                .collect(),
        }))
    }

    pub async fn resolve_mentions(
        &self,
        host: String,
        cwd: String,
        text: String,
        skill: Option<String>,
    ) -> Result<Vec<Resolved>, MuError> {
        let resolved =
            self.on(&host, move |client| async move { client.resolve(cwd.as_ref(), text, skill).await }).await?;
        Ok(resolved
            .into_iter()
            .map(|resolved| Resolved {
                path: resolved.path,
                file: resolved.file.to_string_lossy().into_owned(),
                kind: match resolved.kind {
                    mu_client::Attached::Image => Attached::Image,
                    mu_client::Attached::Text { lines, head } => Attached::Text { lines: lines as u32, head },
                },
            })
            .collect())
    }

    pub async fn read_image(&self, host: String, path: String) -> Result<Vec<u8>, MuError> {
        self.on(&host, |client| async move { client.read_image(path.as_ref()).await }).await
    }

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
    async fn on<T, F>(&self, host: &str, call: impl FnOnce(Client) -> F + Send + 'static) -> Result<T, MuError>
    where
        T: Send + 'static,
        F: Future<Output = mu_client::Result<T>> + Send + 'static,
    {
        let connect = self.connect(host)?;
        spawn(self.runtime.handle(), async move { call(connect.await?).await }).await
    }

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
        context: view.model.context.map(|context| context as i64),
        context_used: view.stats.context_used as i64,
        input: view.stats.input as i64,
        uncached_input: view.stats.uncached_input as i64,
        output: view.stats.output as i64,
        queued: view.queued.clone(),
    }
}

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

    pub async fn send(&self, text: String, skill: Option<String>) -> Result<(), MuError> {
        let (client, id) = (self.client.clone(), self.id.clone());
        spawn(&self.runtime, async move { client.send(&id, text, skill).await }).await
    }

    pub async fn append_shell(&self, run: String) -> Result<(), MuError> {
        let (client, id) = (self.client.clone(), self.id.clone());
        spawn(&self.runtime, async move { client.append_shell(&id, &run).await }).await
    }

    pub async fn set_model(&self, name: String, effort: Option<String>) -> Result<(), MuError> {
        let (client, id) = (self.client.clone(), self.id.clone());
        let model = Model { name, effort, context: None };
        spawn(&self.runtime, async move { client.set_model(&id, model).await }).await
    }

    pub async fn stop(&self) -> Result<(), MuError> {
        let (client, id) = (self.client.clone(), self.id.clone());
        spawn(&self.runtime, async move { client.stop(&id).await }).await
    }

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
    Host { profile: HostProfile, sessions: Vec<SessionSummary> },
    Lost,
}

#[derive(uniffi::Record)]
pub struct HostProfile {
    pub name: String,
    pub home: String,
    pub icon: Option<String>,
    pub description: String,
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
    pub in_use: i64,
}

#[derive(uniffi::Record)]
pub struct Directory {
    pub path: String,
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
    Repository,
}

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

#[derive(uniffi::Enum)]
pub enum ShellUpdate {
    Output { tool: Tool },
    Done { run: String, tool: Tool },
    Failed { message: String },
}

#[uniffi::export(with_foreign)]
pub trait SessionListener: Send + Sync {
    fn on_update(&self, update: SessionUpdate);
}

// A reset clears everything but the first `keep` blocks of the transcript; the
// rest of it then arrives as ordinary commits.
#[derive(uniffi::Enum)]
pub enum SessionUpdate {
    Reset { keep: i32, cwd: String, state: SessionState, skills: Vec<Skill> },
    State { state: SessionState, started: bool },
    Committed { blocks: Vec<Block> },
    Live { index: i32, block: Block },
    LiveNotice { block: Block },
    Feedback { block: Block },
    Lost,
}

#[derive(uniffi::Record)]
pub struct SessionState {
    pub status: Status,
    pub title: String,
    pub model: String,
    pub effort: Option<String>,
    pub context: Option<i64>,
    pub context_used: i64,
    pub input: i64,
    pub uncached_input: i64,
    pub output: i64,
    pub queued: Vec<String>,
}

#[derive(uniffi::Record)]
pub struct Mention {
    pub start: u32,
    pub end: u32,
    pub query: String,
    pub entries: Vec<MentionEntry>,
}

#[derive(uniffi::Record)]
pub struct MentionEntry {
    pub text: String,
    pub label: String,
}

#[derive(uniffi::Record)]
pub struct Written {
    pub start: u32,
    pub end: u32,
    pub path: String,
    pub closed: bool,
}

#[derive(uniffi::Record)]
pub struct Resolved {
    pub path: String,
    pub file: String,
    pub kind: Attached,
}

#[derive(uniffi::Enum)]
pub enum Attached {
    Image,
    Text { lines: u32, head: String },
}

#[derive(uniffi::Record)]
pub struct ModelInfo {
    pub name: String,
    pub context: Option<i64>,
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
    pub error: Option<String>,
    pub unread: bool,
    // Milliseconds since the Unix epoch.
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
    pub tool: Option<Tool>,
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
    Shell,
    Notice,
    Warning,
}

#[derive(uniffi::Record)]
pub struct Tool {
    pub status: ToolStatus,
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
    Failed,
}
