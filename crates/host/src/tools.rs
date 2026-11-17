use crate::{Result, bridge, now, process, unique_id};
use mu_wire::{Span, ToolOutput, ToolStatus};
use serde::Deserialize;
use serde_json::Value;
use std::{
    collections::VecDeque,
    env, fs,
    io::{BufWriter, Write},
    os::unix::fs::OpenOptionsExt,
    path::{Path, PathBuf},
    process::Command,
    sync::{Arc, atomic::AtomicBool},
    time::{Duration, Instant},
};

const PREVIEW_LIMIT: usize = 64 * 1024;

pub struct BashResult {
    pub output: ToolOutput,
    pub status: ToolStatus,
    pub images: Vec<Value>,
}

#[derive(Default)]
struct Capture {
    head: Vec<u8>,
    tail: VecDeque<u8>,
    total: usize,
    omitted_newlines: usize,
    omitted_last: u8,
    log: Option<(PathBuf, BufWriter<fs::File>)>,
}

impl Capture {
    fn push(&mut self, bytes: &[u8]) -> Result<()> {
        if self.log.is_none() && self.total.saturating_add(bytes.len()) > PREVIEW_LIMIT {
            let path = env::temp_dir().canonicalize()?.join(format!("mu-output-{}.log", unique_id()));
            let mut file = BufWriter::new(fs::OpenOptions::new().write(true).create_new(true).mode(0o600).open(&path)?);
            file.write_all(&self.head)?;
            let (a, b) = self.tail.as_slices();
            file.write_all(a)?;
            file.write_all(b)?;
            self.log = Some((path, file));
        }
        if let Some((_, file)) = &mut self.log {
            file.write_all(bytes)?;
        }
        self.total = self.total.saturating_add(bytes.len());
        let n = bytes.len().min(PREVIEW_LIMIT / 2 - self.head.len());
        self.head.extend_from_slice(&bytes[..n]);
        let rest = &bytes[n..];
        let excess = (self.tail.len() + rest.len()).saturating_sub(PREVIEW_LIMIT / 2);
        let from_tail = excess.min(self.tail.len());
        let from_rest = excess - from_tail;
        for byte in self.tail.drain(..from_tail).chain(rest[..from_rest].iter().copied()) {
            self.omitted_newlines += usize::from(byte == b'\n');
            self.omitted_last = byte;
        }
        self.tail.extend(&rest[from_rest..]);
        Ok(())
    }

    fn flush(&mut self) -> Result<()> {
        if let Some((_, file)) = &mut self.log {
            file.flush()?;
        }
        Ok(())
    }

    fn preview(&self) -> ToolOutput {
        let truncated = self.total > PREVIEW_LIMIT;
        let mut bytes = self.head.clone();
        if truncated {
            // Count an unfinished line too: even output without newlines can
            // lose part of a line between the retained head and tail.
            let lines = self.omitted_newlines + usize::from(self.omitted_last != b'\n');
            bytes.extend_from_slice(
                format!(
                    "\n[… {} bytes ({} {}) omitted …]\n",
                    self.total - PREVIEW_LIMIT,
                    lines,
                    if lines == 1 { "line" } else { "lines" }
                )
                .as_bytes(),
            );
        }
        bytes.extend(&self.tail);
        ToolOutput {
            text: String::from_utf8_lossy(&bytes).into_owned(),
            log: self.log.as_ref().map(|(path, _)| path.clone()),
            truncated,
        }
    }
}

// The special commands go first on PATH after direnv, which rebuilds PATH from what it saw before
// loading, or unloads an environment the host inherited.
const PREAMBLE: &str = r#"export -n MU_COMMAND MU_BIN
command -v direnv >/dev/null && eval "$(DIRENV_LOG_FORMAT= direnv export bash)"
PATH="$MU_BIN:$PATH"
eval "$MU_COMMAND""#;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Args {
    command: String,
    #[serde(rename = "timeoutMs")]
    timeout_ms: Option<u64>,
}

pub fn bash(
    arguments: &str,
    cwd: &Path,
    run: &bridge::Run,
    cancel: &Arc<AtomicBool>,
    progress: impl FnMut(ToolOutput),
) -> Result<BashResult> {
    let args: Args = serde_json::from_str(arguments)?;
    let timeout = args.timeout_ms.unwrap_or(120_000);
    if timeout == 0 {
        return Err("timeoutMs must be positive".into());
    }
    execute(&args.command, cwd, Some(timeout), run, false, cancel, progress)
}

pub struct Run {
    pub command: String,
    pub cwd: PathBuf,
    pub span: Span,
    pub status: ToolStatus,
    pub output: ToolOutput,
    pub images: Vec<Value>,
}

pub fn user(
    command: String,
    cwd: PathBuf,
    run: &bridge::Run,
    cancel: &Arc<AtomicBool>,
    mut progress: impl FnMut(ToolOutput),
) -> Run {
    let started = now();
    let mut shown = ToolOutput::default();
    let result = execute(&command, &cwd, None, run, true, cancel, |output| {
        shown = output.clone();
        progress(output);
    });
    let (status, output, images) = match result {
        Ok(result) => (result.status, result.output, result.images),
        Err(e) => {
            shown.text.push_str(&format!("\n[error: {e}]"));
            (ToolStatus::Error, shown, vec![])
        }
    };
    Run { command, cwd, span: Span { started, finished: now() }, status, output, images }
}

fn execute(
    command: &str,
    cwd: &Path,
    timeout: Option<u64>,
    run: &bridge::Run,
    user: bool,
    cancel: &Arc<AtomicBool>,
    mut progress: impl FnMut(ToolOutput),
) -> Result<BashResult> {
    let mut capture = Capture::default();
    let mut updated = Instant::now();
    let mut published = 0;
    let mut bash = Command::new("bash");
    // Keep the command out of argv so `pkill -f` and friends can't match their own shell.
    bash.args(["-c", PREAMBLE]).current_dir(cwd).env("MU_COMMAND", command).envs(run.env());
    match user {
        true => bash.env("MU_USER_TRIGGERED", "1"),
        false => bash.env_remove("MU_USER_TRIGGERED"),
    };
    let exit =
        process::run(&mut bash, None, timeout.map_or(Duration::MAX, Duration::from_millis), cancel, |_, chunk| {
            capture.push(chunk)?;
            if capture.total != published && (published == 0 || updated.elapsed() >= Duration::from_millis(100)) {
                capture.flush()?;
                progress(capture.preview());
                published = capture.total;
                updated = Instant::now();
            }
            Ok(())
        });
    let flushed = capture.flush();
    let output = capture.preview();
    progress(output.clone());
    flushed.map_err(|e| format!("Cannot flush output log: {e}"))?;
    let exit = exit?;
    let status = if exit.cancelled {
        ToolStatus::Cancelled
    } else if exit.timed_out {
        ToolStatus::TimedOut(timeout.unwrap_or_default())
    } else {
        ToolStatus::Exited(exit.code)
    };
    Ok(BashResult { output, status, images: run.images() })
}
