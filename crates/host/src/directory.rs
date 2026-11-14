// Directories on the host, for choosing where a session works.
use crate::{Result, home};
use mu_wire::{Directory, DirectoryEntry, EntryKind};
use std::{fs, path::Path};

pub fn list(path: &Path) -> Result<Directory> {
    let path = match path.strip_prefix("~") {
        Ok(rest) => home().join(rest),
        Err(_) if path.as_os_str().is_empty() => home(),
        Err(_) => path.to_owned(),
    };
    if !path.is_absolute() {
        return Err(format!("{} is not an absolute path", path.display()).into());
    }
    let path = path.canonicalize().map_err(|e| format!("{}: {e}", path.display()))?;
    let mut entries: Vec<_> = fs::read_dir(&path)
        .map_err(|e| format!("{}: {e}", path.display()))?
        .filter_map(|entry| entry.ok())
        .take(4096)
        .filter_map(|entry| {
            // A name that is not UTF-8 could not be asked for again.
            let name = entry.file_name().into_string().ok().filter(|name| !name.chars().any(char::is_control))?;
            // Following links, as a session started there would.
            let kind = match entry.path() {
                path if !path.is_dir() => EntryKind::File,
                path if path.join(".git").exists() => EntryKind::Repository,
                _ => EntryKind::Directory,
            };
            Some(DirectoryEntry { name, kind })
        })
        .collect();
    entries.sort_by_cached_key(|entry| (entry.kind == EntryKind::File, entry.name.to_lowercase()));
    Ok(Directory { path, entries })
}
