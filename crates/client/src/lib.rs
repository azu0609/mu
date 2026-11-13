// Connects to a mu host: the shared core of mu's clients. Call from within a
// tokio runtime.
mod view;

use http_body_util::{BodyExt, Full, Limited};
use hyper::{
    Request,
    body::{Bytes, Incoming},
    client::conn::http2::SendRequest,
    header::{self, HeaderValue},
};
use hyper_util::rt::{TokioExecutor, TokioIo};
use mu_wire::{
    Call, Checkout, CompleteMention, CreateSession, Failure, GetTree, Hello, ListSessions, PROTOCOL, RecallQueued,
    SendMessage, SetModel, Stop, Subscribe,
};
use serde::Serialize;
use std::{
    fmt, io,
    os::unix::net::UnixStream as StdStream,
    path::{Path, PathBuf},
    sync::{self, Arc},
    time::Duration,
};
use tokio::{net::UnixStream, sync::Mutex, time};

pub use mu_wire::{
    Block, Event, Kind, Mention, MentionEntry, Model, SessionSummary, Skill, State, Stats, Status, Tool, ToolOutput,
    ToolStatus, TreeEntry,
};
pub use view::{Change, View};

// The most a single reply or event may take. The host accepts calls up to 4 MiB,
// and a user message near that must still fit when it comes back as an event.
const LIMIT: usize = 64 * 1024 * 1024;

#[derive(Debug)]
pub struct Error(String);

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for Error {}

impl From<String> for Error {
    fn from(message: String) -> Self {
        Self(message)
    }
}

impl From<&str> for Error {
    fn from(message: &str) -> Self {
        Self(message.into())
    }
}

macro_rules! error_from {
    ($($type:ty),*) => {$(
        impl From<$type> for Error {
            fn from(error: $type) -> Self {
                Self(error.to_string())
            }
        }
    )*};
}

error_from!(io::Error, hyper::Error, serde_json::Error, Box<dyn std::error::Error + Send + Sync>);

pub type Result<T> = std::result::Result<T, Error>;

pub enum Endpoint {
    // A host listening for any number of clients.
    Socket(PathBuf),
    // One end of a socket pair whose other end a host is already serving:
    // a private host that nothing else can reach.
    Stream(StdStream),
}

enum Target {
    Socket(PathBuf),
    // Taken by the first connection; the host ends with it.
    Stream(sync::Mutex<Option<StdStream>>),
}

#[derive(Clone)]
pub struct Client(Arc<Connection>);

struct Connection {
    target: Target,
    sender: Mutex<Option<SendRequest<Full<Bytes>>>>,
}

impl Client {
    // Fails now, with the real reason, rather than on the first call. A socket
    // connection re-establishes itself if the host restarts; a private stream
    // ends with its host.
    pub async fn connect(endpoint: Endpoint) -> Result<Self> {
        let target = match endpoint {
            Endpoint::Socket(path) => Target::Socket(path),
            Endpoint::Stream(stream) => Target::Stream(sync::Mutex::new(Some(stream))),
        };
        let client = Self(Arc::new(Connection { target, sender: Mutex::new(None) }));
        client.sender().await?;
        Ok(client)
    }

    // The open connection, or a new one that has said hello.
    async fn sender(&self) -> Result<SendRequest<Full<Bytes>>> {
        let mut slot = self.0.sender.lock().await;
        if let Some(sender) = slot.as_ref().filter(|sender| !sender.is_closed()) {
            return Ok(sender.clone());
        }
        *slot = None;
        let stream = match &self.0.target {
            Target::Socket(path) => UnixStream::connect(path).await?,
            Target::Stream(stream) => {
                let stream = stream.lock().unwrap().take().ok_or("The mu host is gone")?;
                stream.set_nonblocking(true)?;
                UnixStream::from_std(stream)?
            }
        };
        let (mut sender, connection) =
            hyper::client::conn::http2::handshake(TokioExecutor::new(), TokioIo::new(stream)).await?;
        tokio::spawn(connection);
        let host = call(&mut sender, &Hello {}).await?;
        if host.protocol != PROTOCOL {
            return Err(format!(
                "The mu host runs mu {} (protocol {}), this is mu {} (protocol {PROTOCOL}); restart the older one",
                host.version,
                host.protocol,
                env!("CARGO_PKG_VERSION")
            )
            .into());
        }
        *slot = Some(sender.clone());
        Ok(sender)
    }

    async fn call<C: Call + Sync>(&self, call: C) -> Result<C::Reply> {
        self::call(&mut self.sender().await?, &call).await
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

    // Resolves with the session's first event, so an error means the session cannot be opened.
    pub async fn subscribe(&self, id: &str) -> Result<Subscription> {
        let mut lines = self.open(id).await?;
        let first = lines.next().await?.ok_or("The host closed the session stream")?;
        Ok(Subscription { client: self.clone(), id: id.into(), lines: Some(lines), first: Some(first) })
    }

    async fn open(&self, id: &str) -> Result<Lines> {
        let body = post(&mut self.sender().await?, Subscribe::PATH, &Subscribe { session_id: id.into() }).await?;
        Ok(Lines { body, buf: vec![], scanned: 0 })
    }
}

async fn post(sender: &mut SendRequest<Full<Bytes>>, path: &str, body: &impl Serialize) -> Result<Incoming> {
    let mut request = Request::new(Full::new(serde_json::to_vec(body)?.into()));
    *request.method_mut() = hyper::Method::POST;
    *request.uri_mut() = format!("http://localhost{path}").parse().map_err(|_| "Invalid call path")?;
    request.headers_mut().insert(header::CONTENT_TYPE, HeaderValue::from_static("application/json"));
    let (response, body) = sender.send_request(request).await?.into_parts();
    if response.status.is_success() {
        return Ok(body);
    }
    let failure = serde_json::from_slice::<Failure>(&read(body).await?);
    Err(failure.map_or_else(|_| format!("The mu host answered {}", response.status), |failure| failure.message).into())
}

async fn read(body: Incoming) -> Result<Bytes> {
    Ok(Limited::new(body, LIMIT).collect().await?.to_bytes())
}

async fn call<C: Call + Sync>(sender: &mut SendRequest<Full<Bytes>>, call: &C) -> Result<C::Reply> {
    let body = post(sender, C::PATH, call).await?;
    Ok(serde_json::from_slice(&read(body).await?)?)
}

pub enum Update {
    Event(Event),
    // The stream broke; a fresh snapshot follows once the session is back.
    Lost,
}

// Follows one session for as long as it is polled.
pub struct Subscription {
    client: Client,
    id: String,
    lines: Option<Lines>,
    first: Option<Event>,
}

impl Subscription {
    // Whenever the stream breaks this reports `Lost`, then subscribes again a
    // second later, starting over from a fresh snapshot.
    pub async fn next(&mut self) -> Update {
        if let Some(event) = self.first.take() {
            return Update::Event(event);
        }
        loop {
            let Some(lines) = &mut self.lines else {
                time::sleep(Duration::from_secs(1)).await;
                match self.client.open(&self.id).await {
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

// Newline-delimited events on a response body.
struct Lines {
    body: Incoming,
    buf: Vec<u8>,
    // Bytes of `buf` already known to hold no newline.
    scanned: usize,
}

impl Lines {
    async fn next(&mut self) -> Result<Option<Event>> {
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
