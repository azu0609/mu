use mu_tui::Endpoint;
use std::{env, os::unix::net::UnixStream, path::PathBuf, thread};

// By default the terminal is the only client of a host living in this very
// process: it has this terminal's environment and ends with it. The two still
// only speak RPC, over a socket pair nothing else can reach.
fn private() -> mu_core::Result<()> {
    let (client, server) = UnixStream::pair()?;
    let host = thread::spawn(move || mu_server::serve_stream(server));
    let result = mu_tui::run(Endpoint::Stream(client));
    // The client's end is closed now; wait for the host to stop what it was running.
    let _ = host.join();
    result
}

fn main() -> mu_core::Result<()> {
    let args: Vec<_> = env::args_os().skip(1).collect();
    match args.first().and_then(|arg| arg.to_str()) {
        // Naming a socket means joining the shared host there instead.
        None => match env::var_os("MU_SOCKET").filter(|path| !path.is_empty()) {
            Some(socket) => mu_tui::run(Endpoint::Socket(socket.into())),
            None => private(),
        },
        Some("serve") if args.len() == 1 => mu_server::serve(&mu_proto::socket_path()),
        // The bash tool's `view_image` command re-enters here.
        Some("--view-image") => {
            if args.len() != 2 {
                return Err("Usage: view_image /path/to/image".into());
            }
            mu_core::tools::view_image(&PathBuf::from(&args[1]))
        }
        Some(_) => {
            println!(
                "mu · µ · 無\n\nMU_BASE_URL=http://127.0.0.1:8317/v1 MU_API_KEY=… MU_MODEL=… MU_EFFORT=… MU_CONTEXT=… mu\n\nmu serve          run a shared host for other clients\nMU_SOCKET=… mu    join the shared host at that socket"
            );
            Ok(())
        }
    }
}
