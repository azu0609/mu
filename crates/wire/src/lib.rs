// What a mu host and its clients exchange. Every call is an HTTP/2 POST of a
// JSON body to the call's path, answered with a JSON reply, or with a `Failure`
// under a non-success status. `Subscribe` answers with newline-delimited
// `Event`s instead.
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use std::{ops::Range, path::PathBuf};

// Bump on any change an older peer cannot read; peers refuse a mismatch.
pub const PROTOCOL: u32 = 1;

pub trait Call: Serialize + DeserializeOwned {
    const PATH: &'static str;
    type Reply: Serialize + DeserializeOwned;
}

#[derive(Serialize, Deserialize)]
pub struct Failure {
    pub message: String,
}

#[derive(Serialize, Deserialize)]
pub struct HostInfo {
    pub protocol: u32,
    pub version: String,
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
}

impl Block {
    pub fn new(kind: Kind, text: impl Into<String>) -> Self {
        Self { kind, text: text.into(), tool: None }
    }
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

// Resets the client's view of a session. The transcript along the current path
// and the step in progress follow as ordinary commits and live events.
#[derive(Serialize, Deserialize)]
pub struct Snapshot {
    pub id: String,
    pub cwd: PathBuf,
    pub state: State,
    pub skills: Vec<Skill>,
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

#[derive(Serialize, Deserialize)]
pub struct SessionSummary {
    pub id: String,
    pub title: String,
    pub cwd: PathBuf,
    pub status: Status,
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
    // Takes the newest queued message back for editing.
    RecallQueued { session_id: String } => "/recall_queued" -> Option<String>;
    SetModel { session_id: String, model: Model } => "/set_model" -> ();
    GetTree { session_id: String } => "/get_tree" -> Vec<TreeEntry>;
    // Checking out a user message moves to its parent and returns its text for editing.
    Checkout { session_id: String, node: Option<usize> } => "/checkout" -> Option<String>;
    // `refresh` rebuilds the host's file index; set it when a mention is first opened.
    CompleteMention { session_id: String, text: String, cursor: usize, refresh: bool }
        => "/complete_mention" -> Option<Mention>;
}

// Starts with a snapshot, then streams changes until the client leaves. A
// client that falls too far behind is cut off and may subscribe again.
#[derive(Serialize, Deserialize)]
pub struct Subscribe {
    pub session_id: String,
}

impl Subscribe {
    pub const PATH: &'static str = "/subscribe";
}
