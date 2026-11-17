use crate::{
    Result,
    bridge::Bridge,
    config,
    devices::Devices,
    host::{Host, Network},
    net,
    server::{Peer, handle},
};
use hyper::service::service_fn;
use hyper_util::rt::{TokioExecutor, TokioIo, TokioTimer};
use mu_wire::Status;
use std::{
    convert::Infallible,
    env, fs,
    net::SocketAddr,
    os::unix::{
        fs::{DirBuilderExt, OpenOptionsExt, PermissionsExt},
        net::UnixStream as StdStream,
    },
    path::{Path, PathBuf},
    pin::pin,
    sync::Arc,
    time::Duration,
};
use tokio::{
    io::{AsyncRead, AsyncWrite},
    net::{TcpListener, UnixListener, UnixStream},
    signal::unix::{SignalKind, signal},
    time,
};
use tokio_rustls::TlsAcceptor;

fn runtime_dir() -> PathBuf {
    match env::var_os("XDG_RUNTIME_DIR").filter(|dir| !dir.is_empty()) {
        Some(dir) => dir.into(),
        None => config::state_dir(),
    }
}

pub fn socket_path() -> PathBuf {
    match env::var_os("MU_SOCKET").filter(|path| !path.is_empty()) {
        Some(path) => path.into(),
        None => runtime_dir().join("mu.sock"),
    }
}

fn bind(socket: &Path) -> Result<UnixListener> {
    if let Some(dir) = socket.parent() {
        fs::DirBuilder::new().recursive(true).mode(0o700).create(dir)?;
    }
    let _ = fs::remove_file(socket);
    let listener = UnixListener::bind(socket)?;
    fs::set_permissions(socket, fs::Permissions::from_mode(0o600))?;
    Ok(listener)
}

async fn local(host: &Arc<Host>, accepted: std::io::Result<(UnixStream, tokio::net::unix::SocketAddr)>) {
    match accepted {
        Ok((stream, _)) => {
            let host = host.clone();
            tokio::spawn(async move { connection(&host, Peer::Local, stream).await });
        }
        Err(_) => tokio::time::sleep(Duration::from_millis(100)).await,
    }
}

pub fn listen_address() -> Result<Option<SocketAddr>> {
    let Some(address) = config::var("MU_LISTEN")? else {
        return Ok(None);
    };
    let parsed =
        address.parse().map_err(|_| format!("MU_LISTEN={address}: expected an address and port, like 0.0.0.0:7337"))?;
    Ok(Some(parsed))
}

async fn connection(host: &Arc<Host>, peer: Peer, stream: impl AsyncRead + AsyncWrite + Unpin + Send + 'static) {
    let host = host.clone();
    let service = service_fn(move |request| {
        let host = host.clone();
        async move { Ok::<_, Infallible>(handle(host, peer, request).await) }
    });
    let _ = hyper::server::conn::http2::Builder::new(TokioExecutor::new())
        .timer(TokioTimer::new())
        .keep_alive_interval(Duration::from_secs(30))
        .keep_alive_timeout(Duration::from_secs(20))
        .serve_connection(TokioIo::new(stream), service)
        .await;
}

async fn remote(host: &Arc<Host>, tls: TlsAcceptor, stream: tokio::net::TcpStream) {
    let _ = stream.set_nodelay(true);
    if let Ok(Ok(stream)) = time::timeout(Duration::from_secs(10), tls.accept(stream)).await {
        connection(host, Peer::Remote, stream).await;
    }
}

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

// What only this process uses: its special commands' links, and a private host's socket.
struct Own(PathBuf);

impl Own {
    fn new() -> Self {
        let dir = runtime_dir().join(format!("mu-{}", std::process::id()));
        // Left by a crashed process that had this pid.
        let _ = fs::remove_dir_all(&dir);
        Self(dir)
    }
}

impl Drop for Own {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

// `commands` are the special commands the host's bash runs get: names this binary answers to.
pub fn serve(socket: &Path, address: Option<SocketAddr>, commands: Vec<&'static str>) -> Result<()> {
    let runtime = tokio::runtime::Builder::new_multi_thread().worker_threads(2).enable_all().build()?;
    runtime.block_on(listen(socket, address, commands))
}

// The terminal's private host. Only its own bash runs' commands use its socket.
pub fn serve_stream(stream: StdStream, commands: Vec<&'static str>) -> Result<()> {
    let runtime = tokio::runtime::Builder::new_multi_thread().worker_threads(1).enable_all().build()?;
    runtime.block_on(async {
        stream.set_nonblocking(true)?;
        let own = Own::new();
        let socket = own.0.join("mu.sock");
        let listener = bind(&socket)?;
        let host = Host::new(None, None, Bridge::new(&socket, &own.0, commands)?)?;
        let mut terminal = pin!(connection(&host, Peer::Local, UnixStream::from_std(stream)?));
        loop {
            tokio::select! {
                () = &mut terminal => break,
                accepted = listener.accept() => local(&host, accepted).await,
            }
        }
        shutdown(&host).await;
        Ok(())
    })
}

async fn listen(socket: &Path, address: Option<SocketAddr>, commands: Vec<&'static str>) -> Result<()> {
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
    let listener = bind(socket)?;

    let devices = Devices::load(config::state_dir().join("devices.json"))?;
    let (network, tcp) = match address {
        Some(address) => {
            let identity = net::identity(&config::state_dir().join("tls"))?;
            let tcp = TcpListener::bind(address).await.map_err(|e| format!("MU_LISTEN={address}: {e}"))?;
            let bound = tcp.local_addr()?;
            eprintln!("Listening on {bound} for paired devices; `mu pair` pairs one");
            (Some(Network { bound, cert: identity.cert }), Some((tcp, TlsAcceptor::from(identity.tls))))
        }
        None => (None, None),
    };
    let own = Own::new();
    let host = Host::new(Some(devices), network, Bridge::new(socket, &own.0, commands)?)?;
    let mut terminate = signal(SignalKind::terminate())?;
    let mut interrupt = signal(SignalKind::interrupt())?;
    loop {
        tokio::select! {
            accepted = listener.accept() => local(&host, accepted).await,
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
