use serde::{Deserialize, Serialize, de::DeserializeOwned};
use std::{fmt, net::IpAddr, path::PathBuf, str::FromStr};

#[cfg(feature = "uniffi")]
uniffi::setup_scaffolding!();

// Paths cross to the apps as strings.
#[cfg(feature = "uniffi")]
uniffi::custom_type!(PathBuf, String, {
    remote,
    lower: |path| path.to_string_lossy().into_owned(),
    try_lift: |path| Ok(path.into()),
});

// Bump on any change an older peer cannot read; peers refuse a mismatch.
pub const PROTOCOL: u32 = 11;

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
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
pub enum Status {
    #[default]
    Idle,
    Running,
    Finished,
    Failed,
}

#[derive(Clone, Copy, PartialEq, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
pub enum Kind {
    User,
    // From another session, which `from` names.
    AgentMessage,
    Agent,
    Thought,
    Call,
    Shell,
    Notice,
    Warning,
}

#[derive(Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct Block {
    pub kind: Kind,
    pub text: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool: Option<Tool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub span: Option<Span>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub from: Option<Sender>,
}

impl Block {
    pub fn new(kind: Kind, text: impl Into<String>) -> Self {
        Self { kind, text: text.into(), tool: None, span: None, from: None }
    }
}

// A session as it was when it sent a message.
#[derive(Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct Sender {
    pub session: String,
    pub title: String,
}

// Milliseconds since the Unix epoch.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct Span {
    pub started: u64,
    pub finished: u64,
}

#[derive(Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct Tool {
    pub call_id: String,
    pub status: ToolStatus,
    pub output: ToolOutput,
}

#[derive(Clone, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct ToolOutput {
    pub text: String,
    pub log: Option<PathBuf>,
    pub truncated: bool,
}

#[derive(Clone, Copy, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
pub enum ToolStatus {
    Pending,
    Running,
    Exited(i32),
    TimedOut(u64),
    Cancelled,
    Error,
}

impl ToolStatus {
    // How a tool ended, unless it just succeeded; an error says so in its output.
    pub fn summary(self) -> Option<String> {
        Some(match self {
            Self::Exited(0) | Self::Error => return None,
            Self::Pending => "pending".into(),
            Self::Running => "running".into(),
            Self::Exited(code) => format!("exit {code}"),
            Self::TimedOut(ms) => format!("timed out after {ms} ms; process group killed"),
            Self::Cancelled => "cancelled by user; process group killed".into(),
        })
    }
}

#[derive(Clone, Default, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct Model {
    #[serde(rename = "model")]
    pub name: String,
    pub effort: Option<String>,
    pub context: Option<u64>,
}

#[derive(Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct ModelInfo {
    pub name: String,
    pub context: Option<u64>,
}

#[derive(Clone, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct Stats {
    pub input: u64,
    pub output: u64,
    pub cached: Option<u64>,
    pub uncached_input: u64,
    pub context_used: u64,
}

#[derive(Clone, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct State {
    pub status: Status,
    pub title: String,
    pub model: Model,
    pub stats: Stats,
    pub queued: Vec<String>,
}

#[derive(Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
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

#[derive(Clone, Serialize, Deserialize)]
pub struct Node {
    pub id: usize,
    // Itself included, oldest first.
    pub siblings: Vec<usize>,
    pub blocks: Vec<Block>,
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
    Commit { nodes: Vec<Node> },
    Live(LiveEvent),
    Feedback(Block),
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum LiveEvent {
    Push(Block),
    Delta { index: usize, text: String },
    Set { index: usize, block: Block },
    Output { index: usize, output: ToolOutput },
    Notice(Block),
}

// What a turn has shown so far and not yet committed. The host and its clients
// each keep one, and stay in step by applying the same events.
#[derive(Default)]
pub struct Live {
    pub blocks: Vec<Block>,
    pub notice: Option<Block>,
}

pub enum Touched {
    Block(usize),
    Notice,
}

impl Live {
    pub fn clear(&mut self) {
        *self = Self::default();
    }

    pub fn apply(&mut self, event: &LiveEvent) -> Touched {
        match event {
            LiveEvent::Push(block) => {
                self.blocks.push(block.clone());
                Touched::Block(self.blocks.len() - 1)
            }
            LiveEvent::Delta { index, text } => {
                if let Some(block) = self.blocks.get_mut(*index) {
                    block.text.push_str(text);
                }
                Touched::Block(*index)
            }
            LiveEvent::Set { index, block } => {
                if let Some(slot) = self.blocks.get_mut(*index) {
                    *slot = block.clone();
                }
                Touched::Block(*index)
            }
            LiveEvent::Output { index, output } => {
                if let Some(tool) = self.blocks.get_mut(*index).and_then(|block| block.tool.as_mut()) {
                    tool.output = output.clone();
                }
                Touched::Block(*index)
            }
            LiveEvent::Notice(block) => {
                self.notice = Some(block.clone());
                Touched::Notice
            }
        }
    }
}

#[derive(Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct SessionSummary {
    pub id: String,
    pub title: String,
    pub cwd: PathBuf,
    pub parent: Option<String>,
    pub status: Status,
    pub error: Option<String>,
    pub unread: bool,
    // Milliseconds since the Unix epoch.
    pub updated: u64,
}

