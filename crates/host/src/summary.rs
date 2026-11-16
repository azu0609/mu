// What the list of sessions shows of each, read from the ends of its file
// without loading it.
use crate::{
    Result, config,
    session::{self, End, Header, Node, Record},
};
use mu_wire::Status;
use std::{
    fs,
    io::{self, BufRead, BufReader, Read, Seek, SeekFrom},
    path::{Path, PathBuf},
    time::UNIX_EPOCH,
};

pub struct Summary {
    pub id: String,
    pub title: String,
    pub cwd: PathBuf,
    pub status: Status,
    pub error: Option<String>,
    pub unread: bool,
    pub updated: u64,
}

pub fn summary_of(id: &str) -> Option<Summary> {
    summarize(&session::path(id).ok()?)
}

fn summarize(path: &Path) -> Option<Summary> {
    let mut file = fs::File::open(path).ok()?;
    let mut line = String::new();
    BufReader::new(&file).read_line(&mut line).ok()?;
    if !line.ends_with('\n') {
        return None;
    }
    let header: Header = serde_json::from_str(&line).ok()?;
    if header.version != 1 {
        return None;
    }
    let tail = tail(&mut file).ok()?;
    let (node, read) = (tail.node, tail.read);
    let status = node.as_ref().filter(|_| !tail.moved).map_or(Status::Idle, Node::status);
    let error = match node.as_ref().and_then(|node| node.end.as_ref()) {
        Some(End::Failed { error }) if status == Status::Failed => Some(error.clone()),
        _ => None,
    };
    let modified = || {
        let modified = file.metadata().and_then(|m| m.modified()).ok()?;
        Some(modified.duration_since(UNIX_EPOCH).ok()?.as_millis() as u64)
    };
    Some(Summary {
        id: header.id,
        title: header.title,
        cwd: header.cwd,
        status,
        error,
        unread: node.is_some() && !read,
        updated: node.and_then(|node| node.span).map(|span| span.finished).or_else(modified).unwrap_or(0),
    })
}

struct Tail {
    node: Option<Node>,
    read: bool,
    moved: bool,
}

fn tail(file: &mut fs::File) -> io::Result<Tail> {
    let len = file.metadata()?.len();
    let mut window = 16 * 1024;
    loop {
        let start = len.saturating_sub(window);
        file.seek(SeekFrom::Start(start))?;
        let mut bytes = vec![];
        file.take(len - start).read_to_end(&mut bytes)?;
        // Complete lines only: not an unfinished last one, nor one the window cuts into.
        let end = bytes.iter().rposition(|&b| b == b'\n').map_or(0, |i| i + 1);
        let mut lines: Vec<_> = bytes[..end].split(|&b| b == b'\n').collect();
        lines.pop();
        let complete = if start == 0 { &lines[..] } else { lines.get(1..).unwrap_or_default() };
        let (mut read, mut moved) = (false, false);
        for line in complete.iter().rev() {
            // serde writes the tag first.
            if line.starts_with(br#"{"type":"read""#) {
                read = true;
            } else if line.starts_with(br#"{"type":"cursor""#) {
                moved = true;
            } else if line.starts_with(br#"{"type":"node""#) {
                let Ok(Record::Node(node)) = serde_json::from_slice(line) else {
                    return Err(io::ErrorKind::InvalidData.into());
                };
                return Ok(Tail { node: Some(node), read, moved });
            }
        }
        if start == 0 {
            return Ok(Tail { node: None, read, moved });
        }
        window *= 4;
    }
}

pub fn sessions() -> Result<Vec<Summary>> {
    let dir = &config::state_dir();
    if !dir.exists() {
        return Ok(vec![]);
    }
    let mut sessions: Vec<_> = fs::read_dir(dir)?
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|e| e == "jsonl"))
        .filter_map(|path| summarize(&path))
        .collect();
    sessions.sort_by_key(|summary| std::cmp::Reverse(summary.updated));
    Ok(sessions)
}
