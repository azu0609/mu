// Listeners are called on the core's own threads; hop to the UI's before
// touching its state.
use mu_client::{
    AppendShell, Block, Call, Change, Client, CompleteMention, CreateSession, DeleteSessions, DeleteSnippet, Deleted,
    Directory, Endpoint, HostView, Invite, ListDirectory, ListModels, ListSkills, MarkRead, Mention, Model, ModelInfo,
    Profile, RecallQueued, ResolveMentions, Resolved, SaveSnippet, SendMessage, SessionSummary, SetDescription,
    SetIcon, SetModel, ShellEvent, Skill, State, Stop, ToolOutput, ToolStatus, Touched, Update, View, Written,
    hosts::{Hosts, Paired},
};
use std::{
    collections::HashMap,
    fmt,
    future::Future,
    path::PathBuf,
    sync::{Arc, Mutex},
};
use tokio::{
    runtime::{Handle, Runtime},
    task::JoinHandle,
};

uniffi::setup_scaffolding!();
uniffi::use_remote_type!(mu_wire::PathBuf);

#[uniffi::export]
pub fn version() -> String {
    env!("CARGO_PKG_VERSION").into()
}

#[uniffi::export]
pub fn compact(n: u64) -> String {
    mu_client::counts::compact(n)
}

#[uniffi::export]
pub fn mentions(text: String) -> Vec<Written> {
    mu_client::mentions(&text)
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

// Something the core keeps doing until this is dropped.
#[derive(uniffi::Object)]
pub struct Task(JoinHandle<()>);

impl Drop for Task {
    fn drop(&mut self) {
        self.0.abort();
    }
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

#[derive(Clone)]
struct Shared {
    runtime: Handle,
    hosts: Arc<Mutex<Hosts>>,
    clients: Arc<Mutex<HashMap<String, Client>>>,
}

#[derive(uniffi::Object)]
pub struct Core {
    shared: Shared,
    _runtime: Runtime,
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
        let shared =
            Shared { runtime: runtime.handle().clone(), hosts: Arc::new(Mutex::new(hosts)), clients: Arc::default() };
        Ok(Arc::new(Self { shared, _runtime: runtime }))
    }

    pub fn hosts(&self) -> Vec<Host> {
        self.shared.hosts.lock().unwrap().list().iter().map(Host::from).collect()
    }

    pub async fn pair(&self, invite: String, device: String) -> Result<Host, MuError> {
        let invite: Invite = invite.parse().map_err(MuError::Failed)?;
        let (remote, info) =
            spawn(&self.shared.runtime, async move { mu_client::pair(&invite, &device).await }).await?;
        let paired = Paired { name: info.name, icon: None, description: String::new(), remote };
        self.shared.hosts.lock().unwrap().add(paired.clone())?;
        // A connection made with an older token would keep presenting it.
        self.shared.clients.lock().unwrap().remove(&paired.remote.cert);
        Ok(Host::from(&paired))
    }

    pub fn forget(&self, host: String) -> Result<(), MuError> {
        self.shared.hosts.lock().unwrap().remove(&host)?;
        self.shared.clients.lock().unwrap().remove(&host);
        Ok(())
    }

    // The paired host with this id, to call; it connects when first called.
    pub fn remote(&self, host: String) -> Arc<Remote> {
        Arc::new(Remote { id: host, shared: self.shared.clone() })
    }
}

#[derive(uniffi::Object)]
pub struct Remote {
    id: String,
    shared: Shared,
}

impl Remote {
    fn connect(&self) -> Result<impl Future<Output = mu_client::Result<Client>> + use<>, MuError> {
        let Shared { hosts, clients, .. } = &self.shared;
        let cached = clients.lock().unwrap().get(&self.id).cloned();
        let hosts = hosts.lock().unwrap();
        let paired = hosts.get(&self.id).ok_or_else(|| MuError::Failed("This host is not paired".into()))?;
        let (remote, clients, id) = (paired.remote.clone(), clients.clone(), self.id.clone());
        Ok(async move {
            if let Some(client) = cached {
                return Ok(client);
            }
            let client = Client::connect(Endpoint::Remote(remote)).await?;
            clients.lock().unwrap().insert(id, client.clone());
            Ok(client)
        })
    }

    async fn on<T, F>(&self, run: impl FnOnce(Client) -> F + Send + 'static) -> Result<T, MuError>
    where
        T: Send + 'static,
        F: Future<Output = mu_client::Result<T>> + Send + 'static,
    {
        let connect = self.connect()?;
        spawn(&self.shared.runtime, async move { run(connect.await?).await }).await
    }

    async fn call<C: Call + Send + Sync + 'static>(&self, call: C) -> Result<C::Reply, MuError>
    where
        C::Reply: Send,
    {
        self.on(|client| async move { client.call(call).await }).await
    }
}

#[uniffi::export]
impl Remote {
    pub async fn watch(&self, listener: Arc<dyn HostListener>) -> Result<Arc<Task>, MuError> {
        let mut watch = self.on(|client| async move { client.watch().await }).await?;
        let (hosts, id) = (self.shared.hosts.clone(), self.id.clone());
        let task = self.shared.runtime.spawn(async move {
            let mut view = HostView::default();
            loop {
                let Update::Event(event) = watch.next().await else {
                    listener.on_update(HostUpdate::Lost);
                    continue;
                };
                view.apply(event);
                let profile = &view.profile;
                let _ = hosts.lock().unwrap().update(&id, |paired| {
                    paired.name = profile.name.clone();
                    paired.icon = profile.icon.clone();
                    paired.description = profile.description.clone();
                });
                listener.on_update(HostUpdate::Host { profile: profile.clone(), sessions: view.sessions.clone() });
            }
        });
        Ok(Arc::new(Task(task)))
    }

    // Starts the session with its first message.
    pub async fn create_session(
        &self,
        cwd: PathBuf,
        model: Model,
        text: String,
        skill: Option<String>,
    ) -> Result<String, MuError> {
        self.call(CreateSession { cwd, model, text, skill, parent: None }).await
    }

    pub async fn list_models(&self) -> Result<Vec<ModelInfo>, MuError> {
        self.call(ListModels {}).await
    }

    pub async fn delete_sessions(&self) -> Result<Deleted, MuError> {
        self.call(DeleteSessions {}).await
    }

    pub async fn set_icon(&self, icon: Option<String>) -> Result<(), MuError> {
        self.call(SetIcon { icon }).await
    }

    pub async fn set_description(&self, description: String) -> Result<(), MuError> {
        self.call(SetDescription { description }).await
    }

    pub async fn save_snippet(
        &self,
        id: Option<String>,
        name: String,
        command: String,
        icon: Option<String>,
    ) -> Result<String, MuError> {
        self.call(SaveSnippet { id, name, command, icon }).await
    }

    pub async fn delete_snippet(&self, id: String) -> Result<(), MuError> {
        self.call(DeleteSnippet { id }).await
    }

    pub async fn list_directory(&self, path: PathBuf) -> Result<Directory, MuError> {
        self.call(ListDirectory { path }).await
    }

    pub async fn list_skills(&self, cwd: PathBuf) -> Result<Vec<Skill>, MuError> {
        self.call(ListSkills { cwd }).await
    }

    pub async fn complete_mention(
        &self,
        cwd: PathBuf,
        text: String,
        cursor: u32,
        refresh: bool,
    ) -> Result<Option<Mention>, MuError> {
        self.call(CompleteMention { cwd, text, cursor, refresh }).await
    }

    pub async fn resolve_mentions(
        &self,
        cwd: PathBuf,
        text: String,
        skill: Option<String>,
    ) -> Result<Vec<Resolved>, MuError> {
        self.call(ResolveMentions { cwd, text, skill }).await
    }

    pub async fn read_image(&self, path: PathBuf) -> Result<Vec<u8>, MuError> {
        self.on(|client| async move { client.read_image(&path).await }).await
    }

    pub async fn upload(&self, name: String, data: Vec<u8>) -> Result<PathBuf, MuError> {
        self.on(|client| async move { client.upload(&name, &data).await }).await
    }

    pub async fn shell(
        &self,
        cwd: PathBuf,
        command: String,
        listener: Arc<dyn ShellListener>,
    ) -> Result<Arc<Task>, MuError> {
        let mut run = self.on(|client| async move { client.shell(&cwd, command).await }).await?;
        let task = self.shared.runtime.spawn(async move {
            loop {
                let update = match run.next().await {
                    Ok(Some(ShellEvent::Output(output))) => {
                        listener.on_update(ShellUpdate::Output { output });
                        continue;
                    }
                    Ok(Some(ShellEvent::Done { run, status, output })) => ShellUpdate::Done { run, status, output },
                    Ok(None) => ShellUpdate::Failed { message: "The host stopped sending the command's output".into() },
                    Err(e) => ShellUpdate::Failed { message: e.to_string() },
                };
                listener.on_update(update);
                return;
            }
        });
        Ok(Arc::new(Task(task)))
    }

    pub async fn open_session(
        &self,
        session: String,
        listener: Arc<dyn SessionListener>,
    ) -> Result<Arc<Session>, MuError> {
        let (connect, id) = (self.connect()?, session.clone());
        let (client, mut subscription) = spawn(&self.shared.runtime, async move {
            let client = connect.await?;
            let subscription = client.subscribe(&id).await?;
            Ok((client, subscription))
        })
        .await?;
        let task = self.shared.runtime.spawn(async move {
            let mut view = View::default();
            loop {
                let update = match subscription.next().await {
                    Update::Event(event) => {
                        let change = view.apply(event);
                        translate(&view, change)
                    }
                    Update::Lost => Some(SessionUpdate::Lost),
                };
                if let Some(update) = update {
                    listener.on_update(update);
                }
            }
        });
        Ok(Arc::new(Session { id: session, client, runtime: self.shared.runtime.clone(), _task: Task(task) }))
    }
}

fn translate(view: &View, change: Change) -> Option<SessionUpdate> {
    Some(match change {
        Change::Reset { keep } => SessionUpdate::Reset {
            keep: keep as u32,
            cwd: view.cwd.clone(),
            state: view.state.clone(),
            skills: view.skills.clone(),
        },
        Change::State { started } => SessionUpdate::State { state: view.state.clone(), started },
        Change::Committed { from } => SessionUpdate::Committed { blocks: view.blocks[from..].to_vec() },
        Change::Live(Touched::Block(index)) => {
            SessionUpdate::Live { index: index as u32, block: view.live.blocks.get(index)?.clone() }
        }
        Change::Live(Touched::Notice) => SessionUpdate::LiveNotice { block: view.live.notice.clone()? },
        Change::Feedback(block) => SessionUpdate::Feedback { block },
    })
}

#[derive(uniffi::Object)]
pub struct Session {
    id: String,
    client: Client,
    runtime: Handle,
    _task: Task,
}

impl Session {
    async fn call<C: Call + Send + Sync + 'static>(&self, call: C) -> Result<C::Reply, MuError>
    where
        C::Reply: Send,
    {
        let client = self.client.clone();
        spawn(&self.runtime, async move { client.call(call).await }).await
    }
}

#[uniffi::export]
impl Session {
    pub async fn send(&self, text: String, skill: Option<String>) -> Result<(), MuError> {
        self.call(SendMessage { session_id: self.id.clone(), text, skill, from: None }).await
    }

    pub async fn append_shell(&self, run: String) -> Result<(), MuError> {
        self.call(AppendShell { session_id: self.id.clone(), run }).await
    }

    pub async fn set_model(&self, model: Model) -> Result<(), MuError> {
        self.call(SetModel { session_id: self.id.clone(), model }).await
    }

    // `children` stops the sessions it parents too, and theirs.
    pub async fn stop(&self, children: bool) -> Result<(), MuError> {
        self.call(Stop { session_id: self.id.clone(), children }).await
    }

    pub async fn mark_read(&self) -> Result<(), MuError> {
        self.call(MarkRead { session_id: self.id.clone() }).await
    }

    // False when the message at `index` is no longer `text`, having been sent.
    pub async fn recall_queued(&self, index: u32, text: String) -> Result<bool, MuError> {
        self.call(RecallQueued { session_id: self.id.clone(), index: index as usize, text }).await
    }
}

#[uniffi::export(with_foreign)]
pub trait HostListener: Send + Sync {
    fn on_update(&self, update: HostUpdate);
}

// Each update is all of it.
#[derive(uniffi::Enum)]
pub enum HostUpdate {
    Host { profile: Profile, sessions: Vec<SessionSummary> },
    Lost,
}

#[uniffi::export(with_foreign)]
pub trait ShellListener: Send + Sync {
    fn on_update(&self, update: ShellUpdate);
}

#[derive(uniffi::Enum)]
pub enum ShellUpdate {
    Output { output: ToolOutput },
    Done { run: String, status: ToolStatus, output: ToolOutput },
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
    Reset { keep: u32, cwd: PathBuf, state: State, skills: Vec<Skill> },
    State { state: State, started: bool },
    Committed { blocks: Vec<Block> },
    Live { index: u32, block: Block },
    LiveNotice { block: Block },
    Feedback { block: Block },
    Lost,
}
