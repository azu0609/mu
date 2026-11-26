mod commands;
mod devices;

use mu_client::{Client, Endpoint};
use std::{
    env,
    ffi::{OsStr, OsString},
    path::Path,
    process,
};

async fn connect() -> mu_host::Result<Client> {
    let socket = mu_host::socket_path();
    Client::connect(Endpoint::Socket(socket.clone()))
        .await
        .map_err(|e| format!("Cannot reach the mu host at {}: {e}", socket.display()).into())
}

fn block_on(future: impl Future<Output = mu_host::Result<()>>) -> mu_host::Result<()> {
    tokio::runtime::Builder::new_current_thread().enable_all().build()?.block_on(future)
}

fn command(name: &OsStr, args: &[OsString]) -> Option<mu_host::Result<()>> {
    let command = commands::find(Path::new(name).file_name()?.to_str()?)?;
    let args: Option<Vec<_>> = args.iter().map(|arg| arg.to_str().map(str::to_owned)).collect();
    Some(args.ok_or_else(|| "Arguments must be UTF-8".into()).and_then(|args| command(&args)))
}

fn main() -> mu_host::Result<()> {
    let mut args = env::args_os();
    let name = args.next().unwrap_or_default();
    let args: Vec<_> = args.collect();
    if let Some(result) = command(&name, &args) {
        if let Err(e) = result {
            eprintln!("{e}");
            process::exit(1);
        }
        return Ok(());
    }
    match args.first().and_then(|arg| arg.to_str()) {
        None => mu_tui::run(Endpoint::Socket(mu_host::socket_path())),
        Some("serve") if args.len() == 1 => {
            mu_host::serve(&mu_host::socket_path(), mu_host::listen_address()?, commands::names())
        }
        Some("pair") if args.len() <= 2 && args.iter().all(|arg| arg.to_str().is_some()) => {
            block_on(devices::pair(args.get(1).and_then(|arg| arg.to_str())))
        }
        Some("devices") if args.len() == 1 => block_on(devices::devices()),
        Some("unpair") if args.len() == 2 && args[1].to_str().is_some() => {
            block_on(devices::unpair(args[1].to_str().unwrap_or_default()))
        }
        Some(_) => {
            println!(
                "mu · µ · 無\n\nMU_BASE_URL=http://127.0.0.1:8317/v1 MU_API_KEY=… mu serve    run the host\nMU_MODEL=… MU_EFFORT=… MU_CONTEXT=… mu                          talk to it\nMU_SOCKET=…                                                     where they meet\n\nMU_LISTEN=0.0.0.0:7337 mu serve    also let paired devices in over the network\nmu pair [address]                 pair one more device (shows a QR code)\nmu devices                        list paired devices\nmu unpair <id>                    unpair one"
            );
            Ok(())
        }
    }
}
