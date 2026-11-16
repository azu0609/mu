use crate::{Result, config, home, image, input};
use base64::{Engine, engine::general_purpose::STANDARD};
use std::{
    fs,
    io::{ErrorKind, Write},
    os::unix::fs::{DirBuilderExt, OpenOptionsExt},
    path::{Path, PathBuf},
    time::{Duration, SystemTime},
};

// Messages snapshot what they mention, so an upload only has to outlive the send.
const KEEP: Duration = Duration::from_secs(24 * 60 * 60);

pub fn save(name: &str, data: &str) -> Result<PathBuf> {
    let bytes = STANDARD.decode(data).map_err(|_| "An upload's data must be base64")?;
    if image::mime(&bytes).is_some() {
        if bytes.len() as u64 > image::LIMIT {
            return Err(format!("{name}: Image exceeds 20 MiB").into());
        }
    } else {
        input::text_content(bytes.clone()).map_err(|e| format!("{name}: {e}"))?;
    }
    let name: String =
        name.chars().map(|c| if c.is_ascii_alphanumeric() || "._-".contains(c) { c } else { '_' }).take(64).collect();
    let name = match name.trim_start_matches('.') {
        "" => "upload",
        name => name,
    };
    let (stem, extension) = match name.rsplit_once('.') {
        Some((stem, extension)) if !stem.is_empty() => (stem, format!(".{extension}")),
        _ => (name, String::new()),
    };
    let dir = config::cache_dir();
    fs::DirBuilder::new().recursive(true).mode(0o700).create(&dir)?;
    prune(&dir);
    let mut n = 0;
    loop {
        n += 1;
        let path = dir.join(if n == 1 { name.to_string() } else { format!("{stem}-{n}{extension}") });
        let mut file = match fs::OpenOptions::new().write(true).create_new(true).mode(0o600).open(&path) {
            Err(e) if e.kind() == ErrorKind::AlreadyExists => continue,
            file => file?,
        };
        file.write_all(&bytes)?;
        file.sync_all()?;
        return Ok(match path.strip_prefix(home()) {
            Ok(rest) => Path::new("~").join(rest),
            Err(_) => path,
        });
    }
}

fn prune(dir: &Path) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    let now = SystemTime::now();
    for entry in entries.filter_map(|entry| entry.ok()) {
        let stale = entry
            .metadata()
            .and_then(|metadata| metadata.modified())
            .is_ok_and(|modified| now.duration_since(modified).is_ok_and(|age| age > KEEP));
        if stale {
            let _ = fs::remove_file(entry.path());
        }
    }
}
