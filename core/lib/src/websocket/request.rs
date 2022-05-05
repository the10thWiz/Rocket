//! Websocket Request state

use std::sync::Arc;

use atomic::Atomic;
use rocket_http::{
    uri::{Host, Origin},
    Accept, ContentType, HeaderMap, Method,
};
use state::{Container, Storage};
use tokio::sync::mpsc;

use crate::{
    cookies::CookieJar,
    request::{ConnectionMeta, Request, RequestState},
    Orbit, Rocket,
};

use super::{message::WebSocketMessage, Channel};

type SubHandle = mpsc::Sender<WebSocketMessage>;

/// Websocket request info
///
/// This type mirrors the Request type, but has some modifications to match the WS protocol
///
/// # Rational
///
/// This type is used to construct ephemeral requests to a websocket. The WsRequest lives as long
/// as the webscoket connection is open, while the ephemeral requests only live as long as an
/// individual message. This prevents the need to create unsafe workarounds in FromData impls, e.g.
/// `serde::Json`. The serde json impl takes advantage of the request's local cache to store the
/// raw string, allowing the deserialized struct to reference it. The only requirement is that the
/// lifetime of the deserialized struct is shorter than the request. However, the local cache
/// design has one major issue for this - only one of these strings can be stored, since that's how
/// the `local_cache!` macro works. For HTTP this isn't an issue - after all, only one data stream
/// (i.e. one json string) is parsed per request. To make sure this assumption holds for WS
/// connections, we construct ephemeral requests which only live as long as a single message.
pub(crate) struct WsRequest<'r> {
    method: Method,
    /// The uri, unless an ephemeral request has it
    uri: Option<Origin<'r>>,
    /// The headers, unless an ephemeral request has it
    headers: Option<HeaderMap<'r>>,
    connection: ConnectionMeta,
    state: WebsocketState<'r>,
    /// The subscribe handle, unless an ephemeral request has it or the handle is not yet
    /// available. Note that ephemeral requests cannot be created until this is set.
    handle: Option<SubHandle>,
}

pub(crate) struct WebsocketState<'r> {
    pub rocket: &'r Rocket<Orbit>,
    /// The cookie jar, unless an ephemeral request has it
    pub cookies: Option<CookieJar<'r>>,
    pub accept: Storage<Option<Accept>>,
    pub content_type: Storage<Option<ContentType>>,
    pub cache: Arc<Container![Send + Sync]>,
    pub host: Option<Host<'r>>,
}

impl<'r> WsRequest<'r> {
    /// Create a new WsRequest by cloning the provided request.
    pub fn new(req: &Request<'r>) -> Self {
        Self {
            method: req.method(),
            uri: Some(req.uri().clone()),
            headers: Some(req.headers().clone()),
            connection: req.connection.clone(),
            state: WebsocketState {
                rocket: req.state.rocket,
                cookies: Some(req.state.cookies.clone()),
                accept: req.state.accept.clone(),
                content_type: req.state.content_type.clone(),
                cache: Arc::clone(&req.state.cache),
                host: req.state.host.clone(),
            },
            handle: None,
        }
    }

    /// Set the internal subscribe handle. This should generally only be called once, as soon as
    /// the subscribe handle is available.
    pub fn set_handle(&mut self, handle: SubHandle) {
        self.handle = Some(handle);
    }

    /// Create an ephemeral request to route an individual message.
    ///
    /// # Panics
    ///
    /// There can only be one ephemeral reqeust at a time, to destroy an ephemeral request, call
    /// `complete_request`. Also panics if a handle has not yet been set.
    pub fn ephemeral_channel(&mut self) -> Channel<'r> {
        // TODO: ideally, this should be able to clone less. This likely requires the Request
        // object to contain more references/Cow type constructs
        Channel::new(
            Request {
                method: Atomic::new(self.method),
                uri: self.uri.take().unwrap(),
                headers: self.headers.take().unwrap(),
                connection: self.connection.clone(),
                state: RequestState {
                    rocket: self.state.rocket,
                    route: Atomic::new(None),
                    cookies: self.state.cookies.take().unwrap(),
                    accept: self.state.accept.clone(),
                    content_type: self.state.content_type.clone(), // TODO avoid clone
                    ws_cache: Some(Arc::clone(&self.state.cache)),
                    cache: Arc::new(<Container![Send + Sync]>::new()),
                    host: self.state.host.take(),
                },
            },
            self.handle.take().unwrap(),
        )
    }

    /// Merge changes made by an ephemeral request back into the websocket state
    ///
    /// This is nessecary, since to avoid cloning various parts of the request, we move them into
    /// the request, and move them back out into here
    pub fn complete_channel(&mut self, req: Channel<'r>) {
        self.uri = Some(req.request.uri);
        self.headers =  Some(req.request.headers);
        self.handle = Some(req.sender);
        self.state.host = req.request.state.host;
        let mut jar = req.request.state.cookies;
        jar.apply_delta();
        self.state.cookies = Some(jar);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn it_works() {}
}
