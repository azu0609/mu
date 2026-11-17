use crate::{
    directory,
    host::{Failure, Host, Rpc, absolute, failed, internal},
    image, input, skills,
    stream::{self, Stream},
    upload,
};
use base64::{Engine, engine::general_purpose::STANDARD};
use http_body_util::{BodyExt, Either, Full, Limited};
use hyper::{
    Method, Request, Response, StatusCode,
    body::{Bytes, Incoming},
    header::{self, HeaderValue},
};
use mu_wire::{self as wire, Call};
use serde::{Serialize, de::DeserializeOwned};
use std::sync::Arc;

#[derive(Clone, Copy, PartialEq)]
pub enum Peer {
    Local,
    Remote,
}

type Reply = Response<Either<Full<Bytes>, Stream>>;

fn reply(status: StatusCode, value: &impl Serialize) -> Reply {
    let body = match serde_json::to_vec(value) {
        Ok(body) => body,
        Err(e) => return failure(internal(e)),
    };
    let mut response = Response::new(Either::Left(Full::new(body.into())));
    *response.status_mut() = status;
    response.headers_mut().insert(header::CONTENT_TYPE, HeaderValue::from_static("application/json"));
    response
}

fn failure(Failure(status, message): Failure) -> Reply {
    reply(status, &wire::Failure { message })
}

fn feed(lines: Rpc<Stream>) -> Reply {
    let lines = match lines {
        Ok(lines) => lines,
        Err(e) => return failure(e),
    };
    let mut response = Response::new(Either::Right(lines));
    response.headers_mut().insert(header::CONTENT_TYPE, HeaderValue::from_static("application/x-ndjson"));
    response
}

fn parse<T: DeserializeOwned>(body: &[u8]) -> Rpc<T> {
    let body = if body.is_empty() { b"{}".as_slice() } else { body };
    serde_json::from_slice(body).map_err(failed)
}

async fn call<C: Call + Send + 'static>(
    host: &Arc<Host>,
    body: &[u8],
    f: impl FnOnce(&Arc<Host>, C) -> Rpc<C::Reply> + Send + 'static,
) -> Reply
where
    C::Reply: Send + 'static,
{
    let call = match parse(body) {
        Ok(call) => call,
        Err(e) => return failure(e),
    };
    match host.run(move |host| f(host, call)).await {
        Ok(value) => reply(StatusCode::OK, &value),
        Err(e) => failure(e),
    }
}

async fn streamed<C: DeserializeOwned + Send + 'static>(
    host: &Arc<Host>,
    body: &[u8],
    f: impl FnOnce(&Arc<Host>, C) -> Rpc<Stream> + Send + 'static,
) -> Reply {
    match parse(body) {
        Ok(call) => feed(host.run(move |host| f(host, call)).await),
        Err(e) => failure(e),
    }
}

fn admit(host: &Host, peer: Peer, request: &Request<Incoming>) -> Rpc<()> {
    let path = request.uri().path();
    if peer == Peer::Local || path == wire::Hello::PATH || path == wire::Pair::PATH {
        return Ok(());
    }
    let token = request
        .headers()
        .get(header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.strip_prefix("Bearer "));
    if !token.is_some_and(|token| host.devices.as_ref().is_some_and(|devices| devices.authorized(token))) {
        return Err(Failure(StatusCode::UNAUTHORIZED, format!("This device is not paired with {}", host.name)));
    }
    if [wire::CreateInvite::PATH, wire::ListDevices::PATH, wire::Unpair::PATH].contains(&path) {
        return Err(Failure(StatusCode::FORBIDDEN, "Only local clients can manage devices".into()));
    }
    if [wire::RunSession::PATH, wire::ViewImage::PATH].contains(&path) {
        return Err(Failure(StatusCode::FORBIDDEN, "Only commands in the host's bash can make this call".into()));
    }
    Ok(())
}

