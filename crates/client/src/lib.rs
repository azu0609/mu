pub mod counts;
pub mod hosts;
mod tls;
mod view;

use base64::{Engine, engine::general_purpose::STANDARD};
use http_body_util::{BodyExt, Full, Limited};
use hyper::{
    Request, StatusCode,
    body::{Bytes, Incoming},
    client::conn::http2::SendRequest,
    header::{self, HeaderValue},
};
use hyper_util::rt::{TokioExecutor, TokioIo, TokioTimer};
use percent_encoding::{NON_ALPHANUMERIC, utf8_percent_encode};
use rustls::pki_types::ServerName;
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use std::{
    fmt, io,
    marker::PhantomData,
    path::{Path, PathBuf},
    pin::{Pin, pin},
    sync::{
        self, Arc,
        atomic::{AtomicBool, Ordering},
    },
    task::{Context, Poll, Waker},
    time::Duration,
};
use tokio::{
    io::{AsyncRead, AsyncWrite, ReadBuf},
    net::{TcpStream, UnixStream},
    sync::{Mutex, Notify},
    time::{self, Instant},
};
use tokio_rustls::TlsConnector;

pub use mu_wire::*;
pub use view::{Change, HostView, PathNode, View};

// The host accepts calls up to 4 MiB, and a user message near that must still
// fit when it comes back as an event.
const LIMIT: usize = 64 * 1024 * 1024;

const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);

const PROBE_TIMEOUT: Duration = Duration::from_secs(4);

#[derive(Clone)]
pub enum Error {
    Unpaired(String),
    Other(String),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            Self::Unpaired(message) | Self::Other(message) => f.write_str(message),
        }
    }
}

// Just the message, as a CLI shows an error it returns from main.
impl fmt::Debug for Error {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        fmt::Debug::fmt(&self.to_string(), f)
    }
}

impl std::error::Error for Error {}

impl From<String> for Error {
    fn from(message: String) -> Self {
        Self::Other(message)
    }
}

impl From<&str> for Error {
    fn from(message: &str) -> Self {
        Self::Other(message.into())
    }
}

macro_rules! error_from {
    ($($type:ty),*) => {$(
        impl From<$type> for Error {
            fn from(error: $type) -> Self {
                Self::Other(error.to_string())
            }
        }
    )*};
}

error_from!(io::Error, serde_json::Error, rustls::Error, Box<dyn std::error::Error + Send + Sync>);

impl From<hyper::Error> for Error {
    fn from(error: hyper::Error) -> Self {
        Self::Other(if error.is_timeout() {
            "The mu host stopped answering".into()
        } else if error.is_user() {
            error.to_string()
        } else {
            "Lost the connection to the mu host".into()
        })
    }
}

pub type Result<T> = std::result::Result<T, Error>;

pub enum Endpoint {
    Socket(PathBuf),
    Remote(Remote),
}

#[derive(Clone, PartialEq, Serialize, Deserialize)]
pub struct Remote {
    pub address: String,
    pub cert: String,
    pub token: Option<String>,
}

enum Target {
    Socket(PathBuf),
    Remote { address: String, tls: TlsConnector },
}

#[derive(Clone)]
pub struct Client(Arc<Connection>);

struct Connection {
    target: Target,
    authorization: Option<HeaderValue>,
    link: Mutex<Link>,
    host: sync::Mutex<Option<HostInfo>>,
    nudged: AtomicBool,
    retry: Notify,
}

#[derive(Default)]
struct Link {
    sender: Option<SendRequest<Full<Bytes>>>,
    socket: Option<Arc<Cut>>,
    failures: u32,
    cooldown: Option<(Instant, Error)>,
}

impl Link {
    fn live(&self) -> Option<SendRequest<Full<Bytes>>> {
        self.sender.clone().filter(|sender| !sender.is_closed())
    }

    fn hang_up(&mut self) {
        self.sender = None;
        if let Some(socket) = self.socket.take() {
            socket.cut();
        }
    }
}

fn backoff(failures: u32) -> Duration {
    match failures {
        0 | 1 => Duration::ZERO,
        n => Duration::from_secs(1 << (n - 2).min(3)),
    }
}

