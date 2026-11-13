// The host, as the synchronous UI loop sees it: short calls block, while the
// session's events and slow answers arrive on a channel.
use crate::Result;
use mu_client::{Client, ConnectError, Endpoint, proto as pb};
use std::{future::Future, path::Path, sync::mpsc, time::Duration};
use tokio::{runtime::Runtime, task::JoinHandle, time};

pub enum Incoming {
    // Tagged with the subscription they belong to.
    Event(u64, pb::Event),
    Lost(u64),
    Mention(u64, Result<Option<pb::Mention>>),
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

fn error(e: ConnectError) -> Box<dyn std::error::Error + Send + Sync> {
    e.message.clone().unwrap_or_else(|| e.to_string()).into()
}

// Follows one session for as long as the task lives, resubscribing (and so
// receiving a fresh snapshot) whenever the stream breaks. Only the first
// attempt reports failure to the caller: that session cannot be opened.
async fn follow(
    client: Client,
    id: String,
    generation: u64,
    tx: mpsc::Sender<Incoming>,
    ready: mpsc::SyncSender<Result<()>>,
) {
    let mut ready = Some(ready);
    loop {
        let request = pb::SubscribeRequest { session_id: id.clone(), ..Default::default() };
        let failure = match client.subscribe(request).await {
            Ok(mut stream) => loop {
                match stream.message().await {
                    Ok(Some(message)) => {
                        if let Some(ready) = ready.take() {
                            let _ = ready.send(Ok(()));
                        }
                        if let Some(event) = message.to_owned_message().event
                            && tx.send(Incoming::Event(generation, event)).is_err()
                        {
                            return;
                        }
                    }
                    Ok(None) => break None,
                    Err(e) => break Some(e),
                }
            },
            Err(e) => Some(e),
        };
        if let Some(ready) = ready.take() {
            let _ = ready.send(Err(failure.map_or_else(|| "The host closed the session stream".into(), error)));
            return;
        }
        if tx.send(Incoming::Lost(generation)).is_err() {
            return;
        }
        time::sleep(Duration::from_secs(1)).await;
    }
}

impl Remote {
    pub fn connect(endpoint: Endpoint) -> Result<Self> {
        let runtime = tokio::runtime::Builder::new_multi_thread().worker_threads(1).enable_all().build()?;
        let place = match &endpoint {
            Endpoint::Socket(path) => format!(" at {}", path.display()),
            Endpoint::Stream(_) => String::new(),
        };
        let client = runtime
            .block_on(mu_client::connect(endpoint))
            .map_err(|e| format!("Cannot reach the mu host{place}: {e}"))?;
        let (tx, rx) = mpsc::channel();
        Ok(Self { runtime, client, tx, rx, subscription: None, subscriptions: 0, generation: 0 })
    }

    fn call<T, F>(&self, f: impl FnOnce(Client) -> F) -> Result<T>
    where
        F: Future<Output = std::result::Result<T, ConnectError>>,
    {
        let client = self.client.clone();
        self.runtime
            .block_on(async { time::timeout(Duration::from_secs(10), f(client)).await })
            .map_err(|_| "The mu host did not answer")?
            .map_err(error)
    }

    // Switches to another session, or stays on the current one if it cannot be opened.
    pub fn attach(&mut self, id: &str) -> Result<()> {
        self.subscriptions += 1;
        let (ready, opened) = mpsc::sync_channel(1);
        let task =
            self.runtime.spawn(follow(self.client.clone(), id.into(), self.subscriptions, self.tx.clone(), ready));
        match opened.recv_timeout(Duration::from_secs(10)).unwrap_or_else(|_| Err("The mu host did not answer".into()))
        {
            Ok(()) => {
                if let Some(old) = self.subscription.replace(task) {
                    old.abort();
                }
                self.generation = self.subscriptions;
                Ok(())
            }
            Err(e) => {
                task.abort();
                Err(e)
            }
        }
    }

    pub fn create(
        &self,
        cwd: &Path,
        model: Option<String>,
        effort: Option<String>,
        context: Option<u64>,
    ) -> Result<String> {
        let request = pb::CreateSessionRequest {
            cwd: cwd.to_string_lossy().into_owned(),
            model,
            effort,
            context,
            ..Default::default()
        };
        self.call(|client| async move { Ok(client.create_session(request).await?.into_owned().id) })
    }

    pub fn sessions(&self) -> Result<Vec<pb::SessionSummary>> {
        self.call(|client| async move { Ok(client.list_sessions(Default::default()).await?.into_owned().sessions) })
    }

    pub fn send(&self, id: &str, text: String, skill: Option<String>) -> Result<()> {
        let request = pb::SendRequest { session_id: id.into(), text, skill, ..Default::default() };
        self.call(|client| async move { client.send(request).await.map(drop) })
    }

    pub fn stop(&self, id: &str) -> Result<()> {
        let request = pb::StopRequest { session_id: id.into(), ..Default::default() };
        self.call(|client| async move { client.stop(request).await.map(drop) })
    }

    pub fn recall(&self, id: &str) -> Result<Option<String>> {
        let request = pb::RecallQueuedRequest { session_id: id.into(), ..Default::default() };
        self.call(|client| async move { Ok(client.recall_queued(request).await?.into_owned().text) })
    }

    pub fn set_model(&self, id: &str, model: pb::Model) -> Result<()> {
        let request = pb::SetModelRequest { session_id: id.into(), model: model.into(), ..Default::default() };
        self.call(|client| async move { client.set_model(request).await.map(drop) })
    }

    pub fn tree(&self, id: &str) -> Result<Vec<pb::TreeEntry>> {
        let request = pb::GetTreeRequest { session_id: id.into(), ..Default::default() };
        self.call(|client| async move { Ok(client.get_tree(request).await?.into_owned().entries) })
    }

    pub fn checkout(&self, id: &str, node: Option<u64>) -> Result<Option<String>> {
        let request = pb::CheckoutRequest { session_id: id.into(), node, ..Default::default() };
        self.call(|client| async move { Ok(client.checkout(request).await?.into_owned().draft) })
    }

    // Indexing a large tree takes a while, so the answer arrives as `Incoming::Mention(seq, …)`.
    pub fn complete(&self, seq: u64, id: &str, text: String, cursor: usize, refresh: bool) {
        let request = pb::CompleteMentionRequest {
            session_id: id.into(),
            text,
            cursor: cursor as u32,
            refresh,
            ..Default::default()
        };
        let client = self.client.clone();
        let tx = self.tx.clone();
        self.runtime.spawn(async move {
            let result = client.complete_mention(request).await;
            let _ =
                tx.send(Incoming::Mention(seq, result.map(|r| r.into_owned().mention.into_option()).map_err(error)));
        });
    }
}
