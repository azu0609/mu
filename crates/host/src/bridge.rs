// How the special commands in a bash run reach their host. Each is the binary
// that started the host, linked under the command's name; it calls back over
// the host's socket with the token of the run it is in.
use crate::{
    Result,
    host::{Failure, Rpc, absolute, failed},
    image, unique_id,
};
use hyper::StatusCode;
use serde_json::{Value, json};
use std::{
    collections::HashMap,
    env,
    ffi::OsString,
    fs,
    os::unix::fs::{DirBuilderExt, symlink},
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};

const IMAGES: usize = 8;

pub struct Bridge {
    socket: PathBuf,
    bin: PathBuf,
    commands: Vec<&'static str>,
    runs: Mutex<HashMap<String, Open>>,
}

struct Open {
    session: Option<String>,
    images: Vec<Value>,
}

impl Bridge {
    pub fn new(socket: &Path, dir: &Path, commands: Vec<&'static str>) -> Result<Arc<Self>> {
        let bin = dir.join("bin");
        fs::DirBuilder::new().recursive(true).mode(0o700).create(&bin)?;
        let exe = env::current_exe()?;
        for name in &commands {
            symlink(&exe, bin.join(name))?;
        }
        Ok(Arc::new(Self { socket: socket.into(), bin, commands, runs: Mutex::default() }))
    }

    pub fn commands(&self) -> &[&'static str] {
        &self.commands
    }

    // `session` is the one whose tool runs it; none for the user's own commands.
    pub fn open(self: &Arc<Self>, session: Option<String>) -> Run {
        let token = unique_id();
        self.runs.lock().unwrap().insert(token.clone(), Open { session, images: vec![] });
        Run { bridge: self.clone(), token }
    }

    fn with<T>(&self, run: &str, f: impl FnOnce(&mut Open) -> Rpc<T>) -> Rpc<T> {
        let mut runs = self.runs.lock().unwrap();
        let open = runs.get_mut(run).ok_or_else(|| Failure(StatusCode::NOT_FOUND, "That run has ended".into()))?;
        f(open)
    }

    pub fn session(&self, run: &str) -> Rpc<Option<String>> {
        self.with(run, |open| Ok(open.session.clone()))
    }

    pub fn view_image(&self, run: &str, path: &Path) -> Rpc<()> {
        absolute(path)?;
        let image = image::read(path).map_err(failed)?;
        self.with(run, |open| {
            if open.images.len() == IMAGES {
                return Err(failed(format!("One command shows at most {IMAGES} images")));
            }
            let label = format!("view_image: {}", path.display());
            open.images.push(json!({"role":"user", "content":[{"type":"input_text", "text":label}, image]}));
            Ok(())
        })
    }
}

pub struct Run {
    bridge: Arc<Bridge>,
    token: String,
}

impl Run {
    // MU_BIN is where the commands are linked, for the shell to put first on PATH.
    pub fn env(&self) -> [(&'static str, OsString); 3] {
        [
            ("MU_BIN", self.bridge.bin.clone().into()),
            ("MU_HOST", self.bridge.socket.clone().into()),
            ("MU_RUN", self.token.clone().into()),
        ]
    }

    pub fn images(&self) -> Vec<Value> {
        self.bridge.with(&self.token, |open| Ok(std::mem::take(&mut open.images))).unwrap_or_default()
    }
}

impl Drop for Run {
    fn drop(&mut self) {
        self.bridge.runs.lock().unwrap().remove(&self.token);
    }
}
