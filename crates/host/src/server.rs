// Serves the host's sessions over HTTP/2: each call is a POST of a JSON body,
// and a session's events stream back as newline-delimited JSON.
use crate::{
    Result,
    agent::{self, Agent},
    session,
};
use http_body_util::{BodyExt, Either, Full, Limited};
use hyper::{
    Method, Request, Response, StatusCode,
    body::{Body, Bytes, Frame, Incoming},
    header::{self, HeaderValue},
    service::service_fn,
};
use hyper_util::rt::{TokioExecutor, TokioIo};
use mu_wire::{self as wire, Block, Call, Event, Status};
use serde::{Serialize, de::DeserializeOwned};
use std::{
    collections::HashMap,
    convert::Infallible,
    env, fs, mem,
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

// Where a host listens for local clients.
pub fn socket_path() -> PathBuf {
    if let Some(path) = env::var_os("MU_SOCKET").filter(|path| !path.is_empty()) {
        return path.into();
    }
    match env::var_os("XDG_RUNTIME_DIR").filter(|dir| !dir.is_empty()) {
        Some(dir) => PathBuf::from(dir).join("mu.sock"),
        None => session::state_dir().join("mu.sock"),
    }
}

// A call that did not go through, with the status and message the client gets.
struct Failure(StatusCode, String);

type Rpc<T> = std::result::Result<T, Failure>;

fn failed(error: impl ToString) -> Failure {
    Failure(StatusCode::BAD_REQUEST, error.to_string())
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
        let agent = Agent::load(id).map_err(|e| Failure(StatusCode::NOT_FOUND, format!("Session {id}: {e}")))?;
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

    fn sessions(&self) -> Rpc<Vec<wire::SessionSummary>> {
        self.sweep();
        let loaded: HashMap<_, _> =
            self.agents.lock().unwrap().iter().map(|(id, agent)| (id.clone(), agent.status())).collect();
        Ok(session::sessions()
            .map_err(failed)?
            .into_iter()
            .map(|summary| wire::SessionSummary {
                status: loaded.get(&summary.id).copied().unwrap_or(Status::Idle),
                id: summary.id,
                title: summary.title,
                cwd: summary.cwd,
            })
            .collect())
    }

    fn create(&self, call: wire::CreateSession) -> Rpc<String> {
        if !call.cwd.is_absolute() || !call.cwd.is_dir() {
            return Err(failed(format!("{} is not a directory", call.cwd.display())));
        }
        let model = agent::default_model(call.model, call.effort, call.context).map_err(failed)?;
        let agent = Agent::new(call.cwd, model);
        let id = agent.id();
        self.agents.lock().unwrap().insert(id.clone(), agent);
        Ok(id)
    }

    fn subscribe(self: &Arc<Self>, id: &str) -> Rpc<Events> {
        let (tx, rx) = mpsc::channel(1024);
        let mut agents = self.agents.lock().unwrap();
        let agent = Self::load(&mut agents, id)?;
        // A subscriber that stops reading is dropped rather than buffered without
        // bound; its stream ends and it can subscribe again for a fresh snapshot.
        let subscription = agent.subscribe(move |event| {
            lines(event).is_some_and(|lines| lines.into_iter().all(|line| tx.try_send(line).is_ok()))
        });
        Ok(Events { rx, host: self.clone(), agent, subscription })
    }
}

// An event as lines on the wire, or none if it cannot be encoded (a path that
// is not UTF-8), which cuts the subscriber off. A commit can carry a whole
// transcript, a snapshot's, so it goes out in bounded chunks: no line grows
// with the session.
fn lines(event: &Event) -> Option<Vec<Bytes>> {
    const CHUNK: usize = 256 * 1024;
    let size = |block: &Block| block.text.len() + block.tool.as_ref().map_or(0, |tool| tool.output.text.len());
    let Event::Commit { blocks } = event else {
        return Some(vec![line(event)?]);
    };
    if blocks.iter().map(size).sum::<usize>() < CHUNK {
        return Some(vec![line(event)?]);
    }
    let mut lines = vec![];
    let mut chunk = vec![];
    let mut chunk_size = 0;
    for block in blocks {
        chunk_size += size(block);
        chunk.push(block.clone());
        if chunk_size >= CHUNK {
            lines.push(line(&Event::Commit { blocks: mem::take(&mut chunk) })?);
            chunk_size = 0;
        }
    }
    if !chunk.is_empty() {
        lines.push(line(&Event::Commit { blocks: chunk })?);
    }
    Some(lines)
}

fn line(event: &Event) -> Option<Bytes> {
    let mut line = serde_json::to_vec(event).ok()?;
    line.push(b'\n');
    Some(line.into())
}

// One subscriber's lines, until it leaves or falls too far behind.
struct Events {
    rx: mpsc::Receiver<Bytes>,
    host: Arc<Host>,
    agent: Arc<Agent>,
    subscription: u64,
}

impl Body for Events {
    type Data = Bytes;
    type Error = Infallible;

    fn poll_frame(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Option<std::result::Result<Frame<Bytes>, Infallible>>> {
        self.rx.poll_recv(cx).map(|line| line.map(|line| Ok(Frame::data(line))))
    }
}

impl Drop for Events {
    fn drop(&mut self) {
        self.agent.unsubscribe(self.subscription);
        self.host.sweep();
    }
}

type Reply = Response<Either<Full<Bytes>, Events>>;

fn reply(status: StatusCode, value: &impl Serialize) -> Reply {
    let body = match serde_json::to_vec(value) {
        Ok(body) => body,
        Err(e) => return failure(Failure(StatusCode::INTERNAL_SERVER_ERROR, e.to_string())),
    };
    let mut response = Response::new(Either::Left(Full::new(body.into())));
    *response.status_mut() = status;
    response.headers_mut().insert(header::CONTENT_TYPE, HeaderValue::from_static("application/json"));
    response
}

fn failure(Failure(status, message): Failure) -> Reply {
    reply(status, &wire::Failure { message })
}

fn stream(events: Events) -> Reply {
    let mut response = Response::new(Either::Right(events));
    response.headers_mut().insert(header::CONTENT_TYPE, HeaderValue::from_static("application/x-ndjson"));
    response
}

fn parse<T: DeserializeOwned>(body: &[u8]) -> Rpc<T> {
    // An empty body is an empty call, so calls without fields are easy to make by hand.
    let body = if body.is_empty() { b"{}".as_slice() } else { body };
    serde_json::from_slice(body).map_err(failed)
}

// The engine is synchronous (files, locks, processes); keep it off the reactor.
async fn run<T: Send + 'static>(host: &Arc<Host>, f: impl FnOnce(&Arc<Host>) -> Rpc<T> + Send + 'static) -> Rpc<T> {
    let host = host.clone();
    tokio::task::spawn_blocking(move || f(&host))
        .await
        .map_err(|e| Failure(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
}

async fn call<C: Call + Send + 'static>(
    host: &Arc<Host>,
    body: &[u8],
    f: impl FnOnce(&Arc<Host>, C) -> Rpc<C::Reply> + Send + 'static,
) -> Reply
where
    C::Reply: Send + 'static,
{
    let call = match parse(body) {
        Ok(call) => call,
        Err(e) => return failure(e),
    };
    match run(host, move |host| f(host, call)).await {
        Ok(value) => reply(StatusCode::OK, &value),
        Err(e) => failure(e),
    }
}

async fn handle(host: Arc<Host>, request: Request<Incoming>) -> Reply {
    if request.method() != Method::POST {
        return failure(Failure(StatusCode::METHOD_NOT_ALLOWED, "Calls are POSTs".into()));
    }
    let path = request.uri().path().to_owned();
    let body = match Limited::new(request.into_body(), 4 * 1024 * 1024).collect().await {
        Ok(body) => body.to_bytes(),
        Err(e) => return failure(Failure(StatusCode::PAYLOAD_TOO_LARGE, e.to_string())),
    };
    let host = &host;
    match path.as_str() {
        wire::Hello::PATH => {
            call(host, &body, |_, wire::Hello {}| {
                Ok(wire::HostInfo { protocol: wire::PROTOCOL, version: env!("CARGO_PKG_VERSION").into() })
            })
            .await
        }
        wire::ListSessions::PATH => call(host, &body, |host, wire::ListSessions {}| host.sessions()).await,
        wire::CreateSession::PATH => call(host, &body, |host, call| host.create(call)).await,
        wire::Subscribe::PATH => {
            let call: wire::Subscribe = match parse(&body) {
                Ok(call) => call,
                Err(e) => return failure(e),
            };
            match run(host, move |host| host.subscribe(&call.session_id)).await {
                Ok(events) => stream(events),
                Err(e) => failure(e),
            }
        }
        wire::SendMessage::PATH => {
            call(host, &body, |host, call: wire::SendMessage| {
                host.agent(&call.session_id)?.send(call.text, call.skill.as_deref()).map_err(failed)
            })
            .await
        }
        wire::Stop::PATH => {
            call(host, &body, |host, call: wire::Stop| {
                host.agent(&call.session_id)?.stop();
                Ok(())
            })
            .await
        }
        wire::RecallQueued::PATH => {
            call(host, &body, |host, call: wire::RecallQueued| Ok(host.agent(&call.session_id)?.recall())).await
        }
        wire::SetModel::PATH => {
            call(host, &body, |host, call: wire::SetModel| {
                host.agent(&call.session_id)?.set_model(call.model).map_err(failed)
            })
            .await
        }
        wire::GetTree::PATH => {
            call(host, &body, |host, call: wire::GetTree| Ok(host.agent(&call.session_id)?.tree())).await
        }
        wire::Checkout::PATH => {
            call(host, &body, |host, call: wire::Checkout| {
                host.agent(&call.session_id)?.checkout(call.node).map_err(failed)
            })
            .await
        }
        wire::CompleteMention::PATH => {
            call(host, &body, |host, call: wire::CompleteMention| {
                host.agent(&call.session_id)?.complete(&call.text, call.cursor, call.refresh).map_err(failed)
            })
            .await
        }
        _ => failure(Failure(StatusCode::NOT_FOUND, format!("No call at {path}"))),
    }
}

async fn connection(host: &Arc<Host>, stream: UnixStream) {
    let host = host.clone();
    let service = service_fn(move |request| {
        let host = host.clone();
        async move { Ok::<_, Infallible>(handle(host, request).await) }
    });
    let _ = hyper::server::conn::http2::Builder::new(TokioExecutor::new())
        .serve_connection(TokioIo::new(stream), service)
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
