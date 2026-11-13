mod convert;

use connectrpc::{
    ConnectError, ConnectRpcService, RequestContext, Response, Router, ServiceRequest, ServiceResult, ServiceStream,
};
use futures_core::Stream;
use hyper_util::{
    rt::{TokioExecutor, TokioIo},
    service::TowerToHyperService,
};
use mu_core::{
    Result,
    agent::{self, Agent, Status},
    session,
};
use mu_proto::{self as pb, SessionService};
use std::{
    collections::HashMap,
    fs,
    os::unix::{
        fs::{DirBuilderExt, OpenOptionsExt, PermissionsExt},
        net::UnixStream as StdStream,
    },
    path::{Path, PathBuf},
    pin::Pin,
    sync::{Arc, Mutex},
    task::{Context, Poll},
    time::Duration,
};
use tokio::{
    net::{UnixListener, UnixStream},
    signal::unix::{SignalKind, signal},
    sync::mpsc,
};

type Rpc<T> = std::result::Result<T, ConnectError>;

fn failed(error: impl ToString) -> ConnectError {
    ConnectError::failed_precondition(error.to_string())
}

// Sessions held in memory. Lock order: this map, then an agent.
#[derive(Default)]
struct Host {
    agents: Mutex<HashMap<String, Arc<Agent>>>,
}

impl Host {
    fn load(agents: &mut HashMap<String, Arc<Agent>>, id: &str) -> Rpc<Arc<Agent>> {
        if let Some(agent) = agents.get(id) {
            return Ok(agent.clone());
        }
        let agent = Agent::load(id).map_err(|e| ConnectError::not_found(format!("Session {id}: {e}")))?;
        agents.insert(id.into(), agent.clone());
        Ok(agent)
    }

    fn agent(&self, id: &str) -> Rpc<Arc<Agent>> {
        Self::load(&mut self.agents.lock().unwrap(), id)
    }

    // Sessions live on disk; memory only holds the ones in use.
    fn sweep(&self) {
        self.agents.lock().unwrap().retain(|_, agent| !agent.disposable());
    }

    fn subscribe(self: &Arc<Self>, id: &str) -> Rpc<Events> {
        let (tx, rx) = mpsc::channel(1024);
        let mut agents = self.agents.lock().unwrap();
        let agent = Self::load(&mut agents, id)?;
        // A subscriber that stops reading is dropped rather than buffered without
        // bound; its stream ends and it can subscribe again for a fresh snapshot.
        let subscription =
            agent.subscribe(move |event| convert::events(event).into_iter().all(|event| tx.try_send(event).is_ok()));
        Ok(Events { rx, host: self.clone(), agent, subscription })
    }
}

struct Events {
    rx: mpsc::Receiver<pb::SessionEvent>,
    host: Arc<Host>,
    agent: Arc<Agent>,
    subscription: u64,
}

impl Stream for Events {
    type Item = Rpc<pb::SessionEvent>;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        self.rx.poll_recv(cx).map(|event| event.map(Ok))
    }
}

impl Drop for Events {
    fn drop(&mut self) {
        self.agent.unsubscribe(self.subscription);
        self.host.sweep();
    }
}

struct Service(Arc<Host>);

impl Service {
    // The engine is synchronous (files, locks, processes); keep it off the reactor.
    async fn run<T: Send + 'static>(&self, f: impl FnOnce(&Arc<Host>) -> Rpc<T> + Send + 'static) -> Rpc<T> {
        let host = self.0.clone();
        tokio::task::spawn_blocking(move || f(&host)).await.map_err(|e| ConnectError::internal(e.to_string()))?
    }
}

// The trait spells its return types as `impl Encodable`; naming the message is the intended use.
#[allow(refining_impl_trait)]
impl SessionService for Service {
    async fn list_sessions(
        &self,
        _: RequestContext,
        _: ServiceRequest<'_, pb::ListSessionsRequest>,
    ) -> ServiceResult<pb::ListSessionsResponse> {
        let sessions = self
            .run(|host| {
                host.sweep();
                let loaded: HashMap<_, _> =
                    host.agents.lock().unwrap().iter().map(|(id, agent)| (id.clone(), agent.status())).collect();
                Ok(session::sessions()
                    .map_err(failed)?
                    .into_iter()
                    .map(|summary| pb::SessionSummary {
                        status: convert::status(loaded.get(&summary.id).copied().unwrap_or(Status::Idle)).into(),
                        id: summary.id,
                        title: summary.title,
                        cwd: summary.cwd.to_string_lossy().into_owned(),
                        ..Default::default()
                    })
                    .collect())
            })
            .await?;
        Response::ok(pb::ListSessionsResponse { sessions, ..Default::default() })
    }

