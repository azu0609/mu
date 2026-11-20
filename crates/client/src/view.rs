use mu_wire::{Block, Event, Kind, Live, Profile, SessionSummary, Skill, State, Status, Touched, WatchEvent};
use std::{mem, ops::Range, path::PathBuf};

#[derive(Default)]
pub struct View {
    pub id: String,
    pub cwd: PathBuf,
    pub epoch: u64,
    pub state: State,
    pub skills: Vec<Skill>,
    pub blocks: Vec<Block>,
    pub path: Vec<PathNode>,
    pub live: Live,
}

pub struct PathNode {
    pub id: u64,
    pub siblings: Vec<u64>,
    pub blocks: Range<usize>,
    pub finished: Option<u64>,
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

    pub fn unread(&self, node: &PathNode) -> bool {
        node.finished.is_some() && node.id >= self.state.unread_from
    }

    pub fn has_unread(&self) -> bool {
        self.path.iter().any(|node| self.unread(node))
    }

    pub fn last_text(&self, kind: Kind) -> Option<String> {
        self.blocks.iter().rev().find(|block| block.kind == kind).map(|block| block.text.clone())
    }

    pub fn apply(&mut self, event: Event) -> Change {
        match event {
            Event::Snapshot(snapshot) => {
                let keep = snapshot.keep as usize;
                let mut blocks = mem::take(&mut self.blocks);
                blocks.truncate(keep);
                let mut path = mem::take(&mut self.path);
                path.retain(|node| node.blocks.start < keep);
                *self = Self {
                    id: snapshot.id,
                    cwd: snapshot.cwd,
                    epoch: snapshot.epoch,
                    state: snapshot.state,
                    skills: snapshot.skills,
                    blocks,
                    path,
                    live: Live::default(),
                };
                Change::Reset { keep }
            }
            Event::State(state) => {
                let was_running = self.running();
                self.state = state;
                Change::State { started: self.running() && !was_running }
            }
            Event::Commit { nodes } => {
                let from = self.path.len();
                for node in nodes {
                    let start = self.blocks.len();
                    self.blocks.extend(node.blocks);
                    self.path.push(PathNode {
                        id: node.id,
                        siblings: node.siblings,
                        blocks: start..self.blocks.len(),
                        finished: node.finished,
                    });
                }
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
