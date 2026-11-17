use super::Run;
use mu_client::{CreateSession, ListSessions, Model, RunSession, SendMessage, SessionSummary, Status, Stop};
use mu_host::Result;
use std::{env, path::PathBuf};

const USAGE: &str = "agent spawn MESSAGE [--cwd DIR] [--model NAME] [--effort LEVEL]
agent send ID|parent MESSAGE
agent status [ID|parent]
agent stop ID [--children]

A message wakes the session it is sent to and says who sent it; answer with agent send.
To wait for an answer, end your turn.";

struct Args {
    words: Vec<String>,
    values: Vec<(&'static str, String)>,
    switches: Vec<&'static str>,
}

impl Args {
    fn parse(args: &[String], values: &[&'static str], switches: &[&'static str]) -> Result<Self> {
        let mut parsed = Self { words: vec![], values: vec![], switches: vec![] };
        let mut args = args.iter();
        while let Some(arg) = args.next() {
            if arg == "--" {
                parsed.words.extend(args.by_ref().cloned());
            } else if let Some(&flag) = values.iter().find(|&&flag| flag == arg) {
                parsed.values.push((flag, args.next().ok_or_else(|| format!("{flag} needs a value"))?.clone()));
            } else if let Some(&flag) = switches.iter().find(|&&flag| flag == arg) {
                parsed.switches.push(flag);
            } else if arg.starts_with("--") {
                return Err(format!("Unknown option {arg}\n\n{USAGE}").into());
            } else {
                parsed.words.push(arg.clone());
            }
        }
        Ok(parsed)
    }

    fn value(&self, flag: &str) -> Option<&str> {
        self.values.iter().rev().find(|(name, _)| *name == flag).map(|(_, value)| value.as_str())
    }

    fn words<const N: usize>(&self) -> Result<[&str; N]> {
        let words: Vec<_> = self.words.iter().map(String::as_str).collect();
        words.try_into().map_err(|_| USAGE.into())
    }
}

struct Session<'a> {
    run: &'a Run,
    me: String,
}

impl Session<'_> {
    fn sessions(&self) -> Result<Vec<SessionSummary>> {
        self.run.call(ListSessions {})
    }

    fn resolve(&self, target: &str) -> Result<String> {
        if target != "parent" {
            return Ok(target.into());
        }
        self.sessions()?
            .into_iter()
            .find(|summary| summary.id == self.me)
            .and_then(|summary| summary.parent)
            .ok_or_else(|| "This session has no parent".into())
    }
}

pub fn run(run: &Run, args: &[String]) -> Result<()> {
    let Some((command, args)) = args.split_first() else {
        return Err(USAGE.into());
    };
    let Some(me) = run.call(RunSession { run: run.token.clone() })? else {
        return Err("agent only works in a session's bash, not in your own commands".into());
    };
    let session = Session { run, me };
    match command.as_str() {
        "spawn" => spawn(&session, &Args::parse(args, &["--cwd", "--model", "--effort"], &[])?),
        "send" => send(&session, &Args::parse(args, &[], &[])?),
        "status" => status(&session, &Args::parse(args, &[], &[])?),
        "stop" => stop(&session, &Args::parse(args, &[], &["--children"])?),
        _ => Err(USAGE.into()),
    }
}

fn spawn(session: &Session, args: &Args) -> Result<()> {
    let [text] = args.words()?;
    let cwd = match args.value("--cwd") {
        Some(cwd) => PathBuf::from(cwd),
        None => env::current_dir()?,
    };
    let cwd = cwd.canonicalize().map_err(|e| format!("{}: {e}", cwd.display()))?;
    let model = Model {
        name: args.value("--model").unwrap_or_default().into(),
        effort: args.value("--effort").map(Into::into),
        context: None,
    };
    let id = session.run.call(CreateSession {
        cwd,
        model,
        text: text.into(),
        skill: None,
        parent: Some(session.me.clone()),
    })?;
    println!("{id}");
    Ok(())
}

fn send(session: &Session, args: &Args) -> Result<()> {
    let [target, text] = args.words()?;
    session.run.call(SendMessage {
        session_id: session.resolve(target)?,
        text: text.into(),
        skill: None,
        from: Some(session.me.clone()),
    })?;
    Ok(())
}

fn status(session: &Session, args: &Args) -> Result<()> {
    let target = match &args.words[..] {
        [] => None,
        [target] => Some(session.resolve(target)?),
        _ => return Err(USAGE.into()),
    };
    let sessions = session.sessions()?;
    let shown: Vec<_> = sessions
        .iter()
        .filter(|summary| match &target {
            Some(id) => summary.id == *id,
            None => summary.parent.as_ref() == Some(&session.me),
        })
        .collect();
    if shown.is_empty() {
        return Err(match target {
            Some(id) => format!("No session {id}"),
            None => "This session has no children".into(),
        }
        .into());
    }
    for summary in shown {
        let status = match summary.status {
            Status::Idle => "idle",
            Status::Running => "running",
            Status::Finished => "finished",
            Status::Failed => "failed",
        };
        println!("{}  {status}  {}", summary.id, summary.title);
        if let Some(error) = &summary.error {
            println!("  {error}");
        }
    }
    Ok(())
}

fn stop(session: &Session, args: &Args) -> Result<()> {
    let [target] = args.words()?;
    let id = session.resolve(target)?;
    if id == session.me {
        return Err("A session cannot stop itself".into());
    }
    session.run.call(Stop { session_id: id, children: args.switches.contains(&"--children") })?;
    Ok(())
}