impl Client {
    pub fn new(endpoint: Endpoint) -> Result<Self> {
        let (target, authorization) = match endpoint {
            Endpoint::Socket(path) => (Target::Socket(path), None),
            Endpoint::Remote(remote) => {
                let authorization = match remote.token {
                    Some(token) => Some(
                        HeaderValue::from_str(&format!("Bearer {token}"))
                            .map_err(|_| "This device's token is invalid")?,
                    ),
                    None => None,
                };
                let tls = TlsConnector::from(tls::config(&remote.cert)?);
                (Target::Remote { address: remote.address, tls }, authorization)
            }
        };
        Ok(Self(Arc::new(Connection {
            target,
            authorization,
            link: Mutex::default(),
            host: sync::Mutex::new(None),
            nudged: AtomicBool::new(false),
            retry: Notify::new(),
        })))
    }

    pub async fn connect(endpoint: Endpoint) -> Result<Self> {
        let client = Self::new(endpoint)?;
        client.sender().await?;
        Ok(client)
    }

    pub fn host(&self) -> Option<HostInfo> {
        self.0.host.lock().unwrap().clone()
    }

    // For when the connection may have died unnoticed, as while the app was frozen.
    pub fn nudge(&self) -> impl Future<Output = ()> + Send + use<> {
        self.0.nudged.store(true, Ordering::SeqCst);
        self.0.retry.notify_waiters();
        let client = self.clone();
        async move { client.verify(&mut *client.0.link.lock().await).await }
    }

    async fn verify(&self, link: &mut Link) {
        if !self.0.nudged.swap(false, Ordering::SeqCst) {
            return;
        }
        link.failures = 0;
        link.cooldown = None;
        let Some(mut sender) = link.live() else { return };
        if !matches!(time::timeout(PROBE_TIMEOUT, call(&mut sender, None, &Hello {})).await, Ok(Ok(_))) {
            link.hang_up();
        }
    }

    async fn sender(&self) -> Result<SendRequest<Full<Bytes>>> {
        let mut link = self.0.link.lock().await;
        self.verify(&mut link).await;
        if let Some(sender) = link.live() {
            return Ok(sender);
        }
        link.hang_up();
        if let Some((until, failure)) = &link.cooldown
            && Instant::now() < *until
        {
            return Err(failure.clone());
        }
        match self.dial().await {
            Ok((sender, socket)) => {
                *link = Link { sender: Some(sender.clone()), socket: Some(socket), ..Link::default() };
                Ok(sender)
            }
            Err(error) => {
                link.failures += 1;
                link.cooldown = Some((Instant::now() + backoff(link.failures + 1), error.clone()));
                Err(error)
            }
        }
    }

    async fn dial(&self) -> Result<(SendRequest<Full<Bytes>>, Arc<Cut>)> {
        let (mut sender, socket) = match &self.0.target {
            Target::Socket(path) => {
                let stream = UnixStream::connect(path).await.map_err(|e| format!("{e}; is `mu serve` running?"))?;
                handshake(stream).await?
            }
            Target::Remote { address, tls } => {
                let stream = time::timeout(CONNECT_TIMEOUT, TcpStream::connect(address.as_str()))
                    .await
                    .map_err(|_| format!("{address} did not answer"))?
                    .map_err(|e| format!("Cannot reach {address}: {e}"))?;
                stream.set_nodelay(true)?;
                // The certificate is pinned, so the name it is asked for does not matter.
                let name = ServerName::try_from("mu").expect("valid name");
                let stream = time::timeout(CONNECT_TIMEOUT, tls.connect(name, stream))
                    .await
                    .map_err(|_| format!("{address} did not answer"))?
                    .map_err(|e| match tls::mismatch(&e) {
                        true => format!("{address} is not the host this device paired with: its certificate differs"),
                        false => format!("TLS with {address} failed: {e}"),
                    })?;
                if stream.get_ref().1.alpn_protocol() != Some(b"h2") {
                    return Err(format!("{address} is not a mu host").into());
                }
                handshake(stream).await?
            }
        };
        let hello = time::timeout(CONNECT_TIMEOUT, hello(&mut sender)).await;
        match hello.unwrap_or_else(|_| Err("The mu host did not answer".into())) {
            Ok(host) => {
                *self.0.host.lock().unwrap() = Some(host);
                Ok((sender, socket))
            }
            Err(error) => {
                socket.cut();
                Err(error)
            }
        }
    }