#[derive(Clone, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct Profile {
    pub name: String,
    pub home: PathBuf,
    pub icon: Option<String>,
    pub description: String,
    pub snippets: Vec<Snippet>,
}

#[derive(Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
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
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct Deleted {
    pub sessions: u64,
    pub in_use: u64,
}

#[derive(Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct Directory {
    pub path: PathBuf,
    pub entries: Vec<DirectoryEntry>,
}

#[derive(Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct DirectoryEntry {
    pub name: String,
    pub kind: EntryKind,
}

#[derive(Clone, Copy, PartialEq, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
pub enum EntryKind {
    File,
    Directory,
    Repository,
}

#[derive(Serialize, Deserialize)]
pub struct TreeEntry {
    pub node: usize,
    pub parent: Option<usize>,
    pub kind: Option<Kind>,
    pub text: String,
    pub current: bool,
}

// Offsets and cursors count characters, not bytes.
#[derive(Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct Mention {
    pub start: u32,
    pub end: u32,
    pub query: String,
    pub exact_file: bool,
    pub entries: Vec<MentionEntry>,
}

#[derive(Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct MentionEntry {
    pub text: String,
    pub label: String,
}

// An @mention as written in a message, `path` unescaped. Unclosed ones are still being typed.
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct Written {
    pub start: u32,
    pub end: u32,
    pub path: String,
    pub quoted: bool,
    pub closed: bool,
}

pub fn mentions(text: &str) -> Vec<Written> {
    let chars: Vec<_> = text.chars().collect();
    let mut found = vec![];
    let mut i = 0;
    while i < chars.len() {
        if chars[i] != '@' || (i > 0 && !chars[i - 1].is_whitespace()) || chars.get(i + 1) == Some(&'@') {
            i += 1;
            continue;
        }
        let start = i;
        i += 1;
        let quoted = chars.get(i) == Some(&'"');
        if quoted {
            i += 1;
        }
        let mut path = String::new();
        let mut closed = !quoted;
        while i < chars.len() {
            let c = chars[i];
            if quoted && c == '"' {
                i += 1;
                closed = true;
                break;
            }
            if !quoted && c.is_whitespace() {
                break;
            }
            if quoted && c == '\\' && chars.get(i + 1).is_some_and(|c| matches!(c, '"' | '\\')) {
                i += 1;
            }
            path.push(chars[i]);
            i += 1;
        }
        found.push(Written { start: start as u32, end: i as u32, path, quoted, closed });
    }
    found
}

// What a mention brings along when sent, keyed by its path as written.
#[derive(Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct Resolved {
    pub path: String,
    pub file: PathBuf,
    pub kind: Attached,
}

#[derive(Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
pub enum Attached {
    Image,
    Text { lines: u32, head: String },
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
    // Without a model name, a session with a parent takes the parent's model, and an effort given still applies.
    CreateSession { cwd: PathBuf, model: Model, text: String, skill: Option<String>, parent: Option<String> }
        => "/create_session" -> String;
    // `from` is the session sending it, which the message then says.
    SendMessage { session_id: String, text: String, skill: Option<String>, from: Option<String> }
        => "/send_message" -> ();
    // `children` stops the sessions it parents too, and theirs.
    Stop { session_id: String, children: bool } => "/stop" -> ();
    MarkRead { session_id: String } => "/mark_read" -> ();
    // False when the message at `index` is no longer `text`, having been sent.
    RecallQueued { session_id: String, index: usize, text: String } => "/recall_queued" -> bool;
    SetModel { session_id: String, model: Model } => "/set_model" -> ();
    ListModels {} => "/list_models" -> Vec<ModelInfo>;
    GetTree { session_id: String } => "/get_tree" -> Vec<TreeEntry>;
    Checkout { session_id: String, node: Option<usize> } => "/checkout" -> ();
    // Back to before the user message `node`, whose text it answers.
    Edit { session_id: String, node: usize } => "/edit" -> String;
    SwitchBranch { session_id: String, node: usize } => "/switch_branch" -> ();
    CompleteMention { cwd: PathBuf, text: String, cursor: u32, refresh: bool }
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
    ResolveMentions { cwd: PathBuf, text: String, skill: Option<String> }
        => "/resolve_mentions" -> Vec<Resolved>;
    // `path` must be an image; the reply is its data as base64.
    ReadImage { path: PathBuf } => "/read_image" -> String;
    // `data` is base64; the reply is where the host keeps it, to @mention.
    Upload { name: String, data: String } => "/upload" -> PathBuf;
    Pair { code: String, device: String } => "/pair" -> String;
    CreateInvite {} => "/create_invite" -> Invitation;
    ListDevices {} => "/list_devices" -> Vec<Device>;
    Unpair { device_id: String } => "/unpair" -> ();
    ListSessions {} => "/list_sessions" -> Vec<SessionSummary>;
    // From the special commands in a bash run, which `run` is the token of. Local clients only.
    RunSession { run: String } => "/run_session" -> Option<String>;
    ViewImage { run: String, path: PathBuf } => "/view_image" -> ();
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
