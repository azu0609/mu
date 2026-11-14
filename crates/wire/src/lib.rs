use serde::{Deserialize, Serialize, de::DeserializeOwned};
use std::{fmt, net::IpAddr, ops::Range, path::PathBuf, str::FromStr};

// Bump on any change an older peer cannot read; peers refuse a mismatch.
pub const PROTOCOL: u32 = 5;

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
    Shell,
    Notice,
    Warning,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct Block {
    pub kind: Kind,
    pub text: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool: Option<Tool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub span: Option<Span>,
}

impl Block {
    pub fn new(kind: Kind, text: impl Into<String>) -> Self {
        Self { kind, text: text.into(), tool: None, span: None }
    }
}

// Milliseconds since the Unix epoch.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Span {
    pub started: u64,
    pub finished: u64,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct Tool {
    pub call_id: String,
    pub status: ToolStatus,
    pub output: ToolOutput,
}

#[derive(Clone, Default, Serialize, Deserialize)]
pub struct ToolOutput {
    pub text: String,
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
    pub context: Option<u64>,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct ModelInfo {
    pub name: String,
    pub context: Option<u64>,
}

#[derive(Clone, Default, Serialize, Deserialize)]
pub struct Stats {
    pub input: u64,
    pub output: u64,
    pub cached: Option<u64>,
    pub uncached_input: u64,
    pub context_used: u64,
}

#[derive(Clone, Default, Serialize, Deserialize)]
pub struct State {
    pub status: Status,
    pub title: String,
    pub model: Model,
    pub stats: Stats,
    pub queued: Vec<String>,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct Skill {
    pub name: String,
    pub description: String,
}

#[derive(Serialize, Deserialize)]
pub struct Snapshot {
    pub id: String,
    pub cwd: PathBuf,
    pub state: State,
    pub skills: Vec<Skill>,
    pub epoch: u64,
    pub keep: usize,
}

#[derive(Clone, Copy, Serialize, Deserialize)]
pub struct Resume {
    pub epoch: u64,
    pub blocks: usize,
}

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
    Feedback(Block),
}

#[derive(Clone, Serialize, Deserialize)]
pub struct SessionSummary {
    pub id: String,
    pub title: String,
    pub cwd: PathBuf,
    pub status: Status,
    pub error: Option<String>,
    pub unread: bool,
    // Milliseconds since the Unix epoch.
    pub updated: u64,
}

#[derive(Clone, Default, Serialize, Deserialize)]
pub struct Profile {
    pub name: String,
    pub home: PathBuf,
    pub icon: Option<String>,
    pub description: String,
    pub snippets: Vec<Snippet>,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct Snippet {
    pub id: String,
    pub name: String,
    pub command: String,
    pub icon: Option<String>,
}

#[derive(Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum WatchEvent {
    Host { profile: Profile, sessions: Vec<SessionSummary> },
    Profile(Profile),
    Session(SessionSummary),
}

#[derive(Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ShellEvent {
    Output(ToolOutput),
    Done { run: String, status: ToolStatus, output: ToolOutput },
}

#[derive(Serialize, Deserialize)]
pub struct Deleted {
    pub sessions: u64,
    pub in_use: u64,
}

#[derive(Serialize, Deserialize)]
pub struct Directory {
    pub path: PathBuf,
    pub entries: Vec<DirectoryEntry>,
}

#[derive(Serialize, Deserialize)]
pub struct DirectoryEntry {
    pub name: String,
    pub kind: EntryKind,
}

#[derive(Clone, Copy, PartialEq, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EntryKind {
    File,
    Directory,
    Repository,
}

#[derive(Serialize, Deserialize)]
pub struct TreeEntry {
    pub node: Option<usize>,
    pub label: String,
    pub current: bool,
}

// Ranges and cursors count characters, not bytes.
#[derive(Serialize, Deserialize)]
pub struct Mention {
    pub range: Range<usize>,
    pub query: String,
    pub exact_file: bool,
    pub entries: Vec<MentionEntry>,
}

#[derive(Serialize, Deserialize)]
pub struct MentionEntry {
    pub text: String,
    pub label: String,
}

#[derive(Serialize, Deserialize)]
pub struct Device {
    pub id: String,
    pub name: String,
    // Seconds since the Unix epoch.
    pub paired: u64,
}

#[derive(Serialize, Deserialize)]
pub struct Invitation {
    pub code: String,
    pub cert: String,
    pub port: u16,
    pub addresses: Vec<IpAddr>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Invite {
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
    ListSessions {} => "/list_sessions" -> Vec<SessionSummary>;
    // A session starts with its first message, so none is ever left empty.
    CreateSession { cwd: PathBuf, model: Model, text: String, skill: Option<String> } => "/create_session" -> String;
    SendMessage { session_id: String, text: String, skill: Option<String> } => "/send_message" -> ();
    Stop { session_id: String } => "/stop" -> ();
    MarkRead { session_id: String } => "/mark_read" -> ();
    RecallQueued { session_id: String } => "/recall_queued" -> Option<String>;
    SetModel { session_id: String, model: Model } => "/set_model" -> ();
    ListModels {} => "/list_models" -> Vec<ModelInfo>;
    GetTree { session_id: String } => "/get_tree" -> Vec<TreeEntry>;
    Checkout { session_id: String, node: Option<usize> } => "/checkout" -> Option<String>;
    CompleteMention { cwd: PathBuf, text: String, cursor: usize, refresh: bool }
        => "/complete_mention" -> Option<Mention>;
    AppendShell { session_id: String, run: String } => "/append_shell" -> ();
    DeleteSessions {} => "/delete_sessions" -> Deleted;
    SetIcon { icon: Option<String> } => "/set_icon" -> ();
    SetDescription { description: String } => "/set_description" -> ();
    SaveSnippet { id: Option<String>, name: String, command: String, icon: Option<String> }
        => "/save_snippet" -> String;
    DeleteSnippet { id: String } => "/delete_snippet" -> ();
    ListDirectory { path: PathBuf } => "/list_directory" -> Directory;
    ListSkills { cwd: PathBuf } => "/list_skills" -> Vec<Skill>;
    // `data` is base64; the reply is where the host keeps it, to @mention.
    Upload { name: String, data: String } => "/upload" -> PathBuf;
    Pair { code: String, device: String } => "/pair" -> String;
    CreateInvite {} => "/create_invite" -> Invitation;
    ListDevices {} => "/list_devices" -> Vec<Device>;
    Unpair { device_id: String } => "/unpair" -> ();
}

#[derive(Serialize, Deserialize)]
pub struct Subscribe {
    pub session_id: String,
    #[serde(default)]
    pub resume: Option<Resume>,
}

impl Subscribe {
    pub const PATH: &'static str = "/subscribe";
}

#[derive(Serialize, Deserialize)]
pub struct Watch {}

impl Watch {
    pub const PATH: &'static str = "/watch";
}

#[derive(Serialize, Deserialize)]
pub struct Shell {
    pub cwd: PathBuf,
    pub command: String,
}

impl Shell {
    pub const PATH: &'static str = "/shell";
}
