// The special commands in a host's bash runs. The host links each under its
// name to this binary, which answers to the name it is run by.
mod agent;
mod view_image;

use mu_client::{Call, Client, Endpoint};
use mu_host::Result;
use std::env;
use tokio::runtime::Runtime;

type Command = fn(&Run, &[String]) -> Result<()>;

const COMMANDS: &[(&str, Command)] = &[("view_image", view_image::run), ("agent", agent::run)];

pub fn names() -> Vec<&'static str> {
    COMMANDS.iter().map(|(name, _)| *name).collect()
}

// The bash run a command is in, reached through its host.
pub struct Run {
    runtime: Runtime,
    client: Client,
    pub token: String,
}

impl Run {
    fn new(host: Endpoint, token: String) -> Result<Self> {
        let runtime = tokio::runtime::Builder::new_current_thread().enable_all().build()?;
        let client = runtime.block_on(crate::connect(host))?;
        Ok(Self { runtime, client, token })
    }

    pub fn call<C: Call + Sync>(&self, call: C) -> Result<C::Reply> {
        Ok(self.runtime.block_on(self.client.call(call))?)
    }
}

pub fn find(name: &str) -> Option<impl FnOnce(&[String]) -> Result<()>> {
    let (name, command) = COMMANDS.iter().find(|(command, _)| *command == name)?;
    Some(move |args: &[String]| {
        let (Some(host), Ok(token)) = (env::var_os("MU_HOST"), env::var("MU_RUN")) else {
            return Err(format!("{name} only works in mu's bash").into());
        };
        command(&Run::new(Endpoint::Socket(host.into()), token)?, args)
    })
}
