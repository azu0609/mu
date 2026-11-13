// mu-core's types as they appear on the wire.
use mu_core::{
    agent::{Event, Snapshot, State, Status},
    session::{self, Kind, ToolStatus},
};
use mu_proto as pb;

pub fn status(status: Status) -> pb::SessionStatus {
    match status {
        Status::Idle => pb::SessionStatus::Idle,
        Status::Running => pb::SessionStatus::Running,
        Status::Finished => pb::SessionStatus::Finished,
        Status::Failed => pb::SessionStatus::Failed,
    }
}

fn output(output: &session::ToolOutput) -> pb::ToolOutput {
    pb::ToolOutput {
        text: output.text.clone(),
        log: output.log.as_ref().map(|log| log.to_string_lossy().into_owned()),
        truncated: output.truncated,
        ..Default::default()
    }
}

fn tool(tool: &session::Tool) -> pb::Tool {
    let (state, exit_code, timeout_ms) = match tool.status {
        ToolStatus::Pending => (pb::ToolState::Pending, 0, 0),
        ToolStatus::Running => (pb::ToolState::Running, 0, 0),
        ToolStatus::Exited(code) => (pb::ToolState::Exited, code, 0),
        ToolStatus::TimedOut(ms) => (pb::ToolState::TimedOut, 0, ms),
        ToolStatus::Cancelled => (pb::ToolState::Cancelled, 0, 0),
        ToolStatus::Error => (pb::ToolState::Error, 0, 0),
    };
    pb::Tool {
        call_id: tool.call_id.clone(),
        state: state.into(),
        exit_code,
        timeout_ms,
        output: output(&tool.output).into(),
        ..Default::default()
    }
}

fn block(block: &session::Block) -> pb::Block {
    pb::Block {
        kind: match block.kind {
            Kind::User => pb::BlockKind::User,
            Kind::Agent => pb::BlockKind::Agent,
            Kind::Thought => pb::BlockKind::Thought,
            Kind::Call => pb::BlockKind::Call,
            Kind::Notice => pb::BlockKind::Notice,
            Kind::Warning => pb::BlockKind::Warning,
        }
        .into(),
        text: block.text.clone(),
        tool: block.tool.as_ref().map(tool).into(),
        ..Default::default()
    }
}

fn state(state: &State) -> pb::SessionState {
    pb::SessionState {
        status: status(state.status).into(),
        title: state.title.clone(),
        model: pb::Model {
            name: state.model.name.clone(),
            effort: state.model.effort.clone(),
            context: state.model.context,
            ..Default::default()
        }
        .into(),
        stats: pb::Stats {
            input: state.stats.input,
            output: state.stats.output,
            cached: state.stats.cached,
            uncached_input: state.stats.uncached_input,
            context_used: state.stats.context_used,
            ..Default::default()
        }
        .into(),
        queued: state.queued.clone(),
        ..Default::default()
    }
}

fn wrap(event: pb::Event) -> pb::SessionEvent {
    pb::SessionEvent { event: Some(event), ..Default::default() }
}

fn commit(blocks: &[session::Block]) -> pb::SessionEvent {
    wrap(pb::Event::Commit(pb::Commit { blocks: blocks.iter().map(block).collect(), ..Default::default() }.into()))
}

// A snapshot goes out as a reset followed by the transcript in bounded
// commits and the live step, so no message grows with the session.
fn snapshot(snapshot: &Snapshot, events: &mut Vec<pb::SessionEvent>) {
    const CHUNK: usize = 256 * 1024;
    events.push(wrap(pb::Event::Snapshot(
        pb::Snapshot {
            id: snapshot.id.clone(),
            cwd: snapshot.cwd.to_string_lossy().into_owned(),
            state: state(&snapshot.state).into(),
            skills: snapshot
                .skills
                .iter()
                .map(|skill| pb::Skill {
                    name: skill.name.clone(),
                    description: skill.description.clone(),
                    ..Default::default()
                })
                .collect(),
            ..Default::default()
        }
        .into(),
    )));
    let mut start = 0;
    let mut size = 0;
    for (i, block) in snapshot.blocks.iter().enumerate() {
        size += block.text.len() + block.tool.as_ref().map_or(0, |tool| tool.output.text.len());
        if size >= CHUNK {
            events.push(commit(&snapshot.blocks[start..=i]));
            start = i + 1;
            size = 0;
        }
    }
    if start < snapshot.blocks.len() {
        events.push(commit(&snapshot.blocks[start..]));
    }
    events.extend(snapshot.live.iter().map(|b| wrap(pb::Event::LivePush(block(b).into()))));
    events.extend(snapshot.live_notice.iter().map(|b| wrap(pb::Event::LiveNotice(block(b).into()))));
}

pub fn events(event: &Event) -> Vec<pb::SessionEvent> {
    let event = match event {
        Event::Snapshot(s) => {
            let mut events = vec![];
            snapshot(s, &mut events);
            return events;
        }
        Event::State(s) => pb::Event::State(state(s).into()),
        Event::Commit(blocks) => return vec![commit(blocks)],
        Event::LivePush(b) => pb::Event::LivePush(block(b).into()),
        Event::LiveDelta(i, text) => {
            pb::Event::LiveDelta(pb::LiveDelta { index: *i as u32, text: text.clone(), ..Default::default() }.into())
        }
        Event::LiveSet(i, b) => {
            pb::Event::LiveSet(pb::LiveSet { index: *i as u32, block: block(b).into(), ..Default::default() }.into())
        }
        Event::LiveOutput(i, o) => pb::Event::LiveOutput(
            pb::LiveOutput { index: *i as u32, output: output(o).into(), ..Default::default() }.into(),
        ),
        Event::LiveNotice(b) => pb::Event::LiveNotice(block(b).into()),
        Event::Feedback(b) => pb::Event::Feedback(block(b).into()),
    };
    vec![wrap(event)]
}
