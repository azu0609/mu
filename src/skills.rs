use crate::home;
use std::{
    collections::HashSet,
    fs,
    path::{Path, PathBuf},
};

fn frontmatter(text: &str, field: &str) -> Option<String> {
    let mut lines = text.lines();
    if lines.next()?.trim() != "---" {
        return None;
    }
    let front: Vec<_> = lines.take_while(|l| l.trim() != "---").collect();
    for (i, line) in front.iter().enumerate() {
        if let Some(value) = line.strip_prefix(&format!("{field}:")) {
            let value = value.trim();
            if matches!(value, ">" | "|" | ">-" | "|-") {
                return Some(
                    front[i + 1..]
                        .iter()
                        .take_while(|l| l.starts_with(' '))
                        .map(|l| l.trim())
                        .collect::<Vec<_>>()
                        .join(" "),
                );
            }
            return Some(value.trim_matches(['\'', '"']).to_string());
        }
    }
    None
}

fn discover_skills(dir: &Path, found: &mut Vec<PathBuf>, depth: usize) {
    if depth > 4 {
        return;
    }
    if dir.join("SKILL.md").is_file() {
        found.push(dir.join("SKILL.md"));
        return;
    }
    if let Ok(entries) = fs::read_dir(dir) {
        let mut entries: Vec<_> = entries.filter_map(|e| e.ok()).map(|e| e.path()).filter(|p| p.is_dir()).collect();
        entries.sort();
        for p in entries {
            discover_skills(&p, found, depth + 1);
        }
    }
}

#[derive(Clone)]
pub struct Skill {
    pub name: String,
    pub path: PathBuf,
    pub description: String,
}

pub fn skills(cwd: &Path) -> Vec<Skill> {
    skills_in(&home(), cwd)
}

fn skills_in(home: &Path, cwd: &Path) -> Vec<Skill> {
    let mut paths = vec![];
    discover_skills(&home.join(".agents/skills"), &mut paths, 0);
    for dir in cwd.ancestors().collect::<Vec<_>>().iter().rev() {
        discover_skills(&dir.join(".agents/skills"), &mut paths, 0);
    }
    let mut seen = HashSet::new();
    paths
        .into_iter()
        .filter_map(|path| {
            // Deduplicate by target, but keep the discovered path: instructions should show
            // the symlink the user set up, not the (often store-hashed) path it points to.
            if !seen.insert(path.canonicalize().ok()?) {
                return None;
            }
            let text = fs::read_to_string(&path).ok()?;
            let name = frontmatter(&text, "name").unwrap_or_else(|| {
                path.parent().unwrap().file_name().unwrap_or_default().to_string_lossy().into_owned()
            });
            Some(Skill { name, description: frontmatter(&text, "description")?, path })
        })
        .collect()
}