    async fn settle(&self, delay: Duration) {
        let nudged = self.0.retry.notified();
        let cooldown = self.0.link.lock().await.cooldown.as_ref().map(|(until, _)| *until);
        let until = cooldown.map_or(Instant::now() + delay, |until| until.max(Instant::now() + delay));
        let _ = time::timeout_at(until, nudged).await;
    }

    pub async fn call<C: Call + Sync>(&self, call: C) -> Result<C::Reply> {
        self::call(&mut self.sender().await?, self.0.authorization.as_ref(), &call).await
    }

    pub async fn read_image(&self, path: &Path) -> Result<Vec<u8>> {
        let data = self.call(ReadImage { path: path.into() }).await?;
        STANDARD.decode(data).map_err(|_| Error::Other("The host sent an image that is not base64".into()))
    }

    pub async fn upload(&self, name: &str, data: Vec<u8>) -> Result<PathBuf> {
        let path = format!("{UPLOAD_PATH}?name={}", utf8_percent_encode(name, NON_ALPHANUMERIC));
        let sender = &mut self.sender().await?;
        let body = send(sender, self.0.authorization.as_ref(), &path, data.into(), "application/octet-stream").await?;
        Ok(serde_json::from_slice(&read(body).await?)?)
    }

    pub async fn shell(&self, cwd: &Path, command: String) -> Result<ShellRun> {
        let call = Shell { cwd: cwd.into(), command };
        let body = post(&mut self.sender().await?, self.0.authorization.as_ref(), Shell::PATH, &call).await?;
        Ok(ShellRun { lines: Lines { body, buf: vec![], scanned: 0 } })
    }

    pub fn subscribe(&self, id: &str, resume: Option<Resume>) -> Subscription {
        Subscription::new(self, Feed::Session { id: id.into(), resume })
    }

    pub fn watch(&self) -> Subscription<WatchEvent> {
        Subscription::new(self, Feed::Host)
    }

    async fn open(&self, feed: &Feed) -> Result<Lines> {
        let sender = &mut self.sender().await?;
        let authorization = self.0.authorization.as_ref();
        let body = match feed {
            Feed::Session { id, resume } => {
                let call = Subscribe { session_id: id.clone(), resume: *resume };
                post(sender, authorization, Subscribe::PATH, &call).await?
            }
            Feed::Host => post(sender, authorization, Watch::PATH, &Watch {}).await?,
        };
        Ok(Lines { body, buf: vec![], scanned: 0 })
    }
}

pub async fn pair(invite: &Invite, device: &str) -> Result<(Remote, HostInfo)> {
    let remote = Remote { address: invite.address.clone(), cert: invite.cert.clone(), token: None };
    let client = Client::connect(Endpoint::Remote(remote.clone())).await?;
    let token = client.call(Pair { code: invite.code.clone(), device: device.into() }).await?;
    let host = client.host().ok_or("The mu host did not say hello")?;
    Ok((Remote { token: Some(token), ..remote }, host))
}

async fn handshake(
    stream: impl AsyncRead + AsyncWrite + Unpin + Send + 'static,
) -> Result<(SendRequest<Full<Bytes>>, Arc<Cut>)> {
    let cut = Arc::new(Cut::default());
    let stream = Cuttable { stream, cut: cut.clone() };
    let (sender, connection) = hyper::client::conn::http2::Builder::new(TokioExecutor::new())
        .timer(TokioTimer::new())
        .keep_alive_interval(Duration::from_secs(15))
        .keep_alive_timeout(Duration::from_secs(5))
        .handshake(TokioIo::new(stream))
        .await?;
    tokio::spawn(connection);
    Ok((sender, cut))
}

// hyper drives a connection from tasks of its own, so aborting ours leaves a dead one hanging;
// failing its socket ends it and every stream on it.
#[derive(Default)]
struct Cut {
    done: AtomicBool,
    reader: sync::Mutex<Option<Waker>>,
    writer: sync::Mutex<Option<Waker>>,
}

impl Cut {
    fn cut(&self) {
        self.done.store(true, Ordering::SeqCst);
        for waiting in [&self.reader, &self.writer] {
            if let Some(waker) = waiting.lock().unwrap().take() {
                waker.wake();
            }
        }
    }

