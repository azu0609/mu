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
    os::unix::fs::{DirBuilderExt, OpenOptionsExt, PermissionsExt},
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};
use tokio::{
    io::{AsyncRead, AsyncWrite},
    net::{TcpListener, UnixListener},
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

// `commands` are the special commands the host's bash runs get: names this binary answers to.
pub fn serve(socket: &Path, address: Option<SocketAddr>, commands: Vec<&'static str>) -> Result<()> {
    let runtime = tokio::runtime::Builder::new_multi_thread().worker_threads(2).enable_all().build()?;
    runtime.block_on(listen(socket, address, commands))
}

async fn listen(socket: &Path, address: Option<SocketAddr>, commands: Vec<&'static str>) -> Result<()> {
    if let Some(dir) = socket.parent() {
        fs::DirBuilder::new().recursive(true).mode(0o700).create(dir)?;
    }
    // The lock, not the socket file, says whether a server is alive: a stale
    // socket can be replaced, and two starting servers cannot both win.
    let lock =
        fs::OpenOptions::new().write(true).create(true).truncate(false).mode(0o600).open(beside(socket, ".lock"))?;
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
    let bin = beside(socket, ".bin");
    let host = Host::new(devices, network, Bridge::new(&bin, commands)?)?;
    let mut terminate = signal(SignalKind::terminate())?;
    let mut interrupt = signal(SignalKind::interrupt())?;
    loop {
        tokio::select! {
            accepted = listener.accept() => match accepted {
                Ok((stream, _)) => {
                    let host = host.clone();
                    tokio::spawn(async move { connection(&host, Peer::Local, stream).await });
                }
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
    let _ = fs::remove_dir_all(&bin);
    Ok(())
}

async fn accept(tcp: &Option<(TcpListener, TlsAcceptor)>) -> std::io::Result<tokio::net::TcpStream> {
    match tcp {
        Some((listener, _)) => Ok(listener.accept().await?.0),
        None => std::future::pending().await,
    }
}

fn beside(socket: &Path, suffix: &str) -> PathBuf {
    let mut path = socket.as_os_str().to_owned();
    path.push(suffix);
    path.into()
}
