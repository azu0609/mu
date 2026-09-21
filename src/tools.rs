use crate::{Result, process, session::unique_id};
use base64::{Engine, engine::general_purpose::STANDARD};
use serde::Deserialize;
use serde_json::{Value, json};
use std::{
    env, fs,
    io::Write,
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
    process::Command,
    sync::{Arc, atomic::AtomicBool},
    time::Duration,
};

const OUTPUT_LIMIT: usize = 64 * 1024;
const IMAGE_LIMIT: u64 = 20 * 1024 * 1024;

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
    mut delta: impl FnMut(&str),
) -> Result<(String, Vec<Value>)> {
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
    let mut bytes = vec![];
    let mut truncated = false;
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
            let n = chunk.len().min(OUTPUT_LIMIT - bytes.len());
            bytes.extend_from_slice(&chunk[..n]);
            if n > 0 {
                delta(&String::from_utf8_lossy(&chunk[..n]));
            }
            truncated |= n < chunk.len();
            Ok(())
        },
    )?;
    let mut output = String::from_utf8_lossy(&bytes).into_owned();
    if truncated {
        output.push_str("\n[output truncated at 64 KiB]");
    }
    if exit.timed_out {
        output.push_str(&format!("\n[timed out after {timeout} ms; process group killed]"));
    }
    if exit.cancelled {
        output.push_str("\n[cancelled; process group killed]");
    }
    output.push_str(&format!("\n[exit {}]", exit.code));
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
                Err(e) => output.push_str(&format!("\n[view_image: {e}]")),
            }
        }
    }
    Ok((output, images))
}
