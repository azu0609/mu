use std::{env, path::PathBuf};

connectrpc::include_generated!();

pub use mu::v1::{__buffa::oneof::session_event::Event, *};

// Where a host listens for local clients.
pub fn socket_path() -> PathBuf {
    if let Some(path) = env::var_os("MU_SOCKET").filter(|path| !path.is_empty()) {
        return path.into();
    }
    match env::var_os("XDG_RUNTIME_DIR").filter(|dir| !dir.is_empty()) {
        Some(dir) => PathBuf::from(dir).join("mu.sock"),
        None => env::var_os("XDG_STATE_HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|| env::home_dir().unwrap_or_else(|| ".".into()).join(".local/state"))
            .join("mu/mu.sock"),
    }
}
