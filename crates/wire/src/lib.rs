// What a mu host and its clients exchange. Every call is an HTTP/2 POST of a
// JSON body to the call's path, answered with a JSON reply, or with a `Failure`
// under a non-success status. `Subscribe` answers with newline-delimited
// `Event`s instead.
//
// Local clients reach a host through its unix socket, which only its user can
// open. Over the network a host speaks TLS with a self-signed certificate that
// clients pin, and every call but `Hello` and `Pair` must carry a paired
// device's token as `authorization: Bearer <token>`.
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use std::{fmt, net::IpAddr, ops::Range, path::PathBuf, str::FromStr};

// Bump on any change an older peer cannot read; peers refuse a mismatch.
pub const PROTOCOL: u32 = 2;

pub trait Call: Serialize + DeserializeOwned {
    const PATH: &'static str;
    type Reply: Serialize + DeserializeOwned;
}

#[derive(Serialize, Deserialize)]
pub struct Failure {
    pub message: String,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct HostInfo {
    pub protocol: u32,
    pub version: String,
    // The machine's hostname.
    pub name: String,
}

#[derive(Clone, Copy, Default, PartialEq, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Status {
    #[default]
    Idle,
    Running,
    Finished,
    Failed,
}

#[derive(Clone, Copy, PartialEq, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    User,
    Agent,
    Thought,
    Call,
    Notice,
    Warning,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct Block {
    pub kind: Kind,
    pub text: String,
    // Set for calls.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool: Option<Tool>,
    // When the step this block belongs to ran. Set once it is in the transcript,
    // for steps recorded since mu kept time.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub span: Option<Span>,
}

impl Block {
    pub fn new(kind: Kind, text: impl Into<String>) -> Self {
        Self { kind, text: text.into(), tool: None, span: None }
    }
}

// Milliseconds since the Unix epoch. A user's message starts and finishes at once.
// Also persisted in session logs.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Span {
    pub started: u64,
    pub finished: u64,
}

// Tools and models are also persisted in session logs: keep their representations stable.
#[derive(Clone, Serialize, Deserialize)]
pub struct Tool {
    pub call_id: String,
    pub status: ToolStatus,
    pub output: ToolOutput,
}

#[derive(Clone, Default, Serialize, Deserialize)]
pub struct ToolOutput {
    pub text: String,
    // Host path of the full log when the text is only a preview.
    pub log: Option<PathBuf>,
    pub truncated: bool,
}

#[derive(Clone, Copy, Serialize, Deserialize)]
pub enum ToolStatus {
    Pending,
    Running,
    Exited(i32),
    TimedOut(u64),
    Cancelled,
    Error,
}

#[derive(Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Model {
    #[serde(rename = "model")]
    pub name: String,
    pub effort: Option<String>,
    pub context: u64,
}

// Token totals along the current conversation path.
#[derive(Clone, Default, Serialize, Deserialize)]
pub struct Stats {
    pub input: u64,
    pub output: u64,
    // Unset when the provider does not report cache reads.
    pub cached: Option<u64>,
    pub uncached_input: u64,
    // Tokens the latest response occupied in the context window.
    pub context_used: u64,
}

// The small mutable part of a session; always sent whole.
#[derive(Clone, Default, Serialize, Deserialize)]
pub struct State {
    pub status: Status,
    pub title: String,
    pub model: Model,
    pub stats: Stats,
    // Messages waiting to steer the running agent, oldest first.
    pub queued: Vec<String>,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct Skill {
    pub name: String,
    pub description: String,
}

// Resets the client's view of a session, except for the first `keep` blocks of
// its transcript when it resumed. The rest of the transcript along the current
// path and the step in progress follow as ordinary commits and live events.
#[derive(Serialize, Deserialize)]
pub struct Snapshot {
    pub id: String,
    pub cwd: PathBuf,
    pub state: State,
    pub skills: Vec<Skill>,
    // Changes whenever the current path does, which only checking out does.
    pub epoch: u64,
    pub keep: usize,
}

// What a client already has of a session's transcript: the snapshot's epoch,
// and how many blocks it holds since. Only good for the same session.
#[derive(Clone, Copy, Serialize, Deserialize)]
pub struct Resume {
    pub epoch: u64,
    pub blocks: usize,
}

// A snapshot replaces everything; a commit appends to the transcript and ends
// the live step; live indices are stable in between.
#[derive(Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Event {
    Snapshot(Box<Snapshot>),
    State(State),
    Commit { blocks: Vec<Block> },
    LivePush(Block),
    LiveDelta { index: usize, text: String },
    LiveSet { index: usize, block: Block },
    LiveOutput { index: usize, output: ToolOutput },
    LiveNotice(Block),
    // Transient: not part of the session, not replayed to late subscribers.
    Feedback(Block),
}

#[derive(Clone, Serialize, Deserialize)]
pub struct SessionSummary {
    pub id: String,
    pub title: String,
    pub cwd: PathBuf,
    pub status: Status,
    // Why its latest run failed.
    pub error: Option<String>,
    // Not running, with steps no client has shown since they finished.
    pub unread: bool,
    // When its latest step finished, in milliseconds since the Unix epoch;
    // sessions are listed latest first.
    pub updated: u64,
}

// The session list, as `Watch` streams it.
#[derive(Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum WatchEvent {
    // Every session, replacing what came before.
    Sessions { sessions: Vec<SessionSummary> },
    // One session, new or changed.
    Session(SessionSummary),
}

