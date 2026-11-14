use mu_tui::Endpoint;
use std::{env, os::unix::net::UnixStream, path::PathBuf, thread};

fn private() -> mu_host::Result<()> {
    let (client, server) = UnixStream::pair()?;
    let host = thread::spawn(move || mu_host::serve_stream(server));
    let result = mu_tui::run(Endpoint::Stream(client));
    let _ = host.join();
    result
}

fn shared() -> Endpoint {
    Endpoint::Socket(mu_host::socket_path())
}

fn main() -> mu_host::Result<()> {
    let args: Vec<_> = env::args_os().skip(1).collect();
    match args.first().and_then(|arg| arg.to_str()) {
        None => match env::var_os("MU_SOCKET").filter(|path| !path.is_empty()) {
            Some(socket) => mu_tui::run(Endpoint::Socket(socket.into())),
            None => private(),
        },
        Some("serve") if args.len() == 1 => mu_host::serve(&mu_host::socket_path(), mu_host::listen_address()?),
        Some("pair") if args.len() <= 2 && args.iter().all(|arg| arg.to_str().is_some()) => {
            mu_tui::pair(shared(), args.get(1).and_then(|arg| arg.to_str()))
        }
        Some("devices") if args.len() == 1 => mu_tui::devices(shared()),
        Some("unpair") if args.len() == 2 && args[1].to_str().is_some() => {
            mu_tui::unpair(shared(), args[1].to_str().unwrap_or_default())
        }
        Some("--view-image") => {
            if args.len() != 2 {
                return Err("Usage: view_image /path/to/image".into());
            }
            mu_host::tools::view_image(&PathBuf::from(&args[1]))
        }
        Some(_) => {
            println!(
                "mu · µ · 無\n\nMU_BASE_URL=http://127.0.0.1:8317/v1 MU_API_KEY=… MU_MODEL=… MU_EFFORT=… MU_CONTEXT=… mu\n\nmu serve          run a shared host for other clients\nMU_SOCKET=… mu    join the shared host at that socket\n\nMU_LISTEN=0.0.0.0:7337 mu serve    also let paired devices in over the network\nmu pair [address]                 pair one more device (shows a QR code)\nmu devices                        list paired devices\nmu unpair <id>                    unpair one"
            );
            Ok(())
        }
    }
}
