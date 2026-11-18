// The calls that answer with events for as long as the client listens, one
// JSON line each.
use crate::{
    host::{Change, Host, Rpc, directory, failed, internal},
    tools,
};
use hyper::body::{Body, Bytes, Frame};
use mu_wire::{self as wire, Event, Node, ShellEvent, WatchEvent};
use serde::Serialize;
use std::{
    convert::Infallible,
    mem,
    pin::Pin,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    task::{Context, Poll},
    thread,
};
use tokio::sync::{broadcast, mpsc};

pub struct Stream {
    rx: mpsc::Receiver<Bytes>,
    close: Option<Box<dyn FnOnce() + Send>>,
}

impl Body for Stream {
    type Data = Bytes;
    type Error = Infallible;

    fn poll_frame(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Option<std::result::Result<Frame<Bytes>, Infallible>>> {
        self.rx.poll_recv(cx).map(|line| line.map(|line| Ok(Frame::data(line))))
    }
}

impl Drop for Stream {
    fn drop(&mut self) {
        if let Some(close) = self.close.take() {
            close();
        }
    }
}

fn line(value: &impl Serialize) -> Option<Bytes> {
    let mut line = serde_json::to_vec(value).ok()?;
    line.push(b'\n');
    Some(line.into())
}

// A commit can carry a whole transcript, a snapshot's, so it goes out in bounded
// chunks: no line grows with the session.
fn lines(event: &Event) -> Option<Vec<Bytes>> {
    const CHUNK: usize = 256 * 1024;
    let size = |node: &Node| -> usize {
        node.blocks
            .iter()
            .map(|block| block.text.len() + block.tool.as_ref().map_or(0, |tool| tool.output.text.len()))
            .sum()
    };
    let Event::Commit { nodes } = event else {
        return Some(vec![line(event)?]);
    };
    if nodes.iter().map(size).sum::<usize>() < CHUNK {
        return Some(vec![line(event)?]);
    }
    let mut lines = vec![];
    let mut chunk = vec![];
    let mut chunk_size = 0;
    for node in nodes {
        chunk_size += size(node);
        chunk.push(node.clone());
        if chunk_size >= CHUNK {
            lines.push(line(&Event::Commit { nodes: mem::take(&mut chunk) })?);
            chunk_size = 0;
        }
    }
    if !chunk.is_empty() {
        lines.push(line(&Event::Commit { nodes: chunk })?);
    }
    Some(lines)
}

pub fn subscribe(host: &Arc<Host>, call: wire::Subscribe) -> Rpc<Stream> {
    let (tx, rx) = mpsc::channel(1024);
    let mut agents = host.agents.lock().unwrap();
    let agent = host.load(&mut agents, &call.session_id)?;
    let subscription = agent.subscribe(call.resume, move |event| {
        lines(event).is_some_and(|lines| lines.into_iter().all(|line| tx.try_send(line).is_ok()))
    });
    let host = host.clone();
    let close = move || {
        agent.unsubscribe(subscription);
        host.sweep();
    };
    Ok(Stream { rx, close: Some(Box::new(close)) })
}

pub async fn watch(host: &Arc<Host>) -> Rpc<Stream> {
    // Listening before listing, so no change falls in between.
    let mut changes = host.changes.subscribe();
    let everything = host.run(|host| host.everything()).await?;
    let (tx, rx) = mpsc::channel(64);
    let first = line(&everything).ok_or_else(|| internal("A session path is not UTF-8"))?;
    let _ = tx.try_send(first);
    let host = host.clone();
    tokio::spawn(async move {
        loop {
            let event = tokio::select! {
                _ = tx.closed() => return,
                change = changes.recv() => match change {
                    Ok(Change::Session(id)) => match host.run(move |host| Ok(host.summary(&id))).await {
                        Ok(Some(summary)) => WatchEvent::Session(summary),
                        _ => continue,
                    },
                    Ok(Change::Profile) => WatchEvent::Profile(host.profile.get()),
                    Ok(Change::Sessions) | Err(broadcast::error::RecvError::Lagged(_)) => {
                        match host.run(|host| host.everything()).await {
                            Ok(everything) => everything,
                            Err(_) => return,
                        }
                    }
                    Err(broadcast::error::RecvError::Closed) => return,
                },
            };
            let Some(line) = line(&event) else { return };
            if tx.send(line).await.is_err() {
                return;
            }
        }
    });
    Ok(Stream { rx, close: None })
}

pub fn shell(host: &Arc<Host>, call: wire::Shell) -> Rpc<Stream> {
    directory(&call.cwd)?;
    let command = call.command.trim().to_owned();
    if command.is_empty() || command.contains('\0') {
        return Err(failed("Not a command"));
    }
    let (tx, rx) = mpsc::channel(16);
    let cancel = Arc::new(AtomicBool::new(false));
    let (host, stop) = (host.clone(), cancel.clone());
    thread::spawn(move || {
        let run = tools::user(command, call.cwd, &host.bridge.open(None), &cancel, |output| {
            if let Some(line) = line(&ShellEvent::Output(output)) {
                let _ = tx.try_send(line);
            }
        });
        let (status, output) = (run.status, run.output.clone());
        let done = ShellEvent::Done { run: host.keep(run), status, output };
        if let Some(line) = line(&done) {
            let _ = tx.blocking_send(line);
        }
    });
    Ok(Stream { rx, close: Some(Box::new(move || stop.store(true, Ordering::Relaxed))) })
}