#[derive(Serialize, Deserialize)]
pub struct TreeEntry {
    // Unset for the root.
    pub node: Option<usize>,
    pub label: String,
    pub current: bool,
}

// The @file mention at the cursor. Ranges and cursors count characters.
#[derive(Serialize, Deserialize)]
pub struct Mention {
    pub range: Range<usize>,
    pub query: String,
    // The query already names an existing file.
    pub exact_file: bool,
    pub entries: Vec<MentionEntry>,
}

#[derive(Serialize, Deserialize)]
pub struct MentionEntry {
    // Replacement for the mention's range.
    pub text: String,
    pub label: String,
}

// A device allowed to reach the host over the network.
#[derive(Serialize, Deserialize)]
pub struct Device {
    pub id: String,
    pub name: String,
    // Seconds since the Unix epoch.
    pub paired: u64,
}

// The parts of an invite only the host knows. The address a device should use
// is up to whoever passes the invite on; `addresses` are the host's guesses,
// likeliest first.
#[derive(Serialize, Deserialize)]
pub struct Invitation {
    pub code: String,
    pub cert: String,
    pub port: u16,
    pub addresses: Vec<IpAddr>,
}

// What a device needs to pair with a host, as a link for a QR code:
// `mu://<address>?cert=<fingerprint>&code=<one-time code>`. The fingerprint is
// the unpadded base64url SHA-256 of the host's certificate.
#[derive(Clone, Debug, PartialEq)]
pub struct Invite {
    // host:port, with brackets around an IPv6 host.
    pub address: String,
    pub cert: String,
    pub code: String,
}

impl fmt::Display for Invite {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "mu://{}?cert={}&code={}", self.address, self.cert, self.code)
    }
}

impl FromStr for Invite {
    type Err = String;

    fn from_str(link: &str) -> Result<Self, String> {
        let invalid = || format!("Not a mu invite: {link}");
        let (address, query) =
            link.trim().strip_prefix("mu://").and_then(|rest| rest.split_once('?')).ok_or_else(invalid)?;
        let (mut cert, mut code) = (None, None);
        for pair in query.split('&') {
            match pair.split_once('=') {
                Some(("cert", value)) => cert = Some(value),
                Some(("code", value)) => code = Some(value),
                _ => (),
            }
        }
        let token = |value: Option<&str>| {
            value
                .filter(|value| {
                    !value.is_empty() && value.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
                })
                .map(str::to_owned)
                .ok_or_else(invalid)
        };
        if address.is_empty() || address.contains(|c: char| c.is_whitespace() || c == '/' || c == '@') {
            return Err(invalid());
        }
        Ok(Self { address: address.into(), cert: token(cert)?, code: token(code)? })
    }
}

macro_rules! calls {
    ($($name:ident { $($field:ident: $type:ty),* $(,)? } => $path:literal -> $reply:ty;)*) => {$(
        #[derive(Serialize, Deserialize)]
        pub struct $name { $(pub $field: $type),* }

        impl Call for $name {
            const PATH: &'static str = $path;
            type Reply = $reply;
        }
    )*};
}

calls! {
    Hello {} => "/hello" -> HostInfo;
    // Most recently used first.
    ListSessions {} => "/list_sessions" -> Vec<SessionSummary>;
    // Unset fields fall back to the host's defaults; an empty effort means none.
    CreateSession { cwd: PathBuf, model: Option<String>, effort: Option<String>, context: Option<u64> }
        => "/create_session" -> String;
    // Starts a step, or queues the message as steering while one is running.
    // `skill` names one listed in the snapshot.
    SendMessage { session_id: String, text: String, skill: Option<String> } => "/send_message" -> ();
    Stop { session_id: String } => "/stop" -> ();
    // The session is on screen and its latest steps were seen; clients send it
    // whenever that holds and the agent is not running.
    MarkRead { session_id: String } => "/mark_read" -> ();
    // Takes the newest queued message back for editing.
    RecallQueued { session_id: String } => "/recall_queued" -> Option<String>;
    SetModel { session_id: String, model: Model } => "/set_model" -> ();
    GetTree { session_id: String } => "/get_tree" -> Vec<TreeEntry>;
    // Checking out a user message moves to its parent and returns its text for editing.
    Checkout { session_id: String, node: Option<usize> } => "/checkout" -> Option<String>;
    // `refresh` rebuilds the host's file index; set it when a mention is first opened.
    CompleteMention { session_id: String, text: String, cursor: usize, refresh: bool }
        => "/complete_mention" -> Option<Mention>;
    // Trades an invite's one-time code for this device's token.
    Pair { code: String, device: String } => "/pair" -> String;
    // Local clients only: device management.
    CreateInvite {} => "/create_invite" -> Invitation;
    ListDevices {} => "/list_devices" -> Vec<Device>;
    Unpair { device_id: String } => "/unpair" -> ();
}

// Starts with a snapshot, then streams changes until the client leaves. A
// client that falls too far behind is cut off and may subscribe again, resuming
// from what it has.
#[derive(Serialize, Deserialize)]
pub struct Subscribe {
    pub session_id: String,
    #[serde(default)]
    pub resume: Option<Resume>,
}

impl Subscribe {
    pub const PATH: &'static str = "/subscribe";
}

// Streams `WatchEvent`s: every session, then each one again whenever it
// changes, until the client leaves.
#[derive(Serialize, Deserialize)]
pub struct Watch {}

impl Watch {
    pub const PATH: &'static str = "/watch";
}
