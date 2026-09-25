use crate::{
    Result, process,
    session::{ToolOutput, ToolStatus, unique_id},
};
use base64::{Engine, engine::general_purpose::STANDARD};
use serde::Deserialize;
use serde_json::{Value, json};
use std::{
    collections::VecDeque,
    env, fs,
    io::{BufWriter, Write},
    os::unix::fs::{OpenOptionsExt, PermissionsExt},
    path::{Path, PathBuf},
    process::Command,
    sync::{Arc, atomic::AtomicBool},
    time::{Duration, Instant},
};

const PREVIEW_LIMIT: usize = 64 * 1024;
const IMAGE_LIMIT: u64 = 20 * 1024 * 1024;

pub struct BashResult {
    pub output: ToolOutput,
    pub status: ToolStatus,
    pub images: Vec<Value>,
}

// Small results stay in the session. Only overflow needs a separate, temporary
// file; retain the beginning and a rolling tail in the transcript.
#[derive(Default)]
struct Capture {
    head: Vec<u8>,
    tail: VecDeque<u8>,
    total: usize,
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
        if rest.len() >= PREVIEW_LIMIT / 2 {
            self.tail.clear();
            self.tail.extend(&rest[rest.len() - PREVIEW_LIMIT / 2..]);
        } else {
            let excess = (self.tail.len() + rest.len()).saturating_sub(PREVIEW_LIMIT / 2);
            self.tail.drain(..excess);
            self.tail.extend(rest);
        }
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
            bytes.extend_from_slice(format!("\n[… {} bytes omitted …]\n", self.total - PREVIEW_LIMIT).as_bytes());
        }
        bytes.extend(&self.tail);
        ToolOutput {
            text: String::from_utf8_lossy(&bytes).into_owned(),
            log: self.log.as_ref().map(|(path, _)| path.clone()),
            truncated,
        }
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Args {
    command: String,
    #[serde(rename = "timeoutMs")]
    timeout_ms: Option<u64>,
}

struct Temp(PathBuf);

impl Drop for Temp {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

// Called by the tiny `view_image` executable placed on each shell's PATH.
pub fn view_image(path: &Path) -> Result<()> {
    let manifest = env::var_os("MU_IMAGES").ok_or("view_image is only available inside mu's bash tool")?;
    let path = path.canonicalize()?;
    if fs::metadata(&path)?.len() > IMAGE_LIMIT {
        return Err("Image exceeds 20 MiB".into());
    }
    let mut line = serde_json::to_vec(&path)?;
    line.push(b'\n');
    fs::OpenOptions::new().create(true).append(true).open(manifest)?.write_all(&line)?;
    println!("Image queued: {}", path.display());
    Ok(())
}

fn image(path: &Path) -> Result<Value> {
    use std::io::Read;
    let mut bytes = vec![];
    fs::File::open(path)?.take(IMAGE_LIMIT + 1).read_to_end(&mut bytes)?;
    if bytes.len() as u64 > IMAGE_LIMIT {
        return Err("Image exceeds 20 MiB".into());
    }
    let mime = if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        "image/png"
    } else if bytes.starts_with(b"\xff\xd8\xff") {
        "image/jpeg"
    } else if bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a") {
        "image/gif"
    } else if bytes.starts_with(b"RIFF") && bytes.get(8..12) == Some(b"WEBP") {
        "image/webp"
    } else {
        return Err("Expected PNG, JPEG, GIF or WebP".into());
    };
    Ok(json!({"type":"input_image", "image_url": format!("data:{mime};base64,{}", STANDARD.encode(bytes))}))
}

pub fn bash(
    arguments: &str,
    cwd: &Path,
    cancel: &Arc<AtomicBool>,
    mut progress: impl FnMut(ToolOutput),
) -> Result<BashResult> {
    let args: Args = serde_json::from_str(arguments)?;
    let timeout = args.timeout_ms.unwrap_or(120_000);
    if timeout == 0 {
        return Err("timeoutMs must be positive".into());
    }
    let dir = Temp(env::temp_dir().join(format!("mu-{}", unique_id())));
    fs::create_dir(&dir.0)?;
    fs::set_permissions(&dir.0, fs::Permissions::from_mode(0o700))?;
    let bridge = dir.0.join("view_image");
    fs::write(&bridge, "#!/bin/sh\nexec \"$MU_EXE\" --view-image \"$@\"\n")?;
    fs::set_permissions(&bridge, fs::Permissions::from_mode(0o700))?;
    let manifest = dir.0.join("images");
    let path = format!("{}:{}", dir.0.display(), env::var("PATH").unwrap_or_default());
    let mut capture = Capture::default();
    let mut updated = Instant::now();
    let mut published = 0;
    let exit = process::run(
        Command::new("bash")
            .args(["-c", &args.command])
            .current_dir(cwd)
            .env("PATH", path)
            .env("MU_EXE", env::current_exe()?)
            .env("MU_IMAGES", &manifest),
        None,
        Duration::from_millis(timeout),
        cancel,
        |_, chunk| {
            capture.push(chunk)?;
            // Replace a bounded preview instead of accumulating unbounded UI
            // deltas. Raw bytes are decoded together, not at pipe boundaries.
            if capture.total != published && (published == 0 || updated.elapsed() >= Duration::from_millis(100)) {
                capture.flush()?;
                progress(capture.preview());
                published = capture.total;
                updated = Instant::now();
            }
            Ok(())
        },
    );
    let flushed = capture.flush();
    let mut output = capture.preview();
    progress(output.clone());
    flushed.map_err(|e| format!("Cannot flush output log: {e}"))?;
    let exit = exit?;
    let status = if exit.cancelled {
        ToolStatus::Cancelled
    } else if exit.timed_out {
        ToolStatus::TimedOut(timeout)
    } else {
        ToolStatus::Exited(exit.code)
    };
    let mut images = vec![];
    if let Ok(file) = fs::File::open(manifest) {
        use std::io::{BufRead, BufReader, Read};
        for line in BufReader::new(file.take(64 * 1024)).lines().take(8) {
            let result = (|| -> Result<(PathBuf, Value)> {
                let path: PathBuf = serde_json::from_str(&line?)?;
                let value = image(&path)?;
                Ok((path, value))
            })();
            match result {
                Ok((path, value)) => {
                    images.push(json!({"role":"user", "content":[{"type":"input_text", "text":format!("view_image: {}", path.display())}, value]}));
                }
                Err(e) => output.text.push_str(&format!("\n[view_image: {e}]")),
            }
        }
    }
    Ok(BashResult { output, status, images })
}
