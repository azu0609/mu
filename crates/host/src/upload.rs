use crate::{Result, devices::short_id, session};
use base64::{Engine, engine::general_purpose::STANDARD};
use std::{
    fs,
    io::Write,
    os::unix::fs::{DirBuilderExt, OpenOptionsExt},
    path::{Path, PathBuf},
    time::{Duration, SystemTime},
};

// Messages snapshot what they mention, so an upload only has to outlive the send.
const KEEP: Duration = Duration::from_secs(24 * 60 * 60);

pub fn save(name: &str, data: &str) -> Result<PathBuf> {
    let bytes = STANDARD.decode(data).map_err(|_| "An upload's data must be base64")?;
    let name: String =
        name.chars().map(|c| if c.is_ascii_alphanumeric() || "._-".contains(c) { c } else { '_' }).take(64).collect();
    let name = match name.trim_start_matches('.') {
        "" => "upload",
        name => name,
    };
    let dir = session::state_dir().join("uploads");
    fs::DirBuilder::new().recursive(true).mode(0o700).create(&dir)?;
    prune(&dir);
    let path = dir.join(format!("{}-{name}", short_id()?));
    let mut file = fs::OpenOptions::new().write(true).create_new(true).mode(0o600).open(&path)?;
    file.write_all(&bytes)?;
    file.sync_all()?;
    Ok(path)
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
