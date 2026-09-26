use crate::skills::Skill;
use std::{fs, path::Path};

pub fn build(cwd: &Path, skills: &[Skill]) -> String {
    let mut instructions = "You are coding agent".to_string();
    let ancestors: Vec<_> = cwd.ancestors().collect();
    let project: Vec<_> = ancestors.iter().rev().filter_map(|p| fs::read_to_string(p.join("AGENTS.md")).ok()).collect();
    if !project.is_empty() {
        instructions.push_str("\n\nProject Instructions:\n");
        instructions.push_str(&project.join("\n\n"));
    }
    let entries: Vec<_> =
        skills.iter().map(|skill| format!("{}: {}", skill.path.display(), skill.description)).collect();
    if !entries.is_empty() {
        instructions.push_str("\n\nSkills:\n");
        instructions.push_str(&entries.join("\n"));
    }
    instructions.push_str(&format!("\n\ncwd:{}", cwd.display()));
    instructions
}
