// Connects to a mu host: the shared core of mu's clients. Call from within a
// tokio runtime.
pub mod hosts;
mod tls;
mod view;

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
    GetTree, Hello, ListDevices, ListDirectory, ListSessions, ListSkills, MarkRead, PROTOCOL, Pair, RecallQueued,
    Resume, SaveSnippet, SendMessage, SetDescription, SetIcon, SetModel, Shell, Stop, Subscribe, Unpair, Watch,
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
    MentionEntry, Model, Profile, SessionSummary, ShellEvent, Skill, Snippet, Span, State, Stats, Status, Tool,
    ToolOutput, ToolStatus, TreeEntry, WatchEvent,
};
pub use view::{Change, HostView, View};

// The most a single reply or event may take. The host accepts calls up to 4 MiB,
// and a user message near that must still fit when it comes back as an event.
const LIMIT: usize = 64 * 1024 * 1024;

// How long a host on the network gets to accept a connection.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);

pub enum Error {
    // The host refused this device's token: it was never paired, or unpaired since.
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
    // A host listening for any number of clients.
    Socket(PathBuf),
    // One end of a socket pair whose other end a host is already serving:
    // a private host that nothing else can reach.
    Stream(StdStream),
    // A host on the network.
    Remote(Remote),
}

// A host on the network, and what this device proves itself with.
#[derive(Clone, PartialEq, Serialize, Deserialize)]
pub struct Remote {
    // host:port, with brackets around an IPv6 host.
    pub address: String,
    // The fingerprint of the host's certificate, pinned when pairing.
    pub cert: String,
    // Unset only until this device pairs.
    pub token: Option<String>,
}

enum Target {
    Socket(PathBuf),
    // Taken by the first connection; the host ends with it.
    Stream(sync::Mutex<Option<StdStream>>),
    Remote { address: String, tls: TlsConnector },
}

#[derive(Clone)]
pub struct Client(Arc<Connection>);

struct Connection {
    target: Target,
    // Sent with every call.
    authorization: Option<HeaderValue>,
    sender: Mutex<Option<SendRequest<Full<Bytes>>>>,
    // What the host said in its latest hello.
    host: sync::Mutex<Option<HostInfo>>,
}

impl Client {
    // Fails now, with the real reason, rather than on the first call. Socket and
    // network connections re-establish themselves if the host restarts; a
    // private stream ends with its host.
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

    // What the host said when this client last connected.
    pub fn host(&self) -> Option<HostInfo> {
        self.0.host.lock().unwrap().clone()
    }

    // The open connection, or a new one that has said hello.
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

    // Trades an invite's one-time code for a token naming this device.
    pub async fn pair(&self, code: &str, device: &str) -> Result<String> {
        self.call(Pair { code: code.into(), device: device.into() }).await
    }

    // Local clients only: lets one more device pair.
    pub async fn invite(&self) -> Result<Invitation> {
        self.call(CreateInvite {}).await
    }

    // Local clients only.
    pub async fn devices(&self) -> Result<Vec<Device>> {
        self.call(ListDevices {}).await
    }

    // Local clients only: refuses the device's token from now on.
    pub async fn unpair(&self, device_id: &str) -> Result<()> {
        self.call(Unpair { device_id: device_id.into() }).await
    }

    // Most recently used first.
    pub async fn sessions(&self) -> Result<Vec<SessionSummary>> {
        self.call(ListSessions {}).await
    }

    // Unset fields fall back to the host's defaults; an empty effort means none.
    pub async fn create(
        &self,
        cwd: &Path,
        model: Option<String>,
        effort: Option<String>,
        context: Option<u64>,
    ) -> Result<String> {
        self.call(CreateSession { cwd: cwd.into(), model, effort, context }).await
    }

    // Starts a step, or queues the message as steering while one is running.
    pub async fn send(&self, id: &str, text: String, skill: Option<String>) -> Result<()> {
        self.call(SendMessage { session_id: id.into(), text, skill }).await
    }

    pub async fn stop(&self, id: &str) -> Result<()> {
        self.call(Stop { session_id: id.into() }).await
    }

    // Tells the host the session is on screen with its latest steps seen; call
    // it whenever that holds and the agent is not running.
    pub async fn mark_read(&self, id: &str) -> Result<()> {
        self.call(MarkRead { session_id: id.into() }).await
    }

    // Takes the newest queued message back for editing.
    pub async fn recall(&self, id: &str) -> Result<Option<String>> {
        self.call(RecallQueued { session_id: id.into() }).await
    }

    pub async fn set_model(&self, id: &str, model: Model) -> Result<()> {
        self.call(SetModel { session_id: id.into(), model }).await
    }

    pub async fn tree(&self, id: &str) -> Result<Vec<TreeEntry>> {
        self.call(GetTree { session_id: id.into() }).await
    }