    fn check(&self, waiting: &sync::Mutex<Option<Waker>>, cx: &Context) -> io::Result<()> {
        waiting.lock().unwrap().replace(cx.waker().clone());
        match self.done.load(Ordering::SeqCst) {
            true => Err(io::ErrorKind::ConnectionAborted.into()),
            false => Ok(()),
        }
    }
}

struct Cuttable<S> {
    stream: S,
    cut: Arc<Cut>,
}

impl<S: AsyncRead + Unpin> AsyncRead for Cuttable<S> {
    fn poll_read(self: Pin<&mut Self>, cx: &mut Context, buf: &mut ReadBuf) -> Poll<io::Result<()>> {
        let this = self.get_mut();
        this.cut.check(&this.cut.reader, cx)?;
        Pin::new(&mut this.stream).poll_read(cx, buf)
    }
}

impl<S: AsyncWrite + Unpin> AsyncWrite for Cuttable<S> {
    fn poll_write(self: Pin<&mut Self>, cx: &mut Context, buf: &[u8]) -> Poll<io::Result<usize>> {
        let this = self.get_mut();
        this.cut.check(&this.cut.writer, cx)?;
        Pin::new(&mut this.stream).poll_write(cx, buf)
    }

    fn poll_write_vectored(self: Pin<&mut Self>, cx: &mut Context, bufs: &[io::IoSlice]) -> Poll<io::Result<usize>> {
        let this = self.get_mut();
        this.cut.check(&this.cut.writer, cx)?;
        Pin::new(&mut this.stream).poll_write_vectored(cx, bufs)
    }

    fn is_write_vectored(&self) -> bool {
        self.stream.is_write_vectored()
    }

    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context) -> Poll<io::Result<()>> {
        let this = self.get_mut();
        this.cut.check(&this.cut.writer, cx)?;
        Pin::new(&mut this.stream).poll_flush(cx)
    }

    fn poll_shutdown(self: Pin<&mut Self>, cx: &mut Context) -> Poll<io::Result<()>> {
        Pin::new(&mut self.get_mut().stream).poll_shutdown(cx)
    }
}

async fn hello(sender: &mut SendRequest<Full<Bytes>>) -> Result<HostInfo> {
    let host = call(sender, None, &Hello {}).await?;
    if host.protocol != PROTOCOL {
        return Err(format!(
            "The mu host runs mu {} (protocol {}), this is mu {} (protocol {PROTOCOL}); update the older one",
            host.version,
            host.protocol,
            env!("CARGO_PKG_VERSION")
        )
        .into());
    }
    Ok(host)
}

async fn post(
    sender: &mut SendRequest<Full<Bytes>>,
    authorization: Option<&HeaderValue>,
    path: &str,
    body: &impl Serialize,
) -> Result<Incoming> {
    send(sender, authorization, path, serde_json::to_vec(body)?.into(), "application/json").await
}

async fn send(
    sender: &mut SendRequest<Full<Bytes>>,
    authorization: Option<&HeaderValue>,
    path: &str,
    body: Bytes,
    content_type: &'static str,
) -> Result<Incoming> {
    let mut request = Request::new(Full::new(body));
    *request.method_mut() = hyper::Method::POST;
    *request.uri_mut() = format!("http://localhost{path}").parse().map_err(|_| "Invalid call path")?;
    request.headers_mut().insert(header::CONTENT_TYPE, HeaderValue::from_static(content_type));
    if let Some(authorization) = authorization {
        request.headers_mut().insert(header::AUTHORIZATION, authorization.clone());
    }
    let (response, body) = sender.send_request(request).await?.into_parts();
    if response.status.is_success() {
        return Ok(body);
    }
    let failure = serde_json::from_slice::<Failure>(&read(body).await?);
    let message =
        failure.map_or_else(|_| format!("The mu host answered {}", response.status), |failure| failure.message);
    Err(if response.status == StatusCode::UNAUTHORIZED { Error::Unpaired(message) } else { Error::Other(message) })
}

async fn read(body: Incoming) -> Result<Bytes> {
    Ok(Limited::new(body, LIMIT).collect().await?.to_bytes())
}