    async fn create_session(
        &self,
        _: RequestContext,
        request: ServiceRequest<'_, pb::CreateSessionRequest>,
    ) -> ServiceResult<pb::CreateSessionResponse> {
        let request = request.to_owned_message();
        let id = self
            .run(move |host| {
                let cwd = PathBuf::from(request.cwd);
                if !cwd.is_absolute() || !cwd.is_dir() {
                    return Err(ConnectError::invalid_argument(format!("{} is not a directory", cwd.display())));
                }
                let model = agent::default_model(request.model, request.effort, request.context).map_err(failed)?;
                let agent = Agent::new(cwd, model);
                let id = agent.id();
                host.agents.lock().unwrap().insert(id.clone(), agent);
                Ok(id)
            })
            .await?;
        Response::ok(pb::CreateSessionResponse { id, ..Default::default() })
    }

    async fn subscribe(
        &self,
        _: RequestContext,
        request: ServiceRequest<'_, pb::SubscribeRequest>,
    ) -> ServiceResult<ServiceStream<pb::SessionEvent>> {
        let id = request.session_id.to_owned();
        Response::stream_ok(self.run(move |host| host.subscribe(&id)).await?)
    }

    async fn send(
        &self,
        _: RequestContext,
        request: ServiceRequest<'_, pb::SendRequest>,
    ) -> ServiceResult<pb::SendResponse> {
        let request = request.to_owned_message();
        self.run(move |host| {
            host.agent(&request.session_id)?.send(request.text, request.skill.as_deref()).map_err(failed)
        })
        .await?;
        Response::ok(pb::SendResponse::default())
    }

    async fn stop(
        &self,
        _: RequestContext,
        request: ServiceRequest<'_, pb::StopRequest>,
    ) -> ServiceResult<pb::StopResponse> {
        let id = request.session_id.to_owned();
        self.run(move |host| {
            host.agent(&id)?.stop();
            Ok(())
        })
        .await?;
        Response::ok(pb::StopResponse::default())
    }

    async fn recall_queued(
        &self,
        _: RequestContext,
        request: ServiceRequest<'_, pb::RecallQueuedRequest>,
    ) -> ServiceResult<pb::RecallQueuedResponse> {
        let id = request.session_id.to_owned();
        let text = self.run(move |host| Ok(host.agent(&id)?.recall())).await?;
        Response::ok(pb::RecallQueuedResponse { text, ..Default::default() })
    }

    async fn set_model(
        &self,
        _: RequestContext,
        request: ServiceRequest<'_, pb::SetModelRequest>,
    ) -> ServiceResult<pb::SetModelResponse> {
        let request = request.to_owned_message();
        self.run(move |host| {
            let model = request.model.into_option().ok_or_else(|| ConnectError::invalid_argument("Missing model"))?;
            host.agent(&request.session_id)?
                .set_model(session::Model { name: model.name, effort: model.effort, context: model.context })
                .map_err(failed)
        })
        .await?;
        Response::ok(pb::SetModelResponse::default())
    }

    async fn get_tree(
        &self,
        _: RequestContext,
        request: ServiceRequest<'_, pb::GetTreeRequest>,
    ) -> ServiceResult<pb::GetTreeResponse> {
        let id = request.session_id.to_owned();
        let entries = self.run(move |host| Ok(host.agent(&id)?.tree())).await?;
        Response::ok(pb::GetTreeResponse {
            entries: entries
                .into_iter()
                .map(|entry| pb::TreeEntry {
                    node: entry.node.map(|node| node as u64),
                    label: entry.label,
                    current: entry.current,
                    ..Default::default()
                })
                .collect(),
            ..Default::default()
        })
    }

    async fn checkout(
        &self,
        _: RequestContext,
        request: ServiceRequest<'_, pb::CheckoutRequest>,
    ) -> ServiceResult<pb::CheckoutResponse> {
        let request = request.to_owned_message();
        let draft = self
            .run(move |host| {
                let node = request.node.map(|node| usize::try_from(node).unwrap_or(usize::MAX));
                host.agent(&request.session_id)?.checkout(node).map_err(failed)
            })
            .await?;
        Response::ok(pb::CheckoutResponse { draft, ..Default::default() })
    }

