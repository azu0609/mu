// The session as this client last heard it from the host.
use mu_client::proto as pb;
use std::path::PathBuf;

#[derive(Clone, Copy, PartialEq)]
pub enum Kind {
    User,
    Agent,
    Thought,
    Call,
    Notice,
    Warning,
}

pub struct Block {
    pub kind: Kind,
    pub text: String,
    pub tool: Option<Tool>,
}

impl Block {
    pub fn new(kind: Kind, text: impl Into<String>) -> Self {
        Self { kind, text: text.into(), tool: None }
    }
}

pub struct Tool {
    pub status: ToolStatus,
    pub output: ToolOutput,
}

#[derive(Default)]
pub struct ToolOutput {
    pub text: String,
    pub log: Option<String>,
    pub truncated: bool,
}

#[derive(Clone, Copy)]
pub enum ToolStatus {
    Pending,
    Running,
    Exited(i32),
    TimedOut(u64),
    Cancelled,
    Error,
}

impl ToolStatus {
    pub fn summary(self) -> Option<String> {
        Some(match self {
            Self::Exited(0) | Self::Error => return None,
            Self::Pending => "pending".into(),
            Self::Running => "running".into(),
            Self::Exited(code) => format!("exit {code}"),
            Self::TimedOut(ms) => format!("timed out after {ms} ms; process group killed"),
            Self::Cancelled => "cancelled; process group killed".into(),
        })
    }
}

#[derive(Clone, Copy, Default, PartialEq)]
pub enum Status {
    #[default]
    Idle,
    Running,
    Finished,
    Failed,
}

impl Status {
    pub fn marker(self) -> &'static str {
        match self {
            Self::Idle => "",
            Self::Running => "… ",
            Self::Finished => "✓ ",
            Self::Failed => "! ",
        }
    }
}

pub struct Skill {
    pub name: String,
    pub description: String,
}

#[derive(Default)]
pub struct View {
    pub id: String,
    pub cwd: PathBuf,
    pub status: Status,
    pub model: pb::Model,
    pub stats: pb::Stats,
    pub queued: Vec<String>,
    pub skills: Vec<Skill>,
    pub blocks: Vec<Block>,
    pub live: Vec<Block>,
    pub live_notice: Option<Block>,
}

impl From<pb::ToolOutput> for ToolOutput {
    fn from(output: pb::ToolOutput) -> Self {
        Self { text: output.text, log: output.log, truncated: output.truncated }
    }
}

// Values this client does not know yet degrade to the plainest rendering.
impl From<pb::Block> for Block {
    fn from(block: pb::Block) -> Self {
        Self {
            kind: match block.kind.as_known() {
                Some(pb::BlockKind::User) => Kind::User,
                Some(pb::BlockKind::Agent) => Kind::Agent,
                Some(pb::BlockKind::Thought) => Kind::Thought,
                Some(pb::BlockKind::Call) if block.tool.is_set() => Kind::Call,
                Some(pb::BlockKind::Warning) => Kind::Warning,
                _ => Kind::Notice,
            },
            text: block.text,
            tool: block.tool.into_option().map(|tool| Tool {
                status: match tool.state.as_known() {
                    Some(pb::ToolState::Running) => ToolStatus::Running,
                    Some(pb::ToolState::Exited) => ToolStatus::Exited(tool.exit_code),
                    Some(pb::ToolState::TimedOut) => ToolStatus::TimedOut(tool.timeout_ms),
                    Some(pb::ToolState::Cancelled) => ToolStatus::Cancelled,
                    Some(pb::ToolState::Error) => ToolStatus::Error,
                    _ => ToolStatus::Pending,
                },
                output: tool.output.into_option().unwrap_or_default().into(),
            }),
        }
    }
}

pub fn status(status: buffa::EnumValue<pb::SessionStatus>) -> Status {
    match status.as_known() {
        Some(pb::SessionStatus::Running) => Status::Running,
        Some(pb::SessionStatus::Finished) => Status::Finished,
        Some(pb::SessionStatus::Failed) => Status::Failed,
        _ => Status::Idle,
    }
}

impl View {
    pub fn running(&self) -> bool {
        self.status == Status::Running
    }

    pub fn set_state(&mut self, state: pb::SessionState) {
        self.status = status(state.status);
        self.model = state.model.into_option().unwrap_or_default();
        self.stats = state.stats.into_option().unwrap_or_default();
        self.queued = state.queued;
    }

    pub fn reset(&mut self, snapshot: pb::Snapshot) {
        *self = Self {
            id: snapshot.id,
            cwd: snapshot.cwd.into(),
            skills: snapshot
                .skills
                .into_iter()
                .map(|skill| Skill { name: skill.name, description: skill.description })
                .collect(),
            ..Self::default()
        };
        self.set_state(snapshot.state.into_option().unwrap_or_default());
    }

    pub fn last_text(&self, kind: Kind) -> Option<String> {
        self.blocks.iter().rev().find(|block| block.kind == kind).map(|block| block.text.clone())
    }
}