async fn call<C: Call + Sync>(
    sender: &mut SendRequest<Full<Bytes>>,
    authorization: Option<&HeaderValue>,
    call: &C,
) -> Result<C::Reply> {
    let body = post(sender, authorization, C::PATH, call).await?;
    Ok(serde_json::from_slice(&read(body).await?)?)
}

// A subscription outlives its connection: it says why it's lost, then carries on
// with events when it is back.
pub enum Update<T = Event> {
    Event(T),
    Lost(Error),
}

enum Feed {
    Session { id: String, resume: Option<Resume> },
    Host,
}

pub struct Subscription<T = Event> {
    client: Client,
    feed: Feed,
    lines: Option<Lines>,
    failures: u32,
    unpaired: bool,
    event: PhantomData<T>,
}

impl<T: DeserializeOwned> Subscription<T> {
    fn new(client: &Client, feed: Feed) -> Self {
        Self { client: client.clone(), feed, lines: None, failures: 0, unpaired: false, event: PhantomData }
    }

    pub async fn open(&mut self) -> Result<()> {
        if self.lines.is_none() {
            self.lines = Some(self.client.open(&self.feed).await?);
        }
        Ok(())
    }

    // Only pairing again lets an unpaired device back in, which makes a new subscription.
    async fn pull(&mut self) -> Update<T> {
        if self.unpaired {
            return std::future::pending().await;
        }
        if self.lines.is_none() {
            if self.failures > 0 {
                self.client.settle(backoff(self.failures)).await;
            }
            if let Err(error) = self.open().await {
                self.unpaired = matches!(error, Error::Unpaired(_));
                self.failures += 1;
                return Update::Lost(error);
            }
        }
        let lines = self.lines.as_mut().expect("opened");
        let lost = match lines.next().await {
            Ok(Some(event)) => {
                self.failures = 0;
                return Update::Event(event);
            }
            Ok(None) => "The mu host ended the stream".into(),
            Err(error) => error,
        };
        self.lines = None;
        self.failures += 1;
        Update::Lost(lost)
    }
}

impl Subscription<Event> {
    pub async fn next(&mut self) -> Update {
        let update = self.pull().await;
        self.track(&update);
        update
    }

    // What already arrived on the open stream; it never waits, nor reconnects.
    pub fn try_next(&mut self) -> Option<Update> {
        self.lines.as_ref()?;
        let Poll::Ready(update) = pin!(self.pull()).poll(&mut Context::from_waker(Waker::noop())) else {
            return None;
        };
        self.track(&update);
        Some(update)
    }

    fn track(&mut self, update: &Update) {
        if let (Update::Event(event), Feed::Session { resume, .. }) = (&update, &mut self.feed) {
            match event {
                Event::Snapshot(snapshot) => *resume = Some(Resume { epoch: snapshot.epoch, blocks: snapshot.keep }),
                Event::Commit { nodes } => {
                    if let Some(resume) = resume {
                        resume.blocks += nodes.iter().map(|node| node.blocks.len() as u64).sum::<u64>();
                    }
                }
                _ => (),
            }
        }
    }
}

impl Subscription<WatchEvent> {
    pub async fn next(&mut self) -> Update<WatchEvent> {
        self.pull().await
    }
}

pub struct ShellRun {
    lines: Lines,
}

impl ShellRun {
    pub async fn next(&mut self) -> Result<Option<ShellEvent>> {
        self.lines.next().await
    }
}

struct Lines {
    body: Incoming,
    buf: Vec<u8>,
    scanned: usize,
}

impl Lines {
    async fn next<T: DeserializeOwned>(&mut self) -> Result<Option<T>> {
        loop {
            if let Some(i) = self.buf[self.scanned..].iter().position(|&b| b == b'\n') {
                let end = self.scanned + i;
                let event = serde_json::from_slice(&self.buf[..end]);
                self.buf.drain(..=end);
                self.scanned = 0;
                return Ok(Some(event?));
            }
            self.scanned = self.buf.len();
            if self.buf.len() > LIMIT {
                return Err("The mu host sent an oversized event".into());
            }
            match self.body.frame().await {
                Some(frame) => {
                    if let Ok(data) = frame?.into_data() {
                        self.buf.extend_from_slice(&data);
                    }
                }
                None if self.buf.is_empty() => return Ok(None),
                None => return Err("The mu host cut an event short".into()),
            }
        }
    }
}
