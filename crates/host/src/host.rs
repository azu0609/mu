use crate::{
    Result,
    agent::{self, Agent},
    api,
    bridge::Bridge,
    config,
    devices::Devices,
    input, net,
    profile::Profile,
    session,
    summary::{self, Summary},
    tools,
};
use hyper::StatusCode;
use mu_wire::{self as wire, Status, WatchEvent};
use std::{
    collections::{HashMap, HashSet, VecDeque},
    net::SocketAddr,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};
use tokio::sync::broadcast;

pub struct Failure(pub StatusCode, pub String);

pub type Rpc<T> = std::result::Result<T, Failure>;

pub fn failed(error: impl ToString) -> Failure {
    Failure(StatusCode::BAD_REQUEST, error.to_string())
}

pub fn internal(error: impl ToString) -> Failure {
    Failure(StatusCode::INTERNAL_SERVER_ERROR, error.to_string())
}

pub fn absolute(path: &Path) -> Rpc<()> {
    match path.is_absolute() {
        true => Ok(()),
        false => Err(failed(format!("{} is not an absolute path", path.display()))),
    }
}

pub fn directory(path: &Path) -> Rpc<()> {
    match path.is_absolute() && path.is_dir() {
        true => Ok(()),
        false => Err(failed(format!("{} is not a directory", path.display()))),
    }
}

pub struct Network {
    pub bound: SocketAddr,
    pub cert: String,
}

#[derive(Clone)]
pub enum Change {
    Session(String),
    Sessions,
    Profile,
}

const RUNS: usize = 16;

// Lock order: the agents, then an agent, then the runs.
pub struct Host {
    pub agents: Mutex<HashMap<String, Arc<Agent>>>,
    pub changes: broadcast::Sender<Change>,
    pub name: String,
    pub profile: Profile,
    // The last commands run for the user, for a session to take in.
    runs: Mutex<VecDeque<(String, tools::Run)>>,
    pub devices: Option<Devices>,
    network: Option<Network>,
    pub bridge: Arc<Bridge>,
    models: Mutex<Option<Vec<wire::ModelInfo>>>,
    // The files under the last directory an @mention looked in.
    mentions: Mutex<Option<(PathBuf, Arc<Vec<String>>)>>,
}

impl Host {
    pub fn new(devices: Option<Devices>, network: Option<Network>, bridge: Arc<Bridge>) -> Result<Arc<Self>> {
        let name = net::hostname();
        let profile = Profile::load(config::dir().join("host.json"), name.clone())?;
        Ok(Arc::new(Self {
            agents: Mutex::default(),
            changes: broadcast::channel(256).0,
            name,
            profile,
            runs: Mutex::default(),
            devices,
            network,
            bridge,
            models: Mutex::default(),
            mentions: Mutex::default(),
        }))
    }

