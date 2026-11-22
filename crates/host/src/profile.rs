use crate::{Result, devices::short_id, home, store};
use mu_wire::{self as wire, Pictogram, Snippet};
use serde::{Deserialize, Serialize};
use std::{path::PathBuf, sync::Mutex};

#[derive(Clone, Default, Serialize, Deserialize)]
struct Stored {
    pictogram: Option<Pictogram>,
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
        Ok(Self { stored: Mutex::new(store::load(&path)?), path, name })
    }

    pub fn get(&self) -> wire::Profile {
        let stored = self.stored.lock().unwrap().clone();
        wire::Profile {
            name: self.name.clone(),
            home: home(),
            pictogram: stored.pictogram,
            description: stored.description,
            snippets: stored.snippets,
        }
    }

    pub fn set_pictogram(&self, pictogram: Option<Pictogram>) -> Result<()> {
        let pictogram = valid(pictogram)?;
        self.update(|stored| {
            stored.pictogram = pictogram;
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
        let mut snippet =
            Snippet { id: String::new(), name, command: command.into(), pictogram: valid(call.pictogram)? };
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
        store::save(&self.path, &changed)?;
        *stored = changed;
        Ok(result)
    }
}

fn valid(pictogram: Option<Pictogram>) -> Result<Option<Pictogram>> {
    if let Some(Pictogram { name, .. }) = &pictogram
        && (name.is_empty() || name.len() > 64 || !name.chars().all(|c| c.is_ascii_alphanumeric() || "-_:".contains(c)))
    {
        return Err(format!("Not an icon name: {name}").into());
    }
    Ok(pictogram)
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
