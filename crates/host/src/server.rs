// Serves the host's sessions over HTTP/2: each call is a POST of a JSON body,
// and a session's events stream back as newline-delimited JSON. Local clients
// come through the unix socket; paired devices through TLS on `MU_LISTEN`.
use crate::{
    Result,
    agent::{self, Agent},
    config,
    devices::Devices,
    net, session,
};
use http_body_util::{BodyExt, Either, Full, Limited};
use hyper::{
    Method, Request, Response, StatusCode,
    body::{Body, Bytes, Frame, Incoming},
    header::{self, HeaderValue},
    service::service_fn,
};
use hyper_util::rt::{TokioExecutor, TokioIo, TokioTimer};
use mu_wire::{self as wire, Block, Call, Event, Status, WatchEvent};
use serde::{Serialize, de::DeserializeOwned};
use std::{
    collections::HashMap,
    convert::Infallible,
    env, fs, mem,
    net::SocketAddr,
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
    io::{AsyncRead, AsyncWrite},
    net::{TcpListener, UnixListener, UnixStream},
    signal::unix::{SignalKind, signal},
    sync::{broadcast, mpsc},
    time,
};
use tokio_rustls::TlsAcceptor;

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

// Where a shared host also listens for paired devices, if anywhere.
pub fn listen_address() -> Result<Option<SocketAddr>> {
    let Some(address) = config::var("MU_LISTEN")? else {
        return Ok(None);
    };
    let parsed =
        address.parse().map_err(|_| format!("MU_LISTEN={address}: expected an address and port, like 0.0.0.0:7337"))?;
    Ok(Some(parsed))
}

// Who is on the other end of a connection.
#[derive(Clone, Copy, PartialEq)]
enum Peer {
    // Through the unix socket, or the private host's socket pair: this user.
    Local,
    // Over the network: a paired device, once its token checks out.
    Remote,
}

// How a host is reachable over the network.
struct Network {
    bound: SocketAddr,
    cert: String,
}

// A call that did not go through, with the status and message the client gets.
struct Failure(StatusCode, String);

type Rpc<T> = std::result::Result<T, Failure>;

fn failed(error: impl ToString) -> Failure {
    Failure(StatusCode::BAD_REQUEST, error.to_string())
}

fn internal(error: impl ToString) -> Failure {
    Failure(StatusCode::INTERNAL_SERVER_ERROR, error.to_string())
}

// Sessions held in memory. Lock order: this map, then an agent.
struct Host {
    agents: Mutex<HashMap<String, Arc<Agent>>>,
    // Ids of sessions whose summaries may have changed, for session lists.
    changes: broadcast::Sender<String>,
    name: String,
    // A private host has neither.
    devices: Option<Devices>,
    network: Option<Network>,
}

impl Host {
    fn new(devices: Option<Devices>, network: Option<Network>) -> Arc<Self> {
        let changes = broadcast::channel(256).0;
        Arc::new(Self { agents: Mutex::default(), changes, name: net::hostname(), devices, network })
    }

    fn changed(&self) -> agent::Changed {
        let changes = self.changes.clone();
        Arc::new(move |id| {
            let _ = changes.send(id.to_owned());
        })
    }

    fn devices(&self) -> Rpc<&Devices> {
        self.devices
            .as_ref()
            .ok_or_else(|| failed("This host is private to one terminal; run `mu serve` to pair devices"))
    }

    fn invite(&self) -> Rpc<wire::Invitation> {
        let devices = self.devices()?;
        let network = self.network.as_ref().ok_or_else(|| {
            failed("This host does not listen on the network; set MU_LISTEN, like MU_LISTEN=0.0.0.0:7337")
        })?;
        Ok(wire::Invitation {
            code: devices.invite().map_err(internal)?,
            cert: network.cert.clone(),
            port: network.bound.port(),
            addresses: net::addresses(network.bound.ip()),
        })
    }

    fn pair(&self, call: wire::Pair) -> Rpc<String> {
        self.devices()?.pair(&call.code, &call.device).map_err(internal)?.ok_or_else(|| {
            Failure(StatusCode::FORBIDDEN, "This invite was already used or has expired; make a new one".into())
        })
    }

