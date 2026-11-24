use crate::Result;
use mu_client::{
    Call, Client, CompleteMention, Endpoint, Event, ListModels, MarkRead, Mention, ModelInfo, ReadImage, ShellEvent,
    Update, WatchEvent,
};
use std::{
    future::Future,
    path::{Path, PathBuf},
    sync::mpsc,
    time::Duration,
};
use tokio::{runtime::Runtime, task::JoinHandle, time};

pub enum Incoming {
    Event(u64, Event),
    Lost(u64),
    Mention(u64, Result<Option<Mention>>),
    Host(WatchEvent),
    Shell(u64, Result<ShellEvent>),
    Models(Result<Vec<ModelInfo>>),
    Image(PathBuf, Result<String>),
}

pub struct Remote {
    runtime: Runtime,
    client: Client,
    tx: mpsc::Sender<Incoming>,
    pub rx: mpsc::Receiver<Incoming>,
    subscription: Option<JoinHandle<()>>,
    subscriptions: u64,
    shell: Option<JoinHandle<()>>,
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
        let mut watch = client.watch();
        runtime.block_on(watch.open())?;
        let host = tx.clone();
        runtime.spawn(async move {
            loop {
                if let Update::Event(event) = watch.next().await
                    && host.send(Incoming::Host(event)).is_err()
                {
                    return;
                }
            }
        });
        Ok(Self { runtime, client, tx, rx, subscription: None, subscriptions: 0, shell: None, generation: 0 })
    }

    fn wait<T>(&self, answer: impl Future<Output = mu_client::Result<T>>) -> Result<T> {
        Ok(self
            .runtime
            .block_on(async { time::timeout(Duration::from_secs(10), answer).await })
            .map_err(|_| "The mu host did not answer")??)
    }

    pub fn call<C: Call + Sync>(&self, call: C) -> Result<C::Reply> {
        self.wait(self.client.call(call))
    }

    pub fn attach(&mut self, id: &str) -> Result<()> {
        let mut subscription = self.client.subscribe(id, None);
        self.wait(subscription.open())?;
        self.subscriptions += 1;
        let generation = self.subscriptions;
        let tx = self.tx.clone();
        let task = self.runtime.spawn(async move {
            loop {
                let incoming = match subscription.next().await {
                    Update::Event(event) => Incoming::Event(generation, event),
                    Update::Lost(_) => Incoming::Lost(generation),
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

    pub fn detach(&mut self) {
        if let Some(old) = self.subscription.take() {
            old.abort();
        }
        self.subscriptions += 1;
        self.generation = self.subscriptions;
    }

    pub fn mark_read(&self, id: &str) {
        let (client, session_id) = (self.client.clone(), id.to_owned());
        self.runtime.spawn(async move { client.call(MarkRead { session_id }).await });
    }

    pub fn shell(&mut self, seq: u64, cwd: &Path, command: String) -> Result<()> {
        let mut run = self.wait(self.client.shell(cwd, command))?;
        let tx = self.tx.clone();
        let task = self.runtime.spawn(async move {
            loop {
                let (incoming, done) = match run.next().await {
                    Ok(Some(event)) => {
                        let done = matches!(event, ShellEvent::Done { .. });
                        (Incoming::Shell(seq, Ok(event)), done)
                    }
                    Ok(None) => {
                        (Incoming::Shell(seq, Err("The host stopped sending the command's output".into())), true)
                    }
                    Err(e) => (Incoming::Shell(seq, Err(e.into())), true),
                };
                if tx.send(incoming).is_err() || done {
                    return;
                }
            }
        });
        if let Some(old) = self.shell.replace(task) {
            old.abort();
        }
        Ok(())
    }

    pub fn cancel_shell(&mut self) {
        if let Some(task) = self.shell.take() {
            task.abort();
        }
    }

    pub fn complete(&self, seq: u64, call: CompleteMention) {
        let client = self.client.clone();
        let tx = self.tx.clone();
        self.runtime.spawn(async move {
            let result = client.call(call).await;
            let _ = tx.send(Incoming::Mention(seq, result.map_err(Into::into)));
        });
    }

    pub fn list_models(&self) {
        let client = self.client.clone();
        let tx = self.tx.clone();
        self.runtime.spawn(async move {
            let result = client.call(ListModels {}).await;
            let _ = tx.send(Incoming::Models(result.map_err(Into::into)));
        });
    }

    pub fn read_image(&self, path: PathBuf) {
        let client = self.client.clone();
        let tx = self.tx.clone();
        self.runtime.spawn(async move {
            let result = client.call(ReadImage { path: path.clone() }).await;
            let _ = tx.send(Incoming::Image(path, result.map_err(Into::into)));
        });
    }
}
