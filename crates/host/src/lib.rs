mod agent;
mod api;
mod config;
mod devices;
mod directory;
mod host;
mod image;
mod input;
mod instructions;
mod items;
mod listen;
mod net;
mod process;
mod profile;
mod server;
mod session;
mod skills;
mod store;
mod stream;
mod summary;
pub mod tools;
mod upload;

pub use listen::{listen_address, serve, serve_stream, socket_path};
use std::{
    env, fs,
    io::Read,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

pub type Result<T> = std::result::Result<T, Box<dyn std::error::Error + Send + Sync>>;

pub fn home() -> PathBuf {
    env::home_dir().unwrap_or_else(|| PathBuf::from("."))
}

// Milliseconds since the Unix epoch.
fn now() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_millis() as u64
}

fn unique_id() -> String {
    format!("{}-{}", SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_nanos(), std::process::id())
}

fn read_limited(path: &Path, limit: u64) -> Result<Vec<u8>> {
    if !fs::metadata(path)?.is_file() {
        return Err(format!("{} is not a regular file", path.display()).into());
    }
    let mut bytes = vec![];
    fs::File::open(path)?.take(limit + 1).read_to_end(&mut bytes)?;
    if bytes.len() as u64 > limit {
        return Err(format!("{} exceeds {} MiB", path.display(), limit / (1024 * 1024)).into());
    }
    Ok(bytes)
}
