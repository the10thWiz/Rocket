//! # Rocket WebSocket protocol [(RFC6455)](https://datatracker.ietf.org/doc/html/rfc6455)
//! # implementation
//!
//! Rocket uses an event-based WebSocket implementation, with the goal of making WebSocket support
//! easy to add and maintain.
//!
//! ## A simple example
//!
//! A simple echo handler
//!
//! ```rust
//! # #[macro_use] extern crate rocket;
//! # use rocket::channels::Channel;
//! # use rocket::Data;
//! #[message("/echo", data = "<data>")]
//! async fn echo(data: Data, websocket: Channel<'_>) {
//!     websocket.send(data).await;
//! }
//! ```
//!
//! WebSockets use the HTTP Upgrade mechanism to create a connection. This is entirely handled by
//! Rocket. Rocket will return a 404 error if the client attempts to connect to a route that
//! doesn't exist (TODO: maybe run some guards?). Rocket may also return a 418 error, which should
//! be reported as a bug within Rocket itself.
//!
//! After the client establishes a WebSocket connection, Rocket will wait for messages to be sent
//! by the client. Each message is passed to an event handler. If the event handler fails, the
//! connection is closed, with the provided [`WebSocketStatus`]. If the event handler forwards,
//! Rocket will take specific actions, noted below.
//!
//! Note that Rocket automatically handles Ping and Pong frames, and does not provide any mechanism
//! to initiate a Ping. It is assumed that the client is responsible for keeping the connection
//! alive as long as they wish to be connected.
//!
//! ## WebSocket events:
//!
//! ### Join
//!
//! Run on the first message a client sends. If this forwards, the message is then treated as a
//! normal message.
//!
//! Note that the Join handler doesn't have to succeed. To protect against this, see ChannelLocal
//! below.
//!
//! ### Message
//!
//! Runs on every message the client sends, after the Join handler. Forwards are treated as
//! Failures.
//!
//! ### Leave
//!
//! Run after the client disconnects, regaurdless of which endpoint initiated the close. Forwards
//! and Failures are ignored, since the connection is already disconnected.
//!
//! This event doesn't have any data associated with it right now. This is also implemented as an
//! empty data. TODO: make this make sense.
//!
//! ## WebSocket Guards
//!
//! WebSocket Guards are very similar to Request Guards with one important difference: WebSocket
//! Guards can only be used in WebSocket event handlers. However, Request Guards can be used in
//! WebSocket event handlers as normal.
//!
//! ## Data Guards
//!
//! WebSockets do not have special data guards, rather they use the same data guards as HTTP
//! routes. However, there is a different trait used for sending messages, which is (TODO)
//! implemented on many of the types that implement Responder.
//!
//! ## WebSocket & HTTP routes
//!
//! WebSocket routes use the same default ranks as HTTP routes, and a manual rank can be added in
//! the same way. However, there is a small possiblity of an HTTP route and a WebSocket route
//! having a collision; WebSocket routes also generate an HTTP route (which just returns an
//! UpgradeRequired status code), which has the lowest possible priority.
//!
//! Collisions will be reported if an HTTP Get route has a rank of `isize::max_value()`, and it has the
//! same paths and querys as a WebSocket event handler.
//!
//! Technically, this allows for collisions between the HTTP handlers for two WebSocket routes. The
//! order in which they are tried is unspecified, however this is acceptable since they should have
//! the same implementation.
//!
//! ## Limits
//!
//! Rocket currently does not enforce any limits specific to WebSockets. Since Data is handled
//! using the same Data Guards as HTTP routes, the same limits are enforced. Rocket does not
//! currently limit the number of concurrent connections, although this is likely limited by the
//! underlying OS or the network.
//!
//! There are a few other limitations: Rocket cannot start processing another message from a client
//! until the current event handler completes (this is related to the Request lifetimes).
//!
//! ## Topics
//!
//! Rocket provides a [`Broker`] implementation to facilitate broadcasting to multiple clients. The
//! [`Broker`] identifies groups of clients by topic; the url they used to connect. Note that this
//! includes the query potion of the URL.
//!
//! ## Multiplexing
//!
//! Unimplemented(TODO)
//!
//! Topics, as described in the previous section, have one major drawback: a client needs to create
//! a seperate WebSocket Connection (and the underlying TCP Connection) for every topic they wish
//! to subscribe to. To reduce the overhead, Rocket implements a Multiplexing protocol, which
//! allows a client to connect to multiple topics using a single WebSocket Connection.
//!
//! The Multiplexing protocol does enforce some limitations, and those are discussed in the full
//! protocol description.
//!
//! ## ChannelLocal
//!
//! Unimplemented
//!
//! Rocket already has a mechanism for associating data with a Request, specifically, the Request's
//! local cache. Without Multiplexing, Channel local is trivially the same as Request local,
//! however they are technically still two seperate caches. With Multiplexing, there is a
//! difference: Request local is local to the request, but shared across all of the topics the
//! client has subscribed to. Channel local, on the other hand, is local to a specific topic.
//!
//! ## TODO
//!
//! - [ ] Write more documentation
//!   - [ ] Guide
//! - [ ] Finalize Data type
//! - [ ] Organize websocket types
//! - [ ] More efficient broker
//! - [ ] Event handler panics
//! - [ ] Topic multiplexing
//!   - [ ] Subprotocol Support
//!   - [ ] Fairings
//!   - [ ] Limits
//! - [ ] Autobahn CI
//! - [ ] Broadcast topic checking
//!     In theory, it should be possible to add sentinels that check the broadcast URIs, and verify
//!     that they match at least one mounted route. This would likely require the addition of a
//!     `topic!` uri, which just internally calls `uri!`, and wraps the result into a `Topic` type.
//!     The codegen would then check the function body for the `topic!` macro, and emit a sentinel.
//! - [ ] ChannelLocal
//! - [ ] Close data
//!
//! ### Later
//!
//! These don't need to be completed before this PR can be considered ready.
//!
//! - [ ] Compression
//! - [ ] HTTP/2
//!
//! ## Autobahn testing
//!
//! This implementation doesn't perfectly pass the full autobahn test suite:
//!
//! - Non-strict: This implementation passes many of the tests non-strictly. In general, this is
//! because this implementation fails very fast, and a close frame can (and often does) pre-empt
//! any other frames.
//! - 5.19 & 5.20: This implementation intermittently fails these two tests. The issue is that
//! sometimes the second pong is sent after the final frame has been completed. However, looking at
//! most other implementations I've tested, it appears that they pass partially because they buffer
//! the entire message. It's also likely that they have combined the read & write tast into a
//! single task (or thread), so they don't have any races of the type my implementation has.
//! - 12 & 13: These test compression, which we don't implement. (Yet)