    fn unpair(&self, call: wire::Unpair) -> Rpc<()> {
        match self.devices()?.unpair(&call.device_id).map_err(internal)? {
            true => Ok(()),
            false => Err(Failure(StatusCode::NOT_FOUND, format!("No device {}", call.device_id))),
        }
    }

    fn load(&self, agents: &mut HashMap<String, Arc<Agent>>, id: &str) -> Rpc<Arc<Agent>> {
        if let Some(agent) = agents.get(id) {
            return Ok(agent.clone());
        }
        let agent = Agent::load(id, self.changed())
            .map_err(|e| Failure(StatusCode::NOT_FOUND, format!("Session {id}: {e}")))?;
        agents.insert(id.into(), agent.clone());
        Ok(agent)
    }

    fn agent(&self, id: &str) -> Rpc<Arc<Agent>> {
        self.load(&mut self.agents.lock().unwrap(), id)
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
            .map(|summary| {
                let status = loaded.get(&summary.id).copied();
                summarize(summary, status)
            })
            .collect())
    }

    // None until the session is saved, which its first message does.
    fn summary(&self, id: &str) -> Option<wire::SessionSummary> {
        let status = self.agents.lock().unwrap().get(id).map(|agent| agent.status());
        Some(summarize(session::summary_of(id)?, status))
    }

    fn create(&self, call: wire::CreateSession) -> Rpc<String> {
        if !call.cwd.is_absolute() || !call.cwd.is_dir() {
            return Err(failed(format!("{} is not a directory", call.cwd.display())));
        }
        let model = agent::default_model(call.model, call.effort, call.context).map_err(failed)?;
        let agent = Agent::new(call.cwd, model, self.changed());
        let id = agent.id();
        self.agents.lock().unwrap().insert(id.clone(), agent);
        Ok(id)
    }

    fn subscribe(self: &Arc<Self>, call: wire::Subscribe) -> Rpc<Stream> {
        let (tx, rx) = mpsc::channel(1024);
        let mut agents = self.agents.lock().unwrap();
        let agent = self.load(&mut agents, &call.session_id)?;
        // A subscriber that stops reading is dropped rather than buffered without
        // bound; its stream ends and it can subscribe again, resuming.
        let subscription = agent.subscribe(call.resume, move |event| {
            lines(event).is_some_and(|lines| lines.into_iter().all(|line| tx.try_send(line).is_ok()))
        });
        Ok(Stream { rx, subscription: Some((self.clone(), agent, subscription)) })
    }
}

// A session in memory knows its status best; the log only knows how runs ended.
// Running sessions are not unread yet: nobody could have seen their last step.
fn summarize(summary: session::Summary, loaded: Option<Status>) -> wire::SessionSummary {
    let status = loaded.unwrap_or(summary.status);
    wire::SessionSummary {
        unread: summary.unread && status != Status::Running,
        updated: summary.updated,
        error: summary.error.filter(|_| status == Status::Failed),
        status,
        id: summary.id,
        title: summary.title,
        cwd: summary.cwd,
    }
}

