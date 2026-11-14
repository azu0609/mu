use crate::{Remote, Result};
use serde::{Deserialize, Serialize};
use std::{
    fs,
    io::{self, Write},
    os::unix::fs::OpenOptionsExt,
    path::PathBuf,
};

#[derive(Clone, PartialEq, Serialize, Deserialize)]
pub struct Paired {
    pub name: String,
    pub icon: Option<String>,
    pub description: String,
    #[serde(flatten)]
    pub remote: Remote,
}

pub struct Hosts {
    path: PathBuf,
    list: Vec<Paired>,
}

impl Hosts {
    pub fn load(path: PathBuf) -> Result<Self> {
        let list = match fs::read(&path) {
            Ok(bytes) => serde_json::from_slice(&bytes).map_err(|e| format!("{}: {e}", path.display()))?,
            Err(e) if e.kind() == io::ErrorKind::NotFound => vec![],
            Err(e) => return Err(format!("{}: {e}", path.display()).into()),
        };
        Ok(Self { path, list })
    }

    pub fn list(&self) -> &[Paired] {
        &self.list
    }

    pub fn get(&self, cert: &str) -> Option<&Paired> {
        self.list.iter().find(|host| host.remote.cert == cert)
    }

    pub fn add(&mut self, host: Paired) -> Result<()> {
        let mut list = self.list.clone();
        match list.iter_mut().find(|known| known.remote.cert == host.remote.cert) {
            Some(known) => *known = host,
            None => list.push(host),
        }
        self.save(list)
    }

    pub fn update(&mut self, cert: &str, change: impl FnOnce(&mut Paired)) -> Result<()> {
        let mut list = self.list.clone();
        let Some(host) = list.iter_mut().find(|host| host.remote.cert == cert) else {
            return Ok(());
        };
        change(host);
        if list == self.list {
            return Ok(());
        }
        self.save(list)
    }

    pub fn remove(&mut self, cert: &str) -> Result<bool> {
        let mut list = self.list.clone();
        list.retain(|host| host.remote.cert != cert);
        let removed = list.len() != self.list.len();
        self.save(list)?;
        Ok(removed)
    }

    fn save(&mut self, list: Vec<Paired>) -> Result<()> {
        if let Some(dir) = self.path.parent() {
            fs::create_dir_all(dir)?;
        }
        let temporary = self.path.with_extension("json.tmp");
        let mut file = fs::OpenOptions::new().write(true).create(true).truncate(true).mode(0o600).open(&temporary)?;
        file.write_all(&serde_json::to_vec_pretty(&list)?)?;
        file.sync_all()?;
        fs::rename(&temporary, &self.path)?;
        self.list = list;
        Ok(())
    }
}