    async fn complete_mention(
        &self,
        _: RequestContext,
        request: ServiceRequest<'_, pb::CompleteMentionRequest>,
    ) -> ServiceResult<pb::CompleteMentionResponse> {
        let request = request.to_owned_message();
        let menu = self
            .run(move |host| {
                host.agent(&request.session_id)?
                    .complete(&request.text, request.cursor as usize, request.refresh)
                    .map_err(failed)
            })
            .await?;
        Response::ok(pb::CompleteMentionResponse {
            mention: menu
                .map(|menu| pb::Mention {
                    start: menu.range.start as u32,
                    end: menu.range.end as u32,
                    query: menu.query,
                    exact: menu.exact_file,
                    entries: menu
                        .entries
                        .into_iter()
                        .map(|entry| pb::MentionEntry { text: entry.text, label: entry.label, ..Default::default() })
                        .collect(),
                    ..Default::default()
                })
                .into(),
            ..Default::default()
        })
    }
}

async fn connection(host: &Arc<Host>, stream: UnixStream) {
    let service = ConnectRpcService::new(Router::new().add_service(Arc::new(Service(host.clone()))));
    let _ = hyper::server::conn::http2::Builder::new(TokioExecutor::new())
        .serve_connection(TokioIo::new(stream), TowerToHyperService::new(service))
        .await;
}

// Stopping kills each tool's process group; give the steps a moment to record that.
async fn shutdown(host: &Host) {
    let agents: Vec<_> = host.agents.lock().unwrap().values().cloned().collect();
    for agent in &agents {
        agent.stop();
    }
    for _ in 0..40 {
        if agents.iter().all(|agent| agent.status() != Status::Running) {
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

// A shared host: serves every client of the socket until SIGINT or SIGTERM.
// The socket is the only way in, and only for this user.
pub fn serve(socket: &Path) -> Result<()> {
    let runtime = tokio::runtime::Builder::new_multi_thread().worker_threads(2).enable_all().build()?;
    runtime.block_on(listen(socket))
}

// A private host: serves the one client holding the other end of `stream`,
// and stops everything it was running when that client goes away.
pub fn serve_stream(stream: StdStream) -> Result<()> {
    let runtime = tokio::runtime::Builder::new_multi_thread().worker_threads(1).enable_all().build()?;
    runtime.block_on(async {
        stream.set_nonblocking(true)?;
        let host = Arc::new(Host::default());
        connection(&host, UnixStream::from_std(stream)?).await;
        shutdown(&host).await;
        Ok(())
    })
}

async fn listen(socket: &Path) -> Result<()> {
    if let Some(dir) = socket.parent() {
        fs::DirBuilder::new().recursive(true).mode(0o700).create(dir)?;
    }
    // The lock, not the socket file, says whether a server is alive: a stale
    // socket can be replaced, and two starting servers cannot both win.
    let lock = fs::OpenOptions::new().write(true).create(true).truncate(false).mode(0o600).open(lock_path(socket))?;
    match lock.try_lock() {
        Ok(()) => (),
        Err(fs::TryLockError::WouldBlock) => {
            return Err(format!("A mu server is already serving {}", socket.display()).into());
        }
        Err(fs::TryLockError::Error(e)) => return Err(e.into()),
    }
    let _ = fs::remove_file(socket);
    let listener = UnixListener::bind(socket)?;
    fs::set_permissions(socket, fs::Permissions::from_mode(0o600))?;

    let host = Arc::new(Host::default());
    let mut terminate = signal(SignalKind::terminate())?;
    let mut interrupt = signal(SignalKind::interrupt())?;
    loop {
        tokio::select! {
            accepted = listener.accept() => match accepted {
                Ok((stream, _)) => {
                    let host = host.clone();
                    tokio::spawn(async move { connection(&host, stream).await });
                }
                // Out of descriptors, most likely; existing connections keep working.
                Err(_) => tokio::time::sleep(Duration::from_millis(100)).await,
            },
            _ = terminate.recv() => break,
            _ = interrupt.recv() => break,
        }
    }
    let _ = fs::remove_file(socket);
    shutdown(&host).await;
    Ok(())
}

fn lock_path(socket: &Path) -> PathBuf {
    let mut path = socket.as_os_str().to_owned();
    path.push(".lock");
    path.into()
}
