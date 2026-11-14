use crate::{Result, devices::short_id, home};
use mu_wire::{self as wire, Snippet};
use serde::{Deserialize, Serialize};
use std::{
    fs,
    io::{self, Write},
    os::unix::fs::{DirBuilderExt, OpenOptionsExt},
    path::PathBuf,
    sync::Mutex,
};

#[derive(Clone, Default, Serialize, Deserialize)]
struct Stored {
    icon: Option<String>,
    description: String,
    snippets: Vec<Snippet>,
}

pub struct Profile {
    path: PathBuf,
    stored: Mutex<Stored>,
    name: String,
}

impl Profile {
    pub fn load(path: PathBuf, name: String) -> Result<Self> {
        let stored = match fs::read(&path) {
            Ok(bytes) => serde_json::from_slice(&bytes).map_err(|e| format!("{}: {e}", path.display()))?,
            Err(e) if e.kind() == io::ErrorKind::NotFound => Stored::default(),
            Err(e) => return Err(format!("{}: {e}", path.display()).into()),
        };
        Ok(Self { path, stored: Mutex::new(stored), name })
    }

    pub fn get(&self) -> wire::Profile {
        let stored = self.stored.lock().unwrap().clone();
        wire::Profile {
            name: self.name.clone(),
            home: home(),
            icon: stored.icon,
            description: stored.description,
            snippets: stored.snippets,
        }
    }

    pub fn set_icon(&self, icon: Option<String>) -> Result<()> {
        let icon = icon_name(icon)?;
        self.update(|stored| {
            stored.icon = icon;
            Ok(())
        })
    }

    pub fn set_description(&self, description: String) -> Result<()> {
        let description = line("A description", &description, 200)?;
        self.update(|stored| {
            stored.description = description;
            Ok(())
        })
    }

    pub fn save_snippet(&self, call: wire::SaveSnippet) -> Result<String> {
        let name = line("A snippet's name", &call.name, 64)?;
        let command = call.command.trim();
        if name.is_empty() || command.is_empty() {
            return Err("A snippet needs a name and a command".into());
        }
        if command.len() > 16 * 1024 || command.contains('\0') {
            return Err("A snippet's command must be text under 16 KiB".into());
        }
        let mut snippet = Snippet { id: String::new(), name, command: command.into(), icon: icon_name(call.icon)? };
        self.update(|stored| {
            match call.id {
                Some(id) => {
                    let slot = stored.snippets.iter_mut().find(|known| known.id == id).ok_or("No such snippet")?;
                    snippet.id = id;
                    *slot = snippet.clone();
                }
                None => {
                    snippet.id = short_id()?;
                    while stored.snippets.iter().any(|known| known.id == snippet.id) {
                        snippet.id = short_id()?;
                    }
                    stored.snippets.push(snippet.clone());
                }
            }
            Ok(snippet.id)
        })
    }

    pub fn delete_snippet(&self, id: &str) -> Result<()> {
        self.update(|stored| {
            let before = stored.snippets.len();
            stored.snippets.retain(|snippet| snippet.id != id);
            match stored.snippets.len() == before {
                true => Err("No such snippet".into()),
                false => Ok(()),
            }
        })
    }

    fn update<T>(&self, change: impl FnOnce(&mut Stored) -> Result<T>) -> Result<T> {
        let mut stored = self.stored.lock().unwrap();
        let mut changed = stored.clone();
        let result = change(&mut changed)?;
        self.save(&changed)?;
        *stored = changed;
        Ok(result)
    }

    fn save(&self, stored: &Stored) -> Result<()> {
        if let Some(dir) = self.path.parent() {
            fs::DirBuilder::new().recursive(true).mode(0o700).create(dir)?;
        }
        let temporary = self.path.with_extension("json.tmp");
        let mut file = fs::OpenOptions::new().write(true).create(true).truncate(true).mode(0o600).open(&temporary)?;
        file.write_all(&serde_json::to_vec_pretty(stored)?)?;
        file.sync_all()?;
        fs::rename(&temporary, &self.path)?;
        Ok(())
    }
}

fn icon_name(icon: Option<String>) -> Result<Option<String>> {
    let Some(icon) = icon.filter(|icon| !icon.is_empty()) else {
        return Ok(None);
    };
    if icon.len() > 64 || !icon.chars().all(|c| c.is_ascii_alphanumeric() || "-_:".contains(c)) {
        return Err(format!("Not an icon name: {icon}").into());
    }
    Ok(Some(icon))
}

fn line(what: &str, text: &str, limit: usize) -> Result<String> {
    let text = text.trim();
    if text.chars().any(char::is_control) {
        return Err(format!("{what} is a single line").into());
    }
    if text.chars().count() > limit {
        return Err(format!("{what} takes up to {limit} characters").into());
    }
    Ok(text.into())
}