// The session list, then each session again whenever it changes.
async fn watch(host: &Arc<Host>) -> Reply {
    // Listening before listing, so no change falls in between.
    let mut changes = host.changes.subscribe();
    let sessions = match run(host, |host| host.sessions()).await {
        Ok(sessions) => sessions,
        Err(e) => return failure(e),
    };
    let (tx, rx) = mpsc::channel(64);
    let Some(first) = line(&WatchEvent::Sessions { sessions }) else {
        return failure(internal("A session path is not UTF-8"));
    };
    let _ = tx.try_send(first);
    let host = host.clone();
    tokio::spawn(async move {
        loop {
            let event = tokio::select! {
                _ = tx.closed() => return,
                change = changes.recv() => match change {
                    Ok(id) => match run(&host, move |host| Ok(host.summary(&id))).await {
                        Ok(Some(summary)) => WatchEvent::Session(summary),
                        _ => continue,
                    },
                    // Some changes were missed; start over from the whole list.
                    Err(broadcast::error::RecvError::Lagged(_)) => match run(&host, |host| host.sessions()).await {
                        Ok(sessions) => WatchEvent::Sessions { sessions },
                        Err(_) => return,
                    },
                    Err(broadcast::error::RecvError::Closed) => return,
                },
            };
            let Some(line) = line(&event) else { return };
            if tx.send(line).await.is_err() {
                return;
            }
        }
    });
    stream(Stream { rx, subscription: None })
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

fn line(value: &impl Serialize) -> Option<Bytes> {
    let mut line = serde_json::to_vec(value).ok()?;
    line.push(b'\n');
    Some(line.into())
}

// Lines for one client, until it leaves or falls too far behind.
struct Stream {
    rx: mpsc::Receiver<Bytes>,
    // For a session's events: what to unsubscribe from when the client leaves.
    subscription: Option<(Arc<Host>, Arc<Agent>, u64)>,
}

impl Body for Stream {
    type Data = Bytes;
    type Error = Infallible;

    fn poll_frame(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Option<std::result::Result<Frame<Bytes>, Infallible>>> {
        self.rx.poll_recv(cx).map(|line| line.map(|line| Ok(Frame::data(line))))
    }
}

impl Drop for Stream {
    fn drop(&mut self) {
        if let Some((host, agent, subscription)) = self.subscription.take() {
            agent.unsubscribe(subscription);
            host.sweep();
        }
    }
}

type Reply = Response<Either<Full<Bytes>, Stream>>;

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

fn stream(lines: Stream) -> Reply {
    let mut response = Response::new(Either::Right(lines));
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

// Over the network, everything but saying hello and pairing takes a paired
// device's token, and managing devices takes a local client.
fn admit(host: &Host, peer: Peer, request: &Request<Incoming>) -> Rpc<()> {
    let path = request.uri().path();
    if peer == Peer::Local || path == wire::Hello::PATH || path == wire::Pair::PATH {
        return Ok(());
    }
    let token = request
        .headers()
        .get(header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.strip_prefix("Bearer "));
    if !token.is_some_and(|token| host.devices.as_ref().is_some_and(|devices| devices.authorized(token))) {
        return Err(Failure(StatusCode::UNAUTHORIZED, format!("This device is not paired with {}", host.name)));
    }
    if [wire::CreateInvite::PATH, wire::ListDevices::PATH, wire::Unpair::PATH].contains(&path) {
        return Err(Failure(StatusCode::FORBIDDEN, "Only local clients can manage devices".into()));
    }
    Ok(())
}

async fn handle(host: Arc<Host>, peer: Peer, request: Request<Incoming>) -> Reply {
    if request.method() != Method::POST {
        return failure(Failure(StatusCode::METHOD_NOT_ALLOWED, "Calls are POSTs".into()));
    }
    if let Err(e) = admit(&host, peer, &request) {
        return failure(e);
    }
    let path = request.uri().path().to_owned();
    let body = match Limited::new(request.into_body(), 4 * 1024 * 1024).collect().await {
        Ok(body) => body.to_bytes(),
        Err(e) => return failure(Failure(StatusCode::PAYLOAD_TOO_LARGE, e.to_string())),
    };
    let host = &host;
    match path.as_str() {
        wire::Hello::PATH => {
            call(host, &body, |host, wire::Hello {}| {
                Ok(wire::HostInfo {
                    protocol: wire::PROTOCOL,
                    version: env!("CARGO_PKG_VERSION").into(),
                    name: host.name.clone(),
                })
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
            match run(host, move |host| host.subscribe(call)).await {
                Ok(lines) => stream(lines),
                Err(e) => failure(e),
            }
        }
        wire::Watch::PATH => watch(host).await,
        wire::MarkRead::PATH => {
            call(host, &body, |host, call: wire::MarkRead| host.agent(&call.session_id)?.mark_read().map_err(internal))
                .await
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
        wire::Pair::PATH => call(host, &body, |host, call| host.pair(call)).await,
        wire::CreateInvite::PATH => call(host, &body, |host, wire::CreateInvite {}| host.invite()).await,
        wire::ListDevices::PATH => call(host, &body, |host, wire::ListDevices {}| Ok(host.devices()?.list())).await,
        wire::Unpair::PATH => call(host, &body, |host, call| host.unpair(call)).await,
        _ => failure(Failure(StatusCode::NOT_FOUND, format!("No call at {path}"))),
    }
}

async fn connection(host: &Arc<Host>, peer: Peer, stream: impl AsyncRead + AsyncWrite + Unpin + Send + 'static) {
    let host = host.clone();
    let service = service_fn(move |request| {
        let host = host.clone();
        async move { Ok::<_, Infallible>(handle(host, peer, request).await) }
    });
    // Pings drop clients that vanished without a word, like a phone that lost its network.
    let _ = hyper::server::conn::http2::Builder::new(TokioExecutor::new())
        .timer(TokioTimer::new())
        .keep_alive_interval(Duration::from_secs(30))
        .keep_alive_timeout(Duration::from_secs(20))
        .serve_connection(TokioIo::new(stream), service)
        .await;
}

// A device's connection, once it gets through TLS. Pairing happens later, per call.
async fn remote(host: &Arc<Host>, tls: TlsAcceptor, stream: tokio::net::TcpStream) {
    let _ = stream.set_nodelay(true);
    if let Ok(Ok(stream)) = time::timeout(Duration::from_secs(10), tls.accept(stream)).await {
        connection(host, Peer::Remote, stream).await;
    }
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

// A shared host: serves every client of the socket, and of `address` if given,
// until SIGINT or SIGTERM. The socket is only open to this user; the network
// only to paired devices.
pub fn serve(socket: &Path, address: Option<SocketAddr>) -> Result<()> {
    let runtime = tokio::runtime::Builder::new_multi_thread().worker_threads(2).enable_all().build()?;
    runtime.block_on(listen(socket, address))
}

// A private host: serves the one client holding the other end of `stream`,
// and stops everything it was running when that client goes away.
pub fn serve_stream(stream: StdStream) -> Result<()> {
    let runtime = tokio::runtime::Builder::new_multi_thread().worker_threads(1).enable_all().build()?;
    runtime.block_on(async {
        stream.set_nonblocking(true)?;
        let host = Host::new(None, None);
        connection(&host, Peer::Local, UnixStream::from_std(stream)?).await;
        shutdown(&host).await;
        Ok(())
    })
}

async fn listen(socket: &Path, address: Option<SocketAddr>) -> Result<()> {
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

    let devices = Devices::load(session::state_dir().join("devices.json"))?;
    let (network, tcp) = match address {
        Some(address) => {
            let identity = net::identity(&session::state_dir().join("tls"))?;
            let tcp = TcpListener::bind(address).await.map_err(|e| format!("MU_LISTEN={address}: {e}"))?;
            let bound = tcp.local_addr()?;
            eprintln!("Listening on {bound} for paired devices; `mu pair` pairs one");
            (Some(Network { bound, cert: identity.cert }), Some((tcp, TlsAcceptor::from(identity.tls))))
        }
        None => (None, None),
    };
    let host = Host::new(Some(devices), network);
    let mut terminate = signal(SignalKind::terminate())?;
    let mut interrupt = signal(SignalKind::interrupt())?;
    loop {
        tokio::select! {
            accepted = listener.accept() => match accepted {
                Ok((stream, _)) => {
                    let host = host.clone();
                    tokio::spawn(async move { connection(&host, Peer::Local, stream).await });
                }
                // Out of descriptors, most likely; existing connections keep working.
                Err(_) => tokio::time::sleep(Duration::from_millis(100)).await,
            },
            accepted = accept(&tcp) => match accepted {
                Ok(stream) => {
                    let host = host.clone();
                    let tls = tcp.as_ref().map(|(_, tls)| tls.clone()).expect("listening");
                    tokio::spawn(async move { remote(&host, tls, stream).await });
                }
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

// The next device to connect, or never when not on the network.
async fn accept(tcp: &Option<(TcpListener, TlsAcceptor)>) -> std::io::Result<tokio::net::TcpStream> {
    match tcp {
        Some((listener, _)) => Ok(listener.accept().await?.0),
        None => std::future::pending().await,
    }
}

fn lock_path(socket: &Path) -> PathBuf {
    let mut path = socket.as_os_str().to_owned();
    path.push(".lock");
    path.into()
}
