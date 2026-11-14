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
use mu_wire::{
    AppendShell, Call, Checkout, CompleteMention, CreateInvite, CreateSession, DeleteSessions, DeleteSnippet, Failure,
    GetTree, Hello, ListDevices, ListDirectory, ListModels, ListSessions, ListSkills, MarkRead, PROTOCOL, Pair,
    RecallQueued, Resume, SaveSnippet, SendMessage, SetDescription, SetIcon, SetModel, Shell, Stop, Subscribe, Unpair,
    Upload, Watch,
};
use rustls::pki_types::ServerName;
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use std::{
    fmt, io,
    os::unix::net::UnixStream as StdStream,
    path::{Path, PathBuf},
    sync::{self, Arc},
    time::Duration,
};
use tokio::{
    io::{AsyncRead, AsyncWrite},
    net::{TcpStream, UnixStream},
    sync::Mutex,
    time,
};
use tokio_rustls::TlsConnector;

pub use mu_wire::{
    Block, Deleted, Device, Directory, DirectoryEntry, EntryKind, Event, HostInfo, Invitation, Invite, Kind, Mention,
    MentionEntry, Model, ModelInfo, Profile, SessionSummary, ShellEvent, Skill, Snippet, Span, State, Stats, Status,
    Tool, ToolOutput, ToolStatus, TreeEntry, WatchEvent,
};
pub use view::{Change, HostView, View};

// The host accepts calls up to 4 MiB, and a user message near that must still
// fit when it comes back as an event.
const LIMIT: usize = 64 * 1024 * 1024;

const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);

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

error_from!(io::Error, hyper::Error, serde_json::Error, rustls::Error, Box<dyn std::error::Error + Send + Sync>);

pub type Result<T> = std::result::Result<T, Error>;

pub enum Endpoint {
    Socket(PathBuf),
    Stream(StdStream),
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
    Stream(sync::Mutex<Option<StdStream>>),
    Remote { address: String, tls: TlsConnector },
}

#[derive(Clone)]
pub struct Client(Arc<Connection>);

struct Connection {
    target: Target,
    authorization: Option<HeaderValue>,
    sender: Mutex<Option<SendRequest<Full<Bytes>>>>,
    host: sync::Mutex<Option<HostInfo>>,
}

