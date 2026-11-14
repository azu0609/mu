// Devices allowed to reach this host over the network. Each pairs through a
// one-time invite and gets a token of its own, so it can be unpaired alone.
// Only hashes of the tokens are kept.
use crate::Result;
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use mu_wire as wire;
use ring::{
    digest,
    rand::{SecureRandom, SystemRandom},
};
use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    fs,
    io::{self, Write},
    os::unix::fs::{DirBuilderExt, OpenOptionsExt},
    path::PathBuf,
    sync::Mutex,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

// How long an invite's code can be used.
const INVITE_TTL: Duration = Duration::from_secs(10 * 60);

fn random_bytes(len: usize) -> Result<Vec<u8>> {
    let mut bytes = vec![0; len];
    SystemRandom::new().fill(&mut bytes).map_err(|_| "The system's random source failed")?;
    Ok(bytes)
}

// Unpadded base64url, safe in links and headers.
fn random(len: usize) -> Result<String> {
    Ok(URL_SAFE_NO_PAD.encode(random_bytes(len)?))
}

// Eight hex digits, for naming things a person may type.
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
    // Unused invite codes and when they expire; they only live as long as the host.
    invites: Mutex<HashMap<String, Instant>>,
}

impl Devices {
    pub fn load(path: PathBuf) -> Result<Self> {
        let list = match fs::read(&path) {
            Ok(bytes) => serde_json::from_slice(&bytes).map_err(|e| format!("{}: {e}", path.display()))?,
            Err(e) if e.kind() == io::ErrorKind::NotFound => vec![],
            Err(e) => return Err(format!("{}: {e}", path.display()).into()),
        };
        Ok(Self { path, list: Mutex::new(list), invites: Mutex::default() })
    }

    // A code that pairs one device, once, within a few minutes.
    pub fn invite(&self) -> Result<String> {
        let code = random(16)?;
        let now = Instant::now();
        let mut invites = self.invites.lock().unwrap();
        invites.retain(|_, expiry| *expiry > now);
        invites.insert(code.clone(), now + INVITE_TTL);
        Ok(code)
    }

    // Uses up the code and returns the new device's token, or none if the code
    // is unknown, used or expired.
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
        if let Err(e) = self.save(&list) {
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

    // Whether there was such a device. Its open connections stay up, but every
    // further call from it is refused.
    pub fn unpair(&self, id: &str) -> Result<bool> {
        let mut list = self.list.lock().unwrap();
        let Some(i) = list.iter().position(|device| device.id == id) else {
            return Ok(false);
        };
        let device = list.remove(i);
        if let Err(e) = self.save(&list) {
            list.insert(i, device);
            return Err(e);
        }
        Ok(true)
    }

    fn save(&self, list: &[Device]) -> Result<()> {
        if let Some(dir) = self.path.parent() {
            fs::DirBuilder::new().recursive(true).mode(0o700).create(dir)?;
        }
        let temporary = self.path.with_extension("json.tmp");
        let mut file = fs::OpenOptions::new().write(true).create(true).truncate(true).mode(0o600).open(&temporary)?;
        file.write_all(&serde_json::to_vec_pretty(list)?)?;
        file.sync_all()?;
        fs::rename(&temporary, &self.path)?;
        Ok(())
    }
}
