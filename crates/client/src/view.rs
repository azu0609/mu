// What this client last heard from a host: a session, and the session list.
use mu_wire::{Block, Event, Kind, Model, SessionSummary, Skill, State, Stats, Status, WatchEvent};
use std::{mem, path::PathBuf};

#[derive(Default)]
pub struct View {
    pub id: String,
    pub cwd: PathBuf,
    pub title: String,
    pub status: Status,
    pub model: Model,
    pub stats: Stats,
    pub queued: Vec<String>,
    pub skills: Vec<Skill>,
    pub blocks: Vec<Block>,
    pub live: Vec<Block>,
    pub live_notice: Option<Block>,
}

// What an event did to the view, for a client to react to.
pub enum Change {
    // Everything was replaced but the first `keep` blocks of the transcript.
    Reset { keep: usize },
    // `started` when the agent just began running.
    State { started: bool },
    // Blocks from this index on were appended to the transcript, and the live step ended.
    Committed { from: usize },
    // The live block at this index was added or changed.
    Live(usize),
    LiveNotice,
    // Transient: not part of the session, so not kept in the view.
    Feedback(Block),
}

impl View {
    pub fn running(&self) -> bool {
        self.status == Status::Running
    }

    pub fn last_text(&self, kind: Kind) -> Option<String> {
        self.blocks.iter().rev().find(|block| block.kind == kind).map(|block| block.text.clone())
    }

    fn set_state(&mut self, state: State) {
        self.status = state.status;
        self.title = state.title;
        self.model = state.model;
        self.stats = state.stats;
        self.queued = state.queued;
    }

    pub fn apply(&mut self, event: Event) -> Change {
        match event {
            Event::Snapshot(snapshot) => {
                let mut blocks = mem::take(&mut self.blocks);
                blocks.truncate(snapshot.keep);
                *self = Self { id: snapshot.id, cwd: snapshot.cwd, skills: snapshot.skills, blocks, ..Self::default() };
                self.set_state(snapshot.state);
                Change::Reset { keep: snapshot.keep }
            }
            Event::State(state) => {
                let was_running = self.running();
                self.set_state(state);
                Change::State { started: self.running() && !was_running }
            }
            Event::Commit { blocks } => {
                let from = self.blocks.len();
                self.blocks.extend(blocks);
                self.live.clear();
                self.live_notice = None;
                Change::Committed { from }
            }
            Event::LivePush(block) => {
                self.live.push(block);
                Change::Live(self.live.len() - 1)
            }
            Event::LiveDelta { index, text } => {
                if let Some(block) = self.live.get_mut(index) {
                    block.text.push_str(&text);
                }
                Change::Live(index)
            }
            Event::LiveSet { index, block } => {
                if let Some(slot) = self.live.get_mut(index) {
                    *slot = block;
                }
                Change::Live(index)
            }
            Event::LiveOutput { index, output } => {
                if let Some(tool) = self.live.get_mut(index).and_then(|block| block.tool.as_mut()) {
                    tool.output = output;
                }
                Change::Live(index)
            }
            Event::LiveNotice(block) => {
                self.live_notice = Some(block);
                Change::LiveNotice
            }
            Event::Feedback(block) => Change::Feedback(block),
        }
    }
}

// A host's sessions, latest first.
#[derive(Default)]
pub struct SessionList {
    pub sessions: Vec<SessionSummary>,
}

impl SessionList {
    pub fn apply(&mut self, event: WatchEvent) {
        match event {
            WatchEvent::Sessions { sessions } => self.sessions = sessions,
            WatchEvent::Session(summary) => {
                self.sessions.retain(|session| session.id != summary.id);
                let at = self.sessions.partition_point(|session| session.updated >= summary.updated);
                self.sessions.insert(at, summary);
            }
        }
    }
}