use rocket_http::Header;
use rocket_http::hyper::upgrade::OnUpgrade;
use rocket_http::uri::Origin;
use websocket_codec::ClientRequest;

pub(crate) mod channel;
pub(crate) mod message;
pub(crate) mod status;
pub(crate) mod broker;

pub use channel::{WebSocket, Channel};
pub use status::WebSocketStatus;

use crate::Request;
use crate::http::hyper;
use crate::response::Builder;

/// Identifier of WebSocketEvent
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum WebSocketEvent {
    /// A Join event, triggered when a client connects
    Join,
    /// A Message event, triggered when a client sends a message
    Message,
    /// A Leave event, triggered when a client disconnects
    Leave,
}

impl WebSocketEvent {
    pub(crate) fn from_handler<T>(h: &crate::route::WebSocketEvent<T>) -> Option<Self> {
        match h {
            crate::route::WebSocketEvent::Join(_) => Some(Self::Join),
            crate::route::WebSocketEvent::Message(_) => Some(Self::Message),
            crate::route::WebSocketEvent::Leave(_) => Some(Self::Leave),
            crate::route::WebSocketEvent::None => None,
        }
    }
}

/// Create the upgrade object associated with this request IF the request should be upgraded
pub(crate) fn upgrade(req: &mut hyper::Request<hyper::Body>) -> Option<WebsocketUpgrade> {
    if req.method() == hyper::Method::GET {
        ClientRequest::parse(|n|
            req.headers().get(n).map(|s| s.to_str().unwrap_or(""))
        ).ok().map(|accept| WebsocketUpgrade {
            accept: accept.ws_accept(), on_upgrade: hyper::upgrade::on(req)
        })
    } else {
        None
    }
}

/// The extensions and protocol for a websocket connection
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Extensions {
    protocol: Protocol,
    extensions: Vec<Extension>,
}

impl Extensions {
    /// Select a protocol and extensions for the connection from a request
    pub fn new(req: &Request<'_>) -> Self {
        Self {
            protocol: Protocol::new(req),
            extensions: vec![],
        }
    }

    /// Gets the list of headers to describe the extensions and protocol
    pub fn headers(&self, response: &mut Builder<'_>) {
        for header in self.extensions.iter().flat_map(|e| e.header().into_iter()) {
            response.header_adjoin(header);
        }
        for header in self.protocol.header().into_iter() {
            response.header(header);
        }
    }

    pub fn protocol(&self) -> Protocol {
        self.protocol
    }

    pub fn allowed_rsv_bits(&self) -> u8 {
        let mut ret = 0u8;
        for extension in self.extensions.iter() {
            ret |= extension.allowed_rsv_bits()
        }
        ret
    }
}

/// An individual WebSocket Extension
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Extension {
}

impl Extension {
    /// Gets the header valus to enable this extension
    fn header(&self) -> Option<Header<'static>> {
        match self {
            _ => None,
        }
    }

    fn allowed_rsv_bits(&self) -> u8 {
        0u8
    }
}

/// A WebSocket Protocol. This lists every websocket protocol known to Rocket
#[allow(unused)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Protocol {
    Multiplex,
    Naked,
}

impl Protocol {
    pub fn new(_req: &Request<'_>) -> Self {
        Self::Naked
    }

    /// Gets the name to set for the WebSocket Protocol header
    pub fn get_name(&self) -> Option<&'static str> {
        match self {
            _ => None,
        }
    }

    /// Gets the header to identify this protocol
    pub fn header(&self) -> Option<Header<'static>> {
        self.get_name().map(|n| Header::new("Sec-WebSocket-Protocol", n))
    }

    pub fn with_topic<'r>(&self, origin: &Origin<'r>) -> Option<Origin<'r>> {
        match self {
            Self::Naked => None,
            Self::Multiplex => Some(origin.clone()),
        }
    }
}

/// Everything needed to desribe a websocket Upgrade
/// TODO: Maybe don't use this? I think the only thing I do is split it up right away
pub(crate) struct WebsocketUpgrade {
    accept: String,
    on_upgrade: OnUpgrade,
}

impl WebsocketUpgrade {
    /// ?
    pub fn split(self) -> (String, OnUpgrade) {
        (self.accept, self.on_upgrade)
    }
}
