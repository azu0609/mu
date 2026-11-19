use crate::{Result, config, home};
use http_body_util::BodyExt;
use hyper::{Request, body::Incoming};
use percent_encoding::percent_decode_str;
use std::{
    fs,
    io::ErrorKind,
    os::unix::fs::{DirBuilderExt, OpenOptionsExt},
    path::{Path, PathBuf},
    time::{Duration, SystemTime},
};
use tokio::io::AsyncWriteExt;

// Messages snapshot what they mention, so an upload only has to outlive the send.
const KEEP: Duration = Duration::from_secs(24 * 60 * 60);

pub async fn receive(request: Request<Incoming>) -> Result<PathBuf> {
    let name = request
        .uri()
        .query()
        .and_then(|query| query.split('&').find_map(|pair| pair.strip_prefix("name=")))
        .map(|name| percent_decode_str(name).decode_utf8_lossy().into_owned())
        .unwrap_or_default();
    let (path, file) = tokio::task::spawn_blocking(move || create(&name)).await??;
    let mut file = tokio::fs::File::from_std(file);
    let mut body = request.into_body();
    let written: Result<()> = async {
        while let Some(frame) = body.frame().await {
            if let Ok(data) = frame?.into_data() {
                file.write_all(&data).await?;
            }
        }
        Ok(file.sync_all().await?)
    }
    .await;
    if let Err(e) = written {
        let _ = tokio::fs::remove_file(&path).await;
        return Err(e);
    }
    Ok(match path.strip_prefix(home()) {
        Ok(rest) => Path::new("~").join(rest),
        Err(_) => path,
    })
}

fn create(name: &str) -> Result<(PathBuf, fs::File)> {
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
        match fs::OpenOptions::new().write(true).create_new(true).mode(0o600).open(&path) {
            Err(e) if e.kind() == ErrorKind::AlreadyExists => continue,
            file => return Ok((path, file?)),
        }
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
