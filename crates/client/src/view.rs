// A session as this client last heard it from the host.
use mu_wire::{Block, Event, Kind, Model, Skill, State, Stats, Status};
use std::path::PathBuf;

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
    // Everything was replaced.
    Reset,
    // `started` when the agent just began running.
    State { started: bool },
    // Blocks were appended to the transcript, and the live step ended.
    Committed,
    Live,
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
                *self = Self { id: snapshot.id, cwd: snapshot.cwd, skills: snapshot.skills, ..Self::default() };
                self.set_state(snapshot.state);
                Change::Reset
            }
            Event::State(state) => {
                let was_running = self.running();
                self.set_state(state);
                Change::State { started: self.running() && !was_running }
            }
            Event::Commit { blocks } => {
                self.blocks.extend(blocks);
                self.live.clear();
                self.live_notice = None;
                Change::Committed
            }
            Event::LivePush(block) => {
                self.live.push(block);
                Change::Live
            }
            Event::LiveDelta { index, text } => {
                if let Some(block) = self.live.get_mut(index) {
                    block.text.push_str(&text);
                }
                Change::Live
            }
            Event::LiveSet { index, block } => {
                if let Some(slot) = self.live.get_mut(index) {
                    *slot = block;
                }
                Change::Live
            }
            Event::LiveOutput { index, output } => {
                if let Some(tool) = self.live.get_mut(index).and_then(|block| block.tool.as_mut()) {
                    tool.output = output;
                }
                Change::Live
            }
            Event::LiveNotice(block) => {
                self.live_notice = Some(block);
                Change::Live
            }
            Event::Feedback(block) => Change::Feedback(block),
        }
    }
}