    // Runs `f` off the async threads: most of what a host does blocks.
    pub async fn run<T: Send + 'static>(
        self: &Arc<Self>,
        f: impl FnOnce(&Arc<Self>) -> Rpc<T> + Send + 'static,
    ) -> Rpc<T> {
        let host = self.clone();
        tokio::task::spawn_blocking(move || f(&host)).await.map_err(internal)?
    }

    fn changed(&self) -> agent::Changed {
        let changes = self.changes.clone();
        Arc::new(move |id| {
            let _ = changes.send(Change::Session(id.to_owned()));
        })
    }

    pub fn edit<T>(&self, edit: impl FnOnce(&Profile) -> Result<T>) -> Rpc<T> {
        let result = edit(&self.profile).map_err(failed)?;
        let _ = self.changes.send(Change::Profile);
        Ok(result)
    }

    pub fn devices(&self) -> Rpc<&Devices> {
        self.devices
            .as_ref()
            .ok_or_else(|| failed("This host is private to one terminal; run `mu serve` to pair devices"))
    }

    pub fn invite(&self) -> Rpc<wire::Invitation> {
        let devices = self.devices()?;
        let network = self.network.as_ref().ok_or_else(|| {
            failed("This host does not listen on the network; set MU_LISTEN, like MU_LISTEN=0.0.0.0:7337")
        })?;
        Ok(wire::Invitation {
            code: devices.invite().map_err(internal)?,
            cert: network.cert.clone(),
            port: network.bound.port(),
            addresses: net::addresses(network.bound.ip()),
        })
    }

    pub fn pair(&self, call: wire::Pair) -> Rpc<String> {
        self.devices()?.pair(&call.code, &call.device).map_err(internal)?.ok_or_else(|| {
            Failure(StatusCode::FORBIDDEN, "This invite was already used or has expired; make a new one".into())
        })
    }

    pub fn unpair(&self, call: wire::Unpair) -> Rpc<()> {
        match self.devices()?.unpair(&call.device_id).map_err(internal)? {
            true => Ok(()),
            false => Err(Failure(StatusCode::NOT_FOUND, format!("No device {}", call.device_id))),
        }
    }

    pub fn load(&self, agents: &mut HashMap<String, Arc<Agent>>, id: &str) -> Rpc<Arc<Agent>> {
        if let Some(agent) = agents.get(id) {
            return Ok(agent.clone());
        }
        let agent = Agent::load(id, self.changed(), self.bridge.clone())
            .map_err(|e| Failure(StatusCode::NOT_FOUND, format!("Session {id}: {e}")))?;
        agents.insert(id.into(), agent.clone());
        Ok(agent)
    }

    pub fn agent(&self, id: &str) -> Rpc<Arc<Agent>> {
        self.load(&mut self.agents.lock().unwrap(), id)
    }

    pub fn sweep(&self) {
        self.agents.lock().unwrap().retain(|_, agent| !agent.disposable());
    }

    pub fn sessions(&self) -> Rpc<Vec<wire::SessionSummary>> {
        self.sweep();
        let loaded: HashMap<_, _> =
            self.agents.lock().unwrap().iter().map(|(id, agent)| (id.clone(), agent.status())).collect();
        Ok(summary::sessions()
            .map_err(failed)?
            .into_iter()
            .map(|summary| {
                let status = loaded.get(&summary.id).copied();
                summarize(summary, status)
            })
            .collect())
    }

    pub fn everything(&self) -> Rpc<WatchEvent> {
        Ok(WatchEvent::Host { profile: self.profile.get(), sessions: self.sessions()? })
    }

    pub fn summary(&self, id: &str) -> Option<wire::SessionSummary> {
        let status = self.agents.lock().unwrap().get(id).map(|agent| agent.status());
        Some(summarize(summary::summary_of(id)?, status))
    }

    pub fn create(&self, call: wire::CreateSession) -> Rpc<String> {
        directory(&call.cwd)?;
        let parent = call.parent.as_deref().map(|id| self.agent(id)).transpose()?;
        let model = match &parent {
            Some(parent) if call.model.name.is_empty() => {
                let model = parent.model();
                wire::Model { effort: call.model.effort.filter(|effort| !effort.is_empty()).or(model.effort), ..model }
            }
            _ => self.resolve(call.model),
        };
        agent::check(&model).map_err(failed)?;
        let from = call.parent.as_deref().map(|id| self.sender(id)).transpose()?;
        let agent = Agent::new(call.cwd, call.parent, model, self.changed(), self.bridge.clone());
        let id = agent.id();
        self.agents.lock().unwrap().insert(id.clone(), agent.clone());
        if let Err(e) = agent.send(call.text, call.skill.as_deref(), from) {
            self.agents.lock().unwrap().remove(&id);
            return Err(failed(e));
        }
        Ok(id)
    }

    fn sender(&self, id: &str) -> Rpc<wire::Sender> {
        Ok(wire::Sender { session: id.into(), title: self.agent(id)?.title() })
    }

    pub fn send(&self, call: wire::SendMessage) -> Rpc<()> {
        let agent = self.agent(&call.session_id)?;
        let from = call.from.as_deref().map(|id| self.sender(id)).transpose()?;
        agent.send(call.text, call.skill.as_deref(), from).map_err(failed)
    }

    pub fn stop(&self, call: wire::Stop) -> Rpc<()> {
        let agent = self.agent(&call.session_id)?;
        agent.stop();
        if !call.children {
            return Ok(());
        }
        // Only loaded sessions can be running, so the rest need no looking at.
        let agents = self.agents.lock().unwrap();
        let mut stopped = HashSet::from([call.session_id]);
        loop {
            let found: Vec<_> = agents
                .iter()
                .filter(|(id, agent)| !stopped.contains(*id) && agent.parent().is_some_and(|p| stopped.contains(&p)))
                .map(|(id, agent)| (id.clone(), agent))
                .collect();
            if found.is_empty() {
                return Ok(());
            }
            for (id, agent) in found {
                agent.stop();
                stopped.insert(id);
            }
        }
    }

    pub fn list_models(&self) -> Rpc<Vec<wire::ModelInfo>> {
        let models = api::models().map_err(failed)?;
        *self.models.lock().unwrap() = Some(models.clone());
        Ok(models)
    }

    // Fills in how much context the model takes, when the endpoint says.
    pub fn resolve(&self, mut model: wire::Model) -> wire::Model {
        model.effort = model.effort.filter(|effort| !effort.is_empty());
        if model.context.is_none() {
            let mut models = self.models.lock().unwrap();
            let known = models.get_or_insert_with(|| api::models().unwrap_or_default());
            model.context = known.iter().find(|known| known.name == model.name).and_then(|known| known.context);
        }
        model
    }

    pub fn complete(&self, call: wire::CompleteMention) -> Rpc<Option<wire::Mention>> {
        absolute(&call.cwd)?;
        let Some((range, query)) = input::mention_at(&call.text, call.cursor) else {
            return Ok(None);
        };
        let files = if input::is_path(&query) {
            Arc::default()
        } else {
            let mut mentions = self.mentions.lock().unwrap();
            match &*mentions {
                Some((cwd, files)) if !call.refresh && *cwd == call.cwd => files.clone(),
                _ => {
                    let built = input::index(&call.cwd);
                    let files = Arc::new(built.as_ref().cloned().unwrap_or_default());
                    *mentions = Some((call.cwd.clone(), files.clone()));
                    built.map_err(failed)?;
                    files
                }
            }
        };
        Ok(Some(input::complete(range, query, &call.cwd, &files)))
    }

    // Returns the id a session takes the run in by.
    pub fn keep(&self, run: tools::Run) -> String {
        let id = crate::unique_id();
        let mut runs = self.runs.lock().unwrap();
        runs.push_back((id.clone(), run));
        if runs.len() > RUNS {
            runs.pop_front();
        }
        id
    }

    pub fn append(&self, call: wire::AppendShell) -> Rpc<()> {
        let agent = self.agent(&call.session_id)?;
        agent
            .append(|| {
                let mut runs = self.runs.lock().unwrap();
                let i = runs
                    .iter()
                    .position(|(id, _)| *id == call.run)
                    .ok_or("That command's result is no longer kept; run it again")?;
                Ok(runs.remove(i).expect("found").1)
            })
            .map_err(failed)
    }

    pub fn delete_sessions(&self) -> Rpc<wire::Deleted> {
        let mut agents = self.agents.lock().unwrap();
        agents.retain(|_, agent| !agent.disposable());
        let mut deleted = wire::Deleted { sessions: 0, in_use: 0 };
        for summary in summary::sessions().map_err(internal)? {
            let gone = !agents.contains_key(&summary.id) && session::delete(&summary.id).map_err(internal)?;
            match gone {
                true => deleted.sessions += 1,
                false => deleted.in_use += 1,
            }
        }
        drop(agents);
        let _ = self.changes.send(Change::Sessions);
        Ok(deleted)
    }
}

fn summarize(summary: Summary, loaded: Option<Status>) -> wire::SessionSummary {
    let status = loaded.unwrap_or(summary.status);
    wire::SessionSummary {
        unread: summary.unread && status != Status::Running,
        updated: summary.updated,
        error: summary.error.filter(|_| status == Status::Failed),
        status,
        id: summary.id,
        title: summary.title,
        cwd: summary.cwd,
        parent: summary.parent,
    }
}