impl Client {
    pub async fn connect(endpoint: Endpoint) -> Result<Self> {
        let (target, authorization) = match endpoint {
            Endpoint::Socket(path) => (Target::Socket(path), None),
            Endpoint::Stream(stream) => (Target::Stream(sync::Mutex::new(Some(stream))), None),
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
        let client = Self(Arc::new(Connection {
            target,
            authorization,
            sender: Mutex::new(None),
            host: sync::Mutex::new(None),
        }));
        client.sender().await?;
        Ok(client)
    }

    pub fn host(&self) -> Option<HostInfo> {
        self.0.host.lock().unwrap().clone()
    }

    async fn sender(&self) -> Result<SendRequest<Full<Bytes>>> {
        let mut slot = self.0.sender.lock().await;
        if let Some(sender) = slot.as_ref().filter(|sender| !sender.is_closed()) {
            return Ok(sender.clone());
        }
        *slot = None;
        let mut sender = match &self.0.target {
            Target::Socket(path) => handshake(UnixStream::connect(path).await?).await?,
            Target::Stream(stream) => {
                let stream = stream.lock().unwrap().take().ok_or("The mu host is gone")?;
                stream.set_nonblocking(true)?;
                handshake(UnixStream::from_std(stream)?).await?
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
        let host = call(&mut sender, None, &Hello {}).await?;
        if host.protocol != PROTOCOL {
            return Err(format!(
                "The mu host runs mu {} (protocol {}), this is mu {} (protocol {PROTOCOL}); update the older one",
                host.version,
                host.protocol,
                env!("CARGO_PKG_VERSION")
            )
            .into());
        }
        *self.0.host.lock().unwrap() = Some(host);
        *slot = Some(sender.clone());
        Ok(sender)
    }

    async fn call<C: Call + Sync>(&self, call: C) -> Result<C::Reply> {
        self::call(&mut self.sender().await?, self.0.authorization.as_ref(), &call).await
    }

    pub async fn pair(&self, code: &str, device: &str) -> Result<String> {
        self.call(Pair { code: code.into(), device: device.into() }).await
    }

    pub async fn invite(&self) -> Result<Invitation> {
        self.call(CreateInvite {}).await
    }

    pub async fn devices(&self) -> Result<Vec<Device>> {
        self.call(ListDevices {}).await
    }

    pub async fn unpair(&self, device_id: &str) -> Result<()> {
        self.call(Unpair { device_id: device_id.into() }).await
    }

    pub async fn sessions(&self) -> Result<Vec<SessionSummary>> {
        self.call(ListSessions {}).await
    }

    pub async fn create(&self, cwd: &Path, model: Model, text: String, skill: Option<String>) -> Result<String> {
        self.call(CreateSession { cwd: cwd.into(), model, text, skill }).await
    }

    pub async fn send(&self, id: &str, text: String, skill: Option<String>) -> Result<()> {
        self.call(SendMessage { session_id: id.into(), text, skill }).await
    }

    pub async fn stop(&self, id: &str) -> Result<()> {
        self.call(Stop { session_id: id.into() }).await
    }

    pub async fn mark_read(&self, id: &str) -> Result<()> {
        self.call(MarkRead { session_id: id.into() }).await
    }

    pub async fn recall(&self, id: &str) -> Result<Option<String>> {
        self.call(RecallQueued { session_id: id.into() }).await
    }

    pub async fn set_model(&self, id: &str, model: Model) -> Result<()> {
        self.call(SetModel { session_id: id.into(), model }).await
    }

    pub async fn models(&self) -> Result<Vec<ModelInfo>> {
        self.call(ListModels {}).await
    }

    pub async fn tree(&self, id: &str) -> Result<Vec<TreeEntry>> {
        self.call(GetTree { session_id: id.into() }).await
    }

    pub async fn checkout(&self, id: &str, node: Option<usize>) -> Result<Option<String>> {
        self.call(Checkout { session_id: id.into(), node }).await
    }

    pub async fn complete(&self, cwd: &Path, text: String, cursor: usize, refresh: bool) -> Result<Option<Mention>> {
        self.call(CompleteMention { cwd: cwd.into(), text, cursor, refresh }).await
    }

    pub async fn append_shell(&self, id: &str, run: &str) -> Result<()> {
        self.call(AppendShell { session_id: id.into(), run: run.into() }).await
    }

    pub async fn delete_sessions(&self) -> Result<Deleted> {
        self.call(DeleteSessions {}).await
    }

    pub async fn set_icon(&self, icon: Option<String>) -> Result<()> {
        self.call(SetIcon { icon }).await
    }

    pub async fn set_description(&self, description: String) -> Result<()> {
        self.call(SetDescription { description }).await
    }

    pub async fn save_snippet(
        &self,
        id: Option<String>,
        name: String,
        command: String,
        icon: Option<String>,
    ) -> Result<String> {
        self.call(SaveSnippet { id, name, command, icon }).await
    }

    pub async fn delete_snippet(&self, id: &str) -> Result<()> {
        self.call(DeleteSnippet { id: id.into() }).await
    }

    pub async fn list_directory(&self, path: &Path) -> Result<Directory> {
        self.call(ListDirectory { path: path.into() }).await
    }

    pub async fn list_skills(&self, cwd: &Path) -> Result<Vec<Skill>> {
        self.call(ListSkills { cwd: cwd.into() }).await
    }

    pub async fn upload(&self, name: &str, bytes: &[u8]) -> Result<PathBuf> {
        self.call(Upload { name: name.into(), data: STANDARD.encode(bytes) }).await
    }

    pub async fn shell(&self, cwd: &Path, command: String) -> Result<ShellRun> {
        let call = Shell { cwd: cwd.into(), command };
        let body = post(&mut self.sender().await?, self.0.authorization.as_ref(), Shell::PATH, &call).await?;
        Ok(ShellRun { lines: Lines { body, buf: vec![], scanned: 0 } })
    }

    pub async fn subscribe(&self, id: &str) -> Result<Subscription> {
        self.follow(Feed::Session { id: id.into(), resume: None }).await
    }

    pub async fn watch(&self) -> Result<Subscription<WatchEvent>> {
        self.follow(Feed::Host).await
    }

    async fn follow<T: DeserializeOwned>(&self, feed: Feed) -> Result<Subscription<T>> {
        let mut lines = self.open(&feed).await?;
        let first = lines.next().await?.ok_or("The host closed the stream")?;
        Ok(Subscription { client: self.clone(), feed, lines: Some(lines), first: Some(first) })
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
    let token = client.pair(&invite.code, device).await?;
    let host = client.host().ok_or("The mu host did not say hello")?;
    Ok((Remote { token: Some(token), ..remote }, host))
}

async fn handshake(stream: impl AsyncRead + AsyncWrite + Unpin + Send + 'static) -> Result<SendRequest<Full<Bytes>>> {
    let (sender, connection) = hyper::client::conn::http2::Builder::new(TokioExecutor::new())
        .timer(TokioTimer::new())
        .keep_alive_interval(Duration::from_secs(20))
        .keep_alive_timeout(Duration::from_secs(10))
        .handshake(TokioIo::new(stream))
        .await?;
    tokio::spawn(connection);
    Ok(sender)
}

async fn post(
    sender: &mut SendRequest<Full<Bytes>>,
    authorization: Option<&HeaderValue>,
    path: &str,
    body: &impl Serialize,
) -> Result<Incoming> {
    let mut request = Request::new(Full::new(serde_json::to_vec(body)?.into()));
    *request.method_mut() = hyper::Method::POST;
    *request.uri_mut() = format!("http://localhost{path}").parse().map_err(|_| "Invalid call path")?;
    request.headers_mut().insert(header::CONTENT_TYPE, HeaderValue::from_static("application/json"));
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

pub enum Update<T = Event> {
    Event(T),
    Lost,
}

enum Feed {
    Session { id: String, resume: Option<Resume> },
    Host,
}

pub struct Subscription<T = Event> {
    client: Client,
    feed: Feed,
    lines: Option<Lines>,
    first: Option<T>,
}

impl<T: DeserializeOwned> Subscription<T> {
    async fn pull(&mut self) -> Update<T> {
        if let Some(event) = self.first.take() {
            return Update::Event(event);
        }
        loop {
            let Some(lines) = &mut self.lines else {
                time::sleep(Duration::from_secs(1)).await;
                match self.client.open(&self.feed).await {
                    Ok(lines) => self.lines = Some(lines),
                    Err(_) => return Update::Lost,
                }
                continue;
            };
            if let Ok(Some(event)) = lines.next().await {
                return Update::Event(event);
            }
            self.lines = None;
            return Update::Lost;
        }
    }
}

impl Subscription<Event> {
    pub async fn next(&mut self) -> Update {
        let update = self.pull().await;
        if let (Update::Event(event), Feed::Session { resume, .. }) = (&update, &mut self.feed) {
            match event {
                Event::Snapshot(snapshot) => *resume = Some(Resume { epoch: snapshot.epoch, blocks: snapshot.keep }),
                Event::Commit { blocks } => {
                    if let Some(resume) = resume {
                        resume.blocks += blocks.len();
                    }
                }
                _ => (),
            }
        }
        update
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
