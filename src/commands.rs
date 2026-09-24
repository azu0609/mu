use crate::{Result, skills::Skill};
use std::collections::BTreeMap;

#[derive(Clone, Copy)]
pub enum BuiltinKind {
    Model,
    New,
    Resume,
    Tree,
    Copy,
    Quit,
}

pub struct Builtin {
    pub kind: BuiltinKind,
    pub name: &'static str,
    pub hint: &'static str,
}

const BUILTINS: &[Builtin] = &[
    Builtin { kind: BuiltinKind::Model, name: "/model", hint: "<model> [effort|-] [context]" },
    Builtin { kind: BuiltinKind::New, name: "/new", hint: "new conversation" },
    Builtin { kind: BuiltinKind::Resume, name: "/resume", hint: "resume session" },
    Builtin { kind: BuiltinKind::Tree, name: "/tree", hint: "conversation tree" },
    Builtin { kind: BuiltinKind::Copy, name: "/copy", hint: "[agent|user]" },
    Builtin { kind: BuiltinKind::Quit, name: "/quit", hint: "quit" },
];

#[derive(Clone, Copy)]
pub enum Choice {
    Builtin(&'static Builtin),
    Skill(usize),
}

// Both completion and submission use these same choices. Built-ins win
// collisions; the last discovered skill wins among skills.
pub fn choices(skills: &[Skill]) -> BTreeMap<String, (Choice, String)> {
    let mut entries = BTreeMap::new();
    for (i, skill) in skills.iter().enumerate() {
        if !skill.name.is_empty() && skill.name.chars().all(|c| c.is_alphanumeric() || "-_.".contains(c)) {
            entries.insert(format!("/{}", skill.name), (Choice::Skill(i), skill.description.clone()));
        }
    }
    entries
        .extend(BUILTINS.iter().map(|command| (command.name.into(), (Choice::Builtin(command), command.hint.into()))));
    entries
}

pub fn resolve(word: &str, skills: &[Skill]) -> Result<(String, Choice)> {
    let choices = choices(skills);
    if let Some(&(choice, _)) = choices.get(word) {
        return Ok((word.into(), choice));
    }
    let names: Vec<_> = choices.keys().filter(|name| name.starts_with(word)).collect();
    match names.as_slice() {
        [name] => Ok(((*name).clone(), choices[*name].0)),
        [] => Err(format!("Unknown command or skill: {word}").into()),
        _ => {
            Err(format!("Choose a command: {}", names.iter().map(|name| name.as_str()).collect::<Vec<_>>().join(", "))
                .into())
        }
    }
}