pub async fn handle(host: Arc<Host>, peer: Peer, request: Request<Incoming>) -> Reply {
    if request.method() != Method::POST {
        return failure(Failure(StatusCode::METHOD_NOT_ALLOWED, "Calls are POSTs".into()));
    }
    if let Err(e) = admit(&host, peer, &request) {
        return failure(e);
    }
    let path = request.uri().path().to_owned();
    let body = match Limited::new(request.into_body(), 4 * 1024 * 1024).collect().await {
        Ok(body) => body.to_bytes(),
        Err(e) => return failure(Failure(StatusCode::PAYLOAD_TOO_LARGE, e.to_string())),
    };
    let (host, body) = (&host, &body[..]);
    match path.as_str() {
        wire::Hello::PATH => {
            call(host, body, |host, wire::Hello {}| {
                Ok(wire::HostInfo {
                    protocol: wire::PROTOCOL,
                    version: env!("CARGO_PKG_VERSION").into(),
                    name: host.name.clone(),
                })
            })
            .await
        }
        wire::Watch::PATH => feed(stream::watch(host).await),
        wire::Subscribe::PATH => streamed(host, body, stream::subscribe).await,
        wire::Shell::PATH => streamed(host, body, stream::shell).await,
        wire::CreateSession::PATH => call(host, body, |host, call| host.create(call)).await,
        wire::SendMessage::PATH => call(host, body, |host, call| host.send(call)).await,
        wire::Stop::PATH => call(host, body, |host, call| host.stop(call)).await,
        wire::MarkRead::PATH => {
            call(host, body, |host, call: wire::MarkRead| host.agent(&call.session_id)?.mark_read().map_err(internal))
                .await
        }
        wire::RecallQueued::PATH => {
            call(host, body, |host, call: wire::RecallQueued| {
                Ok(host.agent(&call.session_id)?.recall(call.index, &call.text))
            })
            .await
        }
        wire::SetModel::PATH => {
            call(host, body, |host, call: wire::SetModel| {
                host.agent(&call.session_id)?.set_model(host.resolve(call.model)).map_err(failed)
            })
            .await
        }
        wire::GetTree::PATH => {
            call(host, body, |host, call: wire::GetTree| Ok(host.agent(&call.session_id)?.tree())).await
        }
        wire::Checkout::PATH => {
            call(host, body, |host, call: wire::Checkout| {
                host.agent(&call.session_id)?.checkout(call.node).map_err(failed)
            })
            .await
        }
        wire::AppendShell::PATH => call(host, body, |host, call| host.append(call)).await,
        wire::DeleteSessions::PATH => call(host, body, |host, wire::DeleteSessions {}| host.delete_sessions()).await,
        wire::ListModels::PATH => call(host, body, |host, wire::ListModels {}| host.list_models()).await,
        wire::CompleteMention::PATH => call(host, body, |host, call| host.complete(call)).await,
        wire::ResolveMentions::PATH => {
            call(host, body, |_, call: wire::ResolveMentions| {
                absolute(&call.cwd)?;
                let skill =
                    call.skill.and_then(|name| skills::skills(&call.cwd).into_iter().rev().find(|s| s.name == name));
                input::resolve(&call.text, &call.cwd, skill.as_ref()).map_err(failed)
            })
            .await
        }
        wire::ListSkills::PATH => {
            call(host, body, |_, call: wire::ListSkills| {
                absolute(&call.cwd)?;
                Ok(skills::skills(&call.cwd).iter().map(Into::into).collect())
            })
            .await
        }
        wire::ListDirectory::PATH => {
            call(host, body, |_, call: wire::ListDirectory| directory::list(&call.path).map_err(failed)).await
        }
        wire::ReadImage::PATH => {
            call(host, body, |_, call: wire::ReadImage| {
                absolute(&call.path)?;
                image::load(&call.path).map(|bytes| STANDARD.encode(bytes)).map_err(failed)
            })
            .await
        }
        wire::Upload::PATH => {
            call(host, body, |_, call: wire::Upload| upload::save(&call.name, &call.data).map_err(failed)).await
        }
        wire::SetIcon::PATH => {
            call(host, body, |host, call: wire::SetIcon| host.edit(|profile| profile.set_icon(call.icon))).await
        }
        wire::SetDescription::PATH => {
            call(host, body, |host, call: wire::SetDescription| {
                host.edit(|profile| profile.set_description(call.description))
            })
            .await
        }
        wire::SaveSnippet::PATH => call(host, body, |host, call| host.edit(|profile| profile.save_snippet(call))).await,
        wire::DeleteSnippet::PATH => {
            call(host, body, |host, call: wire::DeleteSnippet| host.edit(|profile| profile.delete_snippet(&call.id)))
                .await
        }
        wire::Pair::PATH => call(host, body, |host, call| host.pair(call)).await,
        wire::CreateInvite::PATH => call(host, body, |host, wire::CreateInvite {}| host.invite()).await,
        wire::ListDevices::PATH => call(host, body, |host, wire::ListDevices {}| Ok(host.devices()?.list())).await,
        wire::Unpair::PATH => call(host, body, |host, call| host.unpair(call)).await,
        wire::ListSessions::PATH => call(host, body, |host, wire::ListSessions {}| host.sessions()).await,
        wire::RunSession::PATH => call(host, body, |host, call: wire::RunSession| host.bridge.session(&call.run)).await,
        wire::ViewImage::PATH => {
            call(host, body, |host, call: wire::ViewImage| host.bridge.view_image(&call.run, &call.path)).await
        }
        _ => failure(Failure(StatusCode::NOT_FOUND, format!("No call at {path}"))),
    }
}
