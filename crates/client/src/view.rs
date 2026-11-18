use mu_wire::{Block, Event, Kind, Live, Profile, SessionSummary, Skill, State, Status, Touched, WatchEvent};
use std::{mem, path::PathBuf};

#[derive(Default)]
pub struct View {
    pub id: String,
    pub cwd: PathBuf,
    pub epoch: u64,
    pub state: State,
    pub skills: Vec<Skill>,
    pub blocks: Vec<Block>,
    pub live: Live,
}

pub enum Change {
    Reset { keep: usize },
    State { started: bool },
    Committed { from: usize },
    Live(Touched),
    Feedback(Block),
}

impl View {
    pub fn running(&self) -> bool {
        self.state.status == Status::Running
    }

    pub fn last_text(&self, kind: Kind) -> Option<String> {
        self.blocks.iter().rev().find(|block| block.kind == kind).map(|block| block.text.clone())
    }

    pub fn apply(&mut self, event: Event) -> Change {
        match event {
            Event::Snapshot(snapshot) => {
                let mut blocks = mem::take(&mut self.blocks);
                blocks.truncate(snapshot.keep);
                *self = Self {
                    id: snapshot.id,
                    cwd: snapshot.cwd,
                    epoch: snapshot.epoch,
                    state: snapshot.state,
                    skills: snapshot.skills,
                    blocks,
                    live: Live::default(),
                };
                Change::Reset { keep: snapshot.keep }
            }
            Event::State(state) => {
                let was_running = self.running();
                self.state = state;
                Change::State { started: self.running() && !was_running }
            }
            Event::Commit { blocks } => {
                let from = self.blocks.len();
                self.blocks.extend(blocks);
                self.live.clear();
                Change::Committed { from }
            }
            Event::Live(event) => Change::Live(self.live.apply(&event)),
            Event::Feedback(block) => Change::Feedback(block),
        }
    }
}

#[derive(Default)]
pub struct HostView {
    pub profile: Profile,
    pub sessions: Vec<SessionSummary>,
}

impl HostView {
    pub fn apply(&mut self, event: WatchEvent) {
        match event {
            WatchEvent::Host { profile, sessions } => *self = Self { profile, sessions },
            WatchEvent::Profile(profile) => self.profile = profile,
            WatchEvent::Session(summary) => {
                self.sessions.retain(|session| session.id != summary.id);
                let at = self.sessions.partition_point(|session| session.updated >= summary.updated);
                self.sessions.insert(at, summary);
            }
        }
    }
}