    // Checking out a user message moves to its parent and returns its text for editing.
    // The session's stream answers with a fresh snapshot.
    pub async fn checkout(&self, id: &str, node: Option<usize>) -> Result<Option<String>> {
        self.call(Checkout { session_id: id.into(), node }).await
    }

    // The @file mention at `cursor` (in characters), completed against the host's
    // files. Set `refresh` when a mention is first opened to rebuild the index.
    pub async fn complete(&self, id: &str, text: String, cursor: usize, refresh: bool) -> Result<Option<Mention>> {
        self.call(CompleteMention { session_id: id.into(), text, cursor, refresh }).await
    }

    // Adds a command run in shell mode, and its result, to the session's
    // conversation. Only while its agent is not running, and after a message.
    pub async fn append_shell(&self, id: &str, run: &str) -> Result<()> {
        self.call(AppendShell { session_id: id.into(), run: run.into() }).await
    }

    // Leaves alone sessions in use: running, or open in a client.
    pub async fn delete_sessions(&self) -> Result<Deleted> {
        self.call(DeleteSessions {}).await
    }

    // The profile's changes reach clients through `watch`. An unset or empty
    // icon goes back to the clients' default.
    pub async fn set_icon(&self, icon: Option<String>) -> Result<()> {
        self.call(SetIcon { icon }).await
    }

    pub async fn set_description(&self, description: String) -> Result<()> {
        self.call(SetDescription { description }).await
    }

    // Adds a snippet, or replaces the one with this id. Returns its id.
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

    // `~` and an empty path name the host's home.
    pub async fn list_directory(&self, path: &Path) -> Result<Directory> {
        self.call(ListDirectory { path: path.into() }).await
    }

    // The skills a session in `cwd` would have, for before there is one.
    pub async fn list_skills(&self, cwd: &Path) -> Result<Vec<Skill>> {
        self.call(ListSkills { cwd: cwd.into() }).await
    }

    // Runs a command the user typed, in `cwd` on the host. Resolves once it has
    // started; dropping the run kills it.
    pub async fn shell(&self, cwd: &Path, command: String) -> Result<ShellRun> {
        let call = Shell { cwd: cwd.into(), command };
        let body = post(&mut self.sender().await?, self.0.authorization.as_ref(), Shell::PATH, &call).await?;
        Ok(ShellRun { lines: Lines { body, buf: vec![], scanned: 0 } })
    }

    // Resolves with the session's first event, so an error means the session cannot be opened.
    pub async fn subscribe(&self, id: &str) -> Result<Subscription> {
        self.follow(Feed::Session { id: id.into(), resume: None }).await
    }

    // The host's profile and session list, kept current; see `HostView`.
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

// Pairs with the host an invite points to, as `device`. What it returns is how
// to reach that host from now on, so keep it, like in `hosts::Hosts`.
pub async fn pair(invite: &Invite, device: &str) -> Result<(Remote, HostInfo)> {
    let remote = Remote { address: invite.address.clone(), cert: invite.cert.clone(), token: None };
    let client = Client::connect(Endpoint::Remote(remote.clone())).await?;
    let token = client.pair(&invite.code, device).await?;
    let host = client.host().ok_or("The mu host did not say hello")?;
    Ok((Remote { token: Some(token), ..remote }, host))
}

// Pings notice a connection that died without closing, like a phone's after it
// changes networks, so subscriptions can report it and start over.
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
    // The stream broke; it picks up again once the host is back.
    Lost,
}

// What a subscription follows.
enum Feed {
    // A session, and what this client was given of its transcript so far.
    Session { id: String, resume: Option<Resume> },
    Host,
}

// Follows a session, or the session list, for as long as it is polled.
pub struct Subscription<T = Event> {
    client: Client,
    feed: Feed,
    lines: Option<Lines>,
    first: Option<T>,
}

impl<T: DeserializeOwned> Subscription<T> {
    // Whenever the stream breaks this reports `Lost`, then follows again a
    // second later.
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
    // After a break the session resumes with a snapshot that keeps the
    // transcript it already passed on, as long as the path stayed the same.
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
    // After a break it starts over, with everything.
    pub async fn next(&mut self) -> Update<WatchEvent> {
        self.pull().await
    }
}

// A command running in shell mode. It cannot be picked up again once its
// stream breaks; dropping it kills the command.
pub struct ShellRun {
    lines: Lines,
}

impl ShellRun {
    // `None` once the stream ends, which follows `ShellEvent::Done`, unless the
    // connection broke first.
    pub async fn next(&mut self) -> Result<Option<ShellEvent>> {
        self.lines.next().await
    }
}

// Newline-delimited events on a response body.
struct Lines {
    body: Incoming,
    buf: Vec<u8>,
    // Bytes of `buf` already known to hold no newline.
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
