use crate::{Result, store};
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use mu_wire as wire;
use ring::{
    digest,
    rand::{SecureRandom, SystemRandom},
};
use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    path::PathBuf,
    sync::Mutex,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

const INVITE_TTL: Duration = Duration::from_secs(10 * 60);

fn random_bytes(len: usize) -> Result<Vec<u8>> {
    let mut bytes = vec![0; len];
    SystemRandom::new().fill(&mut bytes).map_err(|_| "The system's random source failed")?;
    Ok(bytes)
}

fn random(len: usize) -> Result<String> {
    Ok(URL_SAFE_NO_PAD.encode(random_bytes(len)?))
}

pub fn short_id() -> Result<String> {
    Ok(random_bytes(4)?.iter().map(|b| format!("{b:02x}")).collect())
}

pub fn sha256(data: &[u8]) -> String {
    URL_SAFE_NO_PAD.encode(digest::digest(&digest::SHA256, data))
}

#[derive(Serialize, Deserialize)]
struct Device {
    id: String,
    name: String,
    paired: u64,
    token_sha256: String,
}

pub struct Devices {
    path: PathBuf,
    list: Mutex<Vec<Device>>,
    invites: Mutex<HashMap<String, Instant>>,
}

impl Devices {
    pub fn load(path: PathBuf) -> Result<Self> {
        Ok(Self { list: Mutex::new(store::load(&path)?), path, invites: Mutex::default() })
    }

    pub fn invite(&self) -> Result<String> {
        let code = random(16)?;
        let now = Instant::now();
        let mut invites = self.invites.lock().unwrap();
        invites.retain(|_, expiry| *expiry > now);
        invites.insert(code.clone(), now + INVITE_TTL);
        Ok(code)
    }

    pub fn pair(&self, code: &str, name: &str) -> Result<Option<String>> {
        if !self.invites.lock().unwrap().remove(code).is_some_and(|expiry| expiry > Instant::now()) {
            return Ok(None);
        }
        let token = random(32)?;
        let name: String = name.chars().filter(|c| !c.is_control()).take(64).collect();
        let device = Device {
            id: short_id()?,
            name: if name.trim().is_empty() { "Unnamed device".into() } else { name },
            paired: SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_secs(),
            token_sha256: sha256(token.as_bytes()),
        };
        let mut list = self.list.lock().unwrap();
        list.push(device);
        if let Err(e) = store::save(&self.path, &*list) {
            list.pop();
            return Err(e);
        }
        Ok(Some(token))
    }

    pub fn authorized(&self, token: &str) -> bool {
        let hash = sha256(token.as_bytes());
        self.list.lock().unwrap().iter().any(|device| device.token_sha256 == hash)
    }

    pub fn list(&self) -> Vec<wire::Device> {
        let list = self.list.lock().unwrap();
        list.iter().map(|d| wire::Device { id: d.id.clone(), name: d.name.clone(), paired: d.paired }).collect()
    }

    pub fn unpair(&self, id: &str) -> Result<bool> {
        let mut list = self.list.lock().unwrap();
        let Some(i) = list.iter().position(|device| device.id == id) else {
            return Ok(false);
        };
        let device = list.remove(i);
        if let Err(e) = store::save(&self.path, &*list) {
            list.insert(i, device);
            return Err(e);
        }
        Ok(true)
    }
}
