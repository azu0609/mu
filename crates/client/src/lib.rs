use connectrpc::client::{ClientConfig, Http2Connection, SharedHttp2Connection};
use hyper_util::rt::TokioIo;
use mu_proto::SessionServiceClient;
use std::{
    io,
    os::unix::net::UnixStream as StdStream,
    path::PathBuf,
    sync::{Arc, Mutex},
};
use tokio::net::UnixStream;

pub use connectrpc::ConnectError;
pub use mu_proto as proto;

pub type Client = SessionServiceClient<SharedHttp2Connection>;

pub enum Endpoint {
    // A host listening for any number of clients.
    Socket(PathBuf),
    // One end of a socket pair whose other end a host is already serving:
    // a private host that nothing else can reach.
    Stream(StdStream),
}

// Call from within a tokio runtime. A socket connection re-establishes itself
// if the host restarts; a private stream ends with its host.
pub async fn connect(endpoint: Endpoint) -> io::Result<Client> {
    let authority: http::Uri = "http://localhost".parse().unwrap();
    let connection = match endpoint {
        Endpoint::Socket(path) => {
            // Fail now, with the real reason, rather than on the first call.
            UnixStream::connect(&path).await?;
            Http2Connection::lazy_unix(path, authority.clone())
        }
        Endpoint::Stream(stream) => {
            stream.set_nonblocking(true)?;
            let stream = Arc::new(Mutex::new(Some(UnixStream::from_std(stream)?)));
            Http2Connection::lazy_with_connector(
                tower::service_fn(move |_| {
                    let stream = stream.lock().unwrap().take();
                    async move { stream.map(TokioIo::new).ok_or_else(|| io::Error::other("The mu host is gone")) }
                }),
                authority.clone(),
            )
        }
    };
    // One user message can approach the host's 4 MiB request limit, and its echo must fit.
    let config = ClientConfig::new(authority).with_default_max_message_size(64 * 1024 * 1024);
    Ok(Client::new(connection.shared(64), config))
}
