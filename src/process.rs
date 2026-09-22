use crate::Result;
use std::os::unix::process::CommandExt;
use std::{
    io::{Read, Write},
    process::{Child, Command, Stdio},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    thread,
    time::{Duration, Instant},
};

struct Group(Child, bool);
impl Drop for Group {
    fn drop(&mut self) {
        // Include grandchildren, even when the shell has already exited.
        if self.1 {
            unsafe {
                libc::kill(-(self.0.id() as i32), libc::SIGKILL);
            }
        }
        let _ = self.0.wait();
    }
}

pub struct Exit {
    pub code: i32,
    pub timed_out: bool,
    pub cancelled: bool,
}

pub fn run(
    command: &mut Command,
    input: Option<Vec<u8>>,
    timeout: Duration,
    cancel: &Arc<AtomicBool>,
    mut chunk: impl FnMut(bool, &[u8]) -> Result<()>,
) -> Result<Exit> {
    if cancel.load(Ordering::Relaxed) {
        return Ok(Exit { code: -1, timed_out: false, cancelled: true });
    }
    let mut child = Group(
        command
            .process_group(0)
            .stdin(if input.is_some() { Stdio::piped() } else { Stdio::null() })
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()?,
        true,
    );
    if let Some(input) = input {
        let mut stdin = child.0.stdin.take().unwrap();
        thread::spawn(move || {
            let _ = stdin.write_all(&input);
        });
    }
    let (tx, rx) = mpsc::sync_channel(32);
    fn reader(
        mut pipe: impl Read + Send + 'static,
        stderr: bool,
        tx: mpsc::SyncSender<std::io::Result<(bool, Vec<u8>)>>,
    ) {
        thread::spawn(move || {
            let mut buf = [0; 8192];
            loop {
                match pipe.read(&mut buf) {
                    Ok(0) => break,
                    Ok(n) => {
                        if tx.send(Ok((stderr, buf[..n].to_vec()))).is_err() {
                            break;
                        }
                    }
                    Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
                    Err(e) => {
                        let _ = tx.send(Err(e));
                        break;
                    }
                }
            }
        });
    }
    reader(child.0.stdout.take().unwrap(), false, tx.clone());
    reader(child.0.stderr.take().unwrap(), true, tx.clone());
    drop(tx);
    let start = Instant::now();
    loop {
        let cancelled = cancel.load(Ordering::Relaxed);
        let timed_out = start.elapsed() >= timeout;
        if cancelled || timed_out {
            return Ok(Exit { code: -1, timed_out, cancelled });
        }
        match rx.recv_timeout(Duration::from_millis(25)) {
            Ok(data) => {
                let (stderr, bytes) = data?;
                chunk(stderr, &bytes)?;
            }
            Err(mpsc::RecvTimeoutError::Timeout) => (),
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                if let Some(status) = child.0.try_wait()? {
                    child.1 = false;
                    return Ok(Exit { code: status.code().unwrap_or(-1), timed_out: false, cancelled: false });
                }
                thread::sleep(Duration::from_millis(25));
            }
        }
    }
}
