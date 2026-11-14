// The host, as the synchronous UI loop sees it: short calls block, while the
// session's events and slow answers arrive on a channel.
use crate::Result;
use mu_client::{Client, Device, Endpoint, Event, Invitation, Mention, Model, TreeEntry, Update, WatchEvent};
use std::{future::Future, path::Path, sync::mpsc, time::Duration};
use tokio::{runtime::Runtime, task::JoinHandle, time};

pub enum Incoming {
    // Tagged with the subscription they belong to.
    Event(u64, Event),
    Lost(u64),
    Mention(u64, Result<Option<Mention>>),
    // The host's session list changed.
    Sessions(WatchEvent),
}

pub struct Remote {
    runtime: Runtime,
    client: Client,
    tx: mpsc::Sender<Incoming>,
    pub rx: mpsc::Receiver<Incoming>,
    subscription: Option<JoinHandle<()>>,
    subscriptions: u64,
    // The subscription whose events are current; older ones may still be in the channel.
    pub generation: u64,
}

impl Remote {
    pub fn connect(endpoint: Endpoint) -> Result<Self> {
        let runtime = tokio::runtime::Builder::new_multi_thread().worker_threads(1).enable_all().build()?;
        let place = match &endpoint {
            Endpoint::Socket(path) => format!(" at {}", path.display()),
            Endpoint::Stream(_) => String::new(),
            Endpoint::Remote(remote) => format!(" at {}", remote.address),
        };
        let client =
            runtime.block_on(Client::connect(endpoint)).map_err(|e| format!("Cannot reach the mu host{place}: {e}"))?;
        let (tx, rx) = mpsc::channel();
        let mut watch = runtime.block_on(client.watch())?;
        let sessions = tx.clone();
        runtime.spawn(async move {
            loop {
                // A break needs no word here; the list starts over when the host is back.
                if let Update::Event(event) = watch.next().await
                    && sessions.send(Incoming::Sessions(event)).is_err()
                {
                    return;
                }
            }
        });
        Ok(Self { runtime, client, tx, rx, subscription: None, subscriptions: 0, generation: 0 })
    }

    fn call<T>(&self, call: impl Future<Output = mu_client::Result<T>>) -> Result<T> {
        Ok(self
            .runtime
            .block_on(async { time::timeout(Duration::from_secs(10), call).await })
            .map_err(|_| "The mu host did not answer")??)
    }

    // Switches to another session, or stays on the current one if it cannot be opened.
    pub fn attach(&mut self, id: &str) -> Result<()> {
        let mut subscription = self.call(self.client.subscribe(id))?;
        self.subscriptions += 1;
        let generation = self.subscriptions;
        let tx = self.tx.clone();
        let task = self.runtime.spawn(async move {
            loop {
                let incoming = match subscription.next().await {
                    Update::Event(event) => Incoming::Event(generation, event),
                    Update::Lost => Incoming::Lost(generation),
                };
                if tx.send(incoming).is_err() {
                    return;
                }
            }
        });
        if let Some(old) = self.subscription.replace(task) {
            old.abort();
        }
        self.generation = generation;
        Ok(())
    }

    pub fn create(
        &self,
        cwd: &Path,
        model: Option<String>,
        effort: Option<String>,
        context: Option<u64>,
    ) -> Result<String> {
        self.call(self.client.create(cwd, model, effort, context))
    }

    // Fire and forget: at worst the session stays unread.
    pub fn mark_read(&self, id: &str) {
        let (client, id) = (self.client.clone(), id.to_owned());
        self.runtime.spawn(async move { client.mark_read(&id).await });
    }

    pub fn send(&self, id: &str, text: String, skill: Option<String>) -> Result<()> {
        self.call(self.client.send(id, text, skill))
    }

    pub fn stop(&self, id: &str) -> Result<()> {
        self.call(self.client.stop(id))
    }

    pub fn recall(&self, id: &str) -> Result<Option<String>> {
        self.call(self.client.recall(id))
    }

    pub fn set_model(&self, id: &str, model: Model) -> Result<()> {
        self.call(self.client.set_model(id, model))
    }

    pub fn tree(&self, id: &str) -> Result<Vec<TreeEntry>> {
        self.call(self.client.tree(id))
    }

    pub fn checkout(&self, id: &str, node: Option<usize>) -> Result<Option<String>> {
        self.call(self.client.checkout(id, node))
    }

    pub fn invite(&self) -> Result<Invitation> {
        self.call(self.client.invite())
    }

    pub fn devices(&self) -> Result<Vec<Device>> {
        self.call(self.client.devices())
    }

    pub fn unpair(&self, id: &str) -> Result<()> {
        self.call(self.client.unpair(id))
    }

    // Indexing a large tree takes a while, so the answer arrives as `Incoming::Mention(seq, …)`.
    pub fn complete(&self, seq: u64, id: &str, text: String, cursor: usize, refresh: bool) {
        let client = self.client.clone();
        let id = id.to_owned();
        let tx = self.tx.clone();
        self.runtime.spawn(async move {
            let result = client.complete(&id, text, cursor, refresh).await;
            let _ = tx.send(Incoming::Mention(seq, result.map_err(Into::into)));
        });
    }
}
