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
//! doesn't exist. Rocket may also return a 418 error, which should
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
//! If any handler fails, the connection is closed. This has less effect on Leave handlers, since
//! the connection is being closed reguardless, but it can effect the status with which the
//! connection is closed.
//!
//! ### Join
//!
//! Run before any messages have been sent. This is not capable of authentication; see [`token`]
//! for an example of how authentication should be handled in Rocket.
//!
//! There is no data associated with join events
//!
//! For convience, if no Join handler matches, the guards of a message handler will be run. The
//! only effect of this is causing the client to be disconnected faster. It will not catch every
//! reason why the client sending a message might fail, but it will never disconnect a client that
//! would have passed a Message handler.
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
//! TODO: Add WebSocketStatus docs
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
//! routes.
//!
//! However, there is a different trait used for sending messages, which is (TODO)
//! implemented on many of the types that implement Responder.
//!
//! TODO: Document the use of Responder itself
//!
//! ## WebSocket & HTTP routes
//!
//! WebSocket routes use the same default ranks as HTTP routes, and a manual rank can be added in
//! the same way. However, there is a small possiblity of an HTTP route and a WebSocket route
//! having a collision; WebSocket routes also generate an HTTP route (which just returns an
//! UpgradeRequired status code), which has the lowest possible priority.
//!
//! Collisions will be reported if an HTTP Get route has a rank of `isize::max_value()`, and it has
//! the same paths and querys as a WebSocket event handler.
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
//! ## Terminology
//!
//! This is subject to change
//!
//! - Message: individual messages
//! - Channel: sends messages
//! - WebSocket: the connection
//! - Broker: broadcasts messages
//! - Multiplex: Related to the multiplexing protocol
//! - Extension, Protocol, Status: Same as the RFC
//!
//! Therefore, `WebSocketChannel` refers to an object that manages the underlying connection, and
//! can send messages. (It also happens to be pub(crate) since it doesn't get exposed to code
//! outside Rocket).
//!
//! ## Topics
//!
//! Rocket provides a [`Broker`] implementation to facilitate broadcasting to multiple clients. The
//! [`Broker`] identifies groups of clients by topic; the url they used to connect. Note that this
//! includes the query potion of the URL.
//!
//! ## Multiplexing
//!
//! Unimplemented
//!
// Topics, as described in the previous section, have one major drawback: a client needs to create
// a seperate WebSocket Connection (and the underlying TCP Connection) for every topic they wish
// to subscribe to. To reduce the overhead, Rocket implements a Multiplexing protocol, which
// allows a client to connect to multiple topics using a single WebSocket Connection.
//
// The Multiplexing protocol does enforce some limitations, and those are discussed in the full
// protocol description.
//
// There are a number of issues with topic multiplexing. First, handling the request object is
// complicated. My first solution used Arc<Request>, and created a vector of them to hold each of
// the multiplexed connections. This eliminates the security enforced via the Request lifetime, and
// was not a sustainable solution. The current code would allow me to change the URI between
// messages, avoiding many of the above issues. However, this isn't simply a free solution.
//
// Second, using the URI as the topic come's with it's own problems. There is no real way to avoid
// cloning the URI, although it may be possible to avoid some of the clones. If we decide not to use
// the URI as the topic, multiplexing may not require a protocol at all.
//
// Other options:
//
// Using Strings, or a user-defined type. This has advantages and disadvantages. First, the user
// would need to decide what topics the client should be subscribed to. However, this frees Rocket
// from having to figure that out.
//
// Another option is to use some type of matcher to allow the user to select a subset of clients
// based on some arbitraty criteria. This could be a function to match the client via their
// Request, and we would likely create a macro to generate one based on the URI and Request Guards.
//
// I imagine this might look something like
//
// ```rust,no-run
// // Simple URI
// channel!(broker, "/room/lobby")
//
// // handler based -> more or less the same as the URI, but an underscore allows a wildcard for
// // that slot
// channel!(broker, echo(...))
//
// // Total control
// channel!(broker, "/room/{age}", |age: u8| age < 5)
// ```
// The `|` would be optional, if the only requirement is that the FromParam, FromPath and
// FromRequest implementations succeed. In that case, there can be no additional code run.
//
// Syntax: BROKER, MATCHER => where BROKER is a variable refering to the broker
//
// MATCHER: (STRING-LIT, ARGS?) | HANDLER_FN => where STRING-LIT is a literal URIs
//
// HANDLER_FN: HANDLER '(' ARGS,* ')' => where HANDLER is the name of a route, and ARGS is a set of
// args. See the URI! macro for more information
//
// ARGS: ( '|' PARAM,* '|' CODE? ) | PARAM,* => where CODE is a valid Rust expression that
// evalueates to a bool or bool-like object (i.e. Result, Option, or Outcome)
//
// PARAM: NAME ':' TYPE => where NAME is an identifier and TYPE is a Rust type
//
// This would be expanded to something like:
// ```rust,no-run
// {
//    function inner<'r>(request: &'r Request<'_>) -> bool {
//       #path_guards
//       #param_guards
//       #request_guards
//       #user_code|true
//    }
//    InnerChannel(&broker, inner)
// }
// ```
//
// ## Latency & Performance
//
// On my laptop, this isn't a major issue. It seems that the largest performance impact is actually
// client side, and it's not a major concern. This means that Multiplexing is likely unnessecary.
// If we choose this path, and drop connection multiplexing, a greater emphasis should be placed on
// HTTP/2 support.
//
// To make this decision, benchmarking Rocket against other WebSocket servers is a must. There are
// several performance implications to deciding against a multiplexing protocol.
//
// 1. With a protocol, client will establish ~one connection to the server. It should
//    be assumed that the number will be much larger than one without.
// 2. Joining K topics could cause performance issues. As noted above, it appears the largest load
//    is actually placed on the client.
// 3. N clients joining K topics each. This could become a problem. Since Rocket maintains nothing
//    across connections; if a disconnection occurs, it is up to the client to reconnect. The
//    performance impact of this is likely to be similar regaurdless of multiplexing, assuming N
//    >> K. This situation is likely when a server crashes or restarts, since every client will
//    attempt to reconnect at the same time.
//
// ## Massconnect
//
// This benchmark attempts a large number of connections concurrently, and notes the latency.
//
// When benchmarking Rocket agaisnt uWebSockets (a minimal C++ websocket library), a few
// interesting points came to light. First, uWebSockets is marginally faster than Rocket. I suspect
// uWebSockets has an advantage since it doesn't need to handle routing non-websocket traffic.
// Second, uWebSockets seems to have an internal limit to the number of concurrent connections,
// unlike Rocket (Limits can be added if desired). In my testing, Rocket was only limited by the OS
// limits.
//
// ## Rocket vs Phoenix
//
// Rocket has some advantages and disadvantages compared to Phoenix. The Channel concept is based
// on Phoenix's Channels, but has diverged quite a bit since then. Phoenix implements a full
// multiplexing protocol, which uses JSON and a client library. Rocket's proposed multiplexing
// protocol would not require JSON, and it should be simple enough to write a client library if one
// doesn't already exist in the target language. There are a few specific downsides to Phoenix's
// channels: the use of JSON, and the channel descriptors (a string, optionally with a colon
// sperated namespace). Rocket doesn't require JSON, and therefore incures no overhead, and Rocket
// uses the connection's URI as the topic, providing both more flexibility and better type
// checking.
//
// Phoenix also provides a convient broadcast functionality, which is far ahead of Rocket's in terms
// of performance. The current implementation in Rocket has not been optimized in any way yet.
// Phoenix does also provide the ability for a client to directly broadcast a message to a topic,
// but Rocket requires broadcasts originate on the server side. This is a minor disadvantage of
// either the simple multiplexing protocol, or lack of any specific protocl. However, I think this
// is overall actually an advantage, since it requires the user to write a route (with request
// guards) to broadcast a message.
//
// Maximum connection number is another issue. Rocket provides no builtin sharding capabilities,
// unlike Phoenix, so Rocket is unlikely to be able to support millions of concurrent connections.
// Such capabilities could be created, but it would require significatant work, and likely complex
// setup. I suspect the most we will do is provide the ability for an external library to implement
// message sharing. (Likely by allowing direct access to broadcasts from an on_liftoff fairing. It
// would then be responsible for mananging the connections to other servers, and forwarding
// messages).
//
// ### Comparitive performance
//
// To measure latency, I will be using N clients joining K topics. Without multiplexing, this
// creates N*K connections, but with multiplexing it only creates N connections (assuming that the
// server doesn't limit the number of topics a client can subscribe to). For testing, I will be
// using broadcast servers, which provide a room functionality. For Rocket, I will be using the
// websocket-channel example, for Phoenix I will be using a server with similar functionality.
//
//! ## ChannelLocal
//!
//! This is trivially Request's local_cache until multiplexing is added into the mix.
//!
//! Unimplemented
//!
// Rocket already has a mechanism for associating data with a Request, specifically, the Request's
// local cache. Without Multiplexing, Channel local is trivially the same as Request local,
// however they are technically still two seperate caches. With Multiplexing, there is a
// difference: Request local is local to the request, but shared across all of the topics the
// client has subscribed to. Channel local, on the other hand, is local to a specific topic.
//
// ## Virtualizing Requests
//
// To handle multiplexing, requests MUST be virtualized in some way. We can either clone a new
// request for each topic (and potentially run fairings on each), or only swap the URI (and maybe
// cache) for each topic. At the moment, fairings are only run on the initial connection, and the
// request should (and almost certainly must) be cloned if we intend to run fairings on each topic.
//
// The only other issue is Cookies, since only the initial request has any chance to actually store
// cookies. It may be best to find some way to just disable cookies for WebSocket requests. This
// could be handled by adding a Result return value from the add and remove methods, which will
// fail in WebSocket contexts. This won't cause existing applications to stop compiling, but it
// will add warnings for the unused result. Maybe they should still effect the cookie jar? They
// shouldn't be applied (since they never are client side), but they could still be added to the
// pending log.
//
//! ## WebSocketToken
//!
//! `WebSocketToken` is my solution to handling WebSocket authentication. See [`token`] for more
//! info.
//!
//! ## TODO
//!
//! - [ ] Write more documentation
//!   - [ ] Guide
//! - [x] Finalize Data type
//! - [ ] Organize websocket types
//! - [ ] More efficient broker
//! - [x] Event handler panics
//! - [ ] Topic multiplexing
//!   - [ ] Subprotocol Support
//!   - [ ] Fairings
//!   - [ ] Limits
//! - [x] Autobahn CI
//! - [ ] Broadcast topic checking
//!     In theory, it should be possible to add sentinels that check the broadcast URIs, and verify
//!     that they match at least one mounted route. This would likely require the addition of a
//!     `topic!` uri, which just internally calls `uri!`, and wraps the result into a `Topic` type.
//!     The codegen would then check the function body for the `topic!` macro, and emit a sentinel.
//! - [ ] ChannelLocal
//! - [x] Close data
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
//! other frames.
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
pub(crate) mod request;

pub mod broker;
pub mod token;

pub use channel::Channel;
pub use status::WebSocketStatus;

use crate::{Data, Request};
use crate::http::hyper;
use crate::response::Builder;

use self::status::WebSocketStatusResult;

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
pub(crate) fn upgrade(req: &mut hyper::Request<hyper::Body>) -> Option<(String, OnUpgrade)> {
    if req.method() == hyper::Method::GET {
        ClientRequest::parse(|n|
            req.headers().get(n).map(|s| s.to_str().unwrap_or(""))
        ).ok().map(|accept| (accept.ws_accept(), hyper::upgrade::on(req)))
    } else {
        None
    }
}

/// A Data wrapper for WebSockets
pub enum WebSocketData<'a> {
    /// For the Join events, there is no data associated
    Join,
    /// Message, just wraps Data
    Message(Data<'a>),
    /// Leave, wraps a WebSocketStatusResult
    Leave(WebSocketStatusResult<'a>),
}

impl<'a> From<Data<'a>> for WebSocketData<'a> {
    fn from(data: Data<'a>) -> Self {
        Self::Message(data)
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

    /// Gets the set of allowed reserved bits for this extension
    fn allowed_rsv_bits(&self) -> u8 {
        match self {
            _ => 0u8,
        }
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
    pub fn new(req: &Request<'_>) -> Self {
        req.headers()
            .get("Sec-WebSocket-Protocol")
            .flat_map(|s| s.split(','))
            .find_map(|s| match s.trim() {
                _ => None,
            })
            .unwrap_or(Self::Naked)
    }

    /// Gets the name to set for the WebSocket Protocol header
    ///
    /// Note that this MUST return exactly the same as one of the protocol headers sent by the
    /// client.
    pub fn get_name(&self) -> Option<String> {
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

#[allow(unused)]
pub mod rocket_multiplex {
    //! # Full rocket-multiplex protocol description:
    //!
    //! Rocket uses the Origin URL of a WebSocket request as a topic identifier. The rocket-multiplex
    //! proprotocol allows sending messages to multiple topics using a single WebSocket connection.
    //!
    //! # Design considerations
    //!
    //! - [ ] Simple
    //! - [ ] RFC compliant
    //! - [ ] Transparent
    //!
    //! One major goal is to make this protocol transparent: on Rocket's side, user code cannot
    //! tell whether the connection is multiplexed or not. There may be some differences, such as
    //! a multiplexed connection only counting towards a connection limit as a single connection.
    //! It should also be possible to design client libraries that are transparent, where user code
    //! can't tell whether the connections are being multiplexed or not.
    //!
    //! # Handshake
    //!
    //! rocket-multiplex acts as a subprotocol for WebSocket connections, like json or xml might
    //! be. Most WebSocket libraries should provide functionality to request a subprotocol, and
    //! should therefore be able to complete the handshake. Use `'rocket-multiplex'`, case
    //! insesitively.
    //!
    //! ## Subprotocols
    //!
    //! As currently defined, there is no way to specify another subprotocol in addition to
    //! rocket-multiplex.
    //!
    //! One option is to specify `rocket-multiplex-*`, where `*` is a subprotocol, and perhaps add
    //! it to the subscribe action. This would require updating and improving the error responses,
    //! which should probably be done anyway.
    //!
    //! # Messages: Data & Control
    //!
    //! Notes:
    //!
    //! This protocol doesn't specify how the messages should be transimitted, and Rocket allows
    //! anything the RFC considers legal. For example, there is no requirement that the control
    //! portions of the message are sent as a single frame.
    //!
    //! ### Seperator character: `'\u{00B7}'`
    //!
    //! This character was chosen because it is not a valid character in a URL (according to the
    //! spec, if MUST be percent encoded), and it is valid UTF-8, i.e. it can be included in a Text
    //! message.
    //!
    //! ## Data
    //!
    //! Data messages start with the topic URL they should be sent to, followed by `'\u{00B7}'`.
    //! This is followed by the contents of the message. The length of the message is not limited by
    //! this protocol, and the URL is not counted towards the length limit of the message.
    //!
    //! Data messages sent to a topic the client is not subscribed to result in an error being sent to
    //! the client.
    //!
    //! Topic URLS are limited to `MAX_TOPIC_LENGTH = 100` (in bytes), to prevent potential DoS attacks.
    //!
    //! # Control
    //!
    //! Control messages are limited to 512 bytes total, although there should never be a reason to
    //! create a longer message. Control messages must be marked as text and take the form:
    //!
    //! `S ACTION S (PARAM S)*`
    //!
    //! Where `S` = `\u{00B7}`, `ACTION` is one of the following actions, and `PARAM` is one of the
    //! positional paramaters associated with the aciton.
    //!
    //! # Actions
    //!
    //! - Subscribe: `SUBSCRIBE`, [Topic]; subscribed the client to a specific topic URL, as if the
    //! client had opened a second WebSocket connection to the topic URL
    //! - Unsubscribe: `UNSUBSCRIBE`, [TOPIC, CODE?, REASON?]; unsubscribes the client from a specific
    //! topic URL, as if the client has closed the second WebSocket connection to the topic URL
    //! - Unsubscribe all: There is no specific unsubscribe all action, although closing the WebSocket
    //! connection is treated as an unsubscribe all
    //!
    //! - Ok: `OK`, [ACTION, PARAMS*]; Sent as a response to an action, this indicates that the action
    //! succeeded.
    //! - Err: `ERR`, [REASON, ACTION, PARAMS*]; Sent as a response to an action, this indicates that
    //! the action failed. REASON is case-insesitive.
    //! - Invalid message: `INVALID`, [REASON]; Sent as a response to an message the client is not
    //! allowed to send. Currently, this is only sent in response to a message to a topic the client is
    //! not subscribed to.
    //!
    //! ## Defined errors
    //!
    //! The following errors are defined as part of this protocol. The server implementation can
    //! return errors not on this list, but should prefer to return one of the errors on this list
    //! if applicable.
    //!
    //! ### Subscribe errors
    //!
    //! - Too many topics: indicates that the client has subscribed to too many topics on this
    //! connection. The limit is not defined by this spec, and no guarentees are made about the
    //! server's behaviour when returning this error. For example, the server is allowed to change
    //! the limit on the fly, or anything else. The server isn't allowed to drop existing
    //! subscriptions, except by termintating the connection as a whole.
    //!
    //! - Not found: Inicates that the requested topic doesn't exist. This is equavalent to the
    //! server returning a 404 during the opening handshake.
    //!
    //! ### Unsubscribe errors
    //!
    //! - Not subscribed: Inicates that the client was not subscribed to this topic, and therefore
    //! cannot be unsubscribed.
    //!
    //! ### WebSocket errors
    //!
    //! When a WebSocket connection encounters an error defined by the RFC, the server is permitted
    //! to close the connection with the error
    //!
    //! # Limits
    //!
    //! As noted above, this protocol does not specify many limits. Implementations are free to
    //! add limits as needed, and should try to return inforative errors when possible.
    //!
    //! # Stability & Recovery
    //!
    //! This spec makes no guarentees about the stability of the connection. There is also no
    //! specific mechanism to resubscribe clients to topics they were subscribed to before they
    //! were disconnected. Rather, clients are responsible for resubscribing to topics they wish to
    //! resubscribe to.
    //!
    //! There is also no builtin mechanism for message persistance, this should be implemented
    //! seperatly. This can take the form of a HTTP route to get previous messages.
    //!
    //! # Alternatives
    //!
    //! There are a few options for alternatives to the protocol described above. For example, we
    //! could provide a similar functionality, with some restrictions, by requiring the rust
    //! handler to subscribe the client to the topics they desire. The client would then be
    //! un-aware of the multiplexing, and therefore unable to tell which topic a message was sent
    //! on. In many situtations, this is an acceptable alternative, since not all clients require
    //! the ability to identify indivitual channels, and the user could choose to add a topic
    //! marker to each message if the client does need to know.
    //!
    //! This also makes implementing access control more obvious to implement, but may have the
    //! drawback that access control is therefore easier to get wrong.
    //!
    //! I think it may also be acceptable to release websocket support without a multiplexing
    //! protocol at all. Once we choose, I don't think should can go back, so I want to get this
    //! right.

    use std::{str::Utf8Error, string::FromUtf8Error, sync::Arc};

    use bytes::Bytes;
    use rocket_http::{ext::IntoOwned, uri::{Origin, Error}};
    use state::Container;
    use ubyte::ByteUnit;

    use crate::Data;

    /// Maximum length of topic URLs, with the possible exception of the original URL used to connect.
    ///
    /// TODO: investigate the exception, and potentially handle it
    pub const MAX_TOPIC_LENGTH: usize = 100;

    /// Control character for seperating information in 'rocket-mutltiplex'
    ///
    /// U+00B7 (MIDDLE DOT) is a printable, valid UTF8 character, but it is never valid within a URL.
    /// To include it, or any other invalid character in a URL, it must be percent-encoded. This means
    /// there is no ambiguity between a URL containing this character, and a URL terminated by this
    /// character
    pub const MULTIPLEX_CONTROL_STR: &'static str = "\u{B7}";
    /// `MULTIPLEX_CONTROL_STR`, but as a `char`
    pub const MULTIPLEX_CONTROL_CHAR: char = '\u{B7}';
    /// `MULTIPLEX_CONTROL_STR`, but as a `&'static [u8]`
    pub const MULTIPLEX_CONTROL_BYTES: &'static [u8] = MULTIPLEX_CONTROL_STR.as_bytes();

    /// Errors associated with decoding and encoding multiplex messages
    pub(crate) enum MultriplexError {
        NotSubscribed,
        AlreadySubscribed,
        TooManyTopics,
        NotFound,
        NoControlChar,
        InvalidUtf8,
        InvalidTopic,
        ControlMessageToLong,
        InvalidControlMessage,
        IoError,
    }

    const INVALID: &'static str = "\u{B7}INVALID\u{B7}";
    const ERR: &'static str = "\u{B7}ERR\u{B7}";

    impl MultriplexError {
        pub fn to_bytes(self) -> Bytes {
            Bytes::from(match self {
                Self::NoControlChar =>
                    format!("{}{}", INVALID, "No Control Character"),
                Self::InvalidUtf8 =>
                    format!("{}{}", INVALID, "Invalid Utf-8"),
                Self::InvalidTopic =>
                    format!("{}{}", INVALID, "Invalid Topic"),
                Self::ControlMessageToLong =>
                    format!("{}{}", INVALID, "Control message exceeds 512 byte limit"),
                Self::InvalidControlMessage =>
                    format!("{}{}", INVALID, "Control Message Invalid"),
                Self::IoError =>
                    format!("{}{}", INVALID, "Io Error"),
                Self::NotSubscribed =>
                    format!("{}{}", ERR, "Not Subscribed"),
                Self::AlreadySubscribed =>
                    format!("{}{}", ERR, "Already Subscribed"),
                Self::TooManyTopics =>
                    format!("{}{}", ERR, "Too Many Topics"),
                Self::NotFound =>
                    format!("{}{}", ERR, "Not Found"),
            })
        }
    }

    impl From<Utf8Error> for MultriplexError {
        fn from(_: Utf8Error) -> Self {
            Self::InvalidUtf8
        }
    }

    impl From<FromUtf8Error> for MultriplexError {
        fn from(_: FromUtf8Error) -> Self {
            Self::InvalidUtf8
        }
    }

    impl<'a> From<Error<'a>> for MultriplexError {
        fn from(_: Error<'a>) -> Self {
            Self::InvalidTopic
        }
    }

    impl From<std::io::Error> for MultriplexError {
        fn from(_: std::io::Error) -> Self {
            Self::IoError
        }
    }

    pub(crate) enum Action<'s, 'r> {
        Subscribed(&'s Origin<'static>),
        Unsubscribed(Origin<'static>),
        Join(&'s Origin<'static>, &'s Arc<Container![Send + Sync]>, Data<'r>),
        Message(&'s Origin<'static>, &'s Arc<Container![Send + Sync]>, Data<'r>),
        Leave(&'s Origin<'static>, &'s Arc<Container![Send + Sync]>, Data<'r>),
    }

    /// Holds the data associated with a Multiplexed connection. Right now, thats an Origin & Cache
    /// for each topic.
    pub(crate) struct MultiplexTopics(Vec<(Origin<'static>, Arc<Container![Send + Sync]>, bool)>);

    impl MultiplexTopics {
        /// Create a MultiplexTopics with the initial topic
        pub fn new(initial: &Origin<'_>) -> Self {
            Self(vec![(initial.clone().into_owned(), Self::new_cache(), false)])
        }

        pub async fn handle_message<'s, 'r>(
            &'s mut self,
            mut data: Data<'r>
        ) -> Result<Action<'s, 'r>, MultriplexError> {
            let tmp = data.peek(MAX_TOPIC_LENGTH + MULTIPLEX_CONTROL_BYTES.len()).await;
            if let Some(i) = unsafe {
                std::str::from_utf8_unchecked(tmp).find(MULTIPLEX_CONTROL_STR)
            } {
                if i != 0 {
                    let topic = Origin::parse(std::str::from_utf8(&tmp[..i])?)?;
                    if let Some((topic, cache, joined)) =
                        self.0.iter_mut().find(|(t, _, _)| t == &topic)
                    {
                        data.take_start(i + MULTIPLEX_CONTROL_BYTES.len()).await;
                        if *joined {
                            Ok(Action::Message(topic, cache, data))
                        } else {
                            *joined = true;
                            Ok(Action::Join(topic, cache, data))
                        }
                    } else {
                        Err(MultriplexError::NotSubscribed)
                    }
                } else {
                    self.handle_control(data).await
                }
            } else {
                Err(MultriplexError::NoControlChar)
            }
        }

        async fn handle_control<'s, 'r>(
            &'s mut self,
            data: Data<'r>
        ) -> Result<Action<'s, 'r>, MultriplexError> {
            let capped = data.open(ByteUnit::Byte(512)).into_bytes().await?;
            if !capped.is_complete() {
                return Err(MultriplexError::ControlMessageToLong)
            }
            let raw = String::from_utf8(capped.into_inner())?;
            let mut parts = raw.split(MULTIPLEX_CONTROL_STR);
            match parts.next() {
                Some("SUBSCRIBE") => match parts.next() {
                    Some(s) => {
                        let new_topic = Origin::parse(s)?.into_owned();
                        if !self.0.iter().any(|(t, _, _)| t == &new_topic) {
                            self.0.push((new_topic, Self::new_cache(), false));
                            Ok(Action::Subscribed(&self.0[self.0.len() - 1].0))
                        } else {
                            Err(MultriplexError::AlreadySubscribed)
                        }
                    }
                    None => Err(MultriplexError::InvalidControlMessage),
                },
                Some("UNSUBSCRIBE") => match parts.next() {
                    Some(s) => {
                        let old_topic = Origin::parse(s)?.into_owned();
                        match self.0.iter().position(|(t, _, _)| t == &old_topic) {
                            Some(i) => {
                                self.0.remove(i);
                                Ok(Action::Unsubscribed(old_topic))
                            }
                            None => Err(MultriplexError::NotSubscribed)
                        }
                    }
                    None => Err(MultriplexError::InvalidControlMessage),
                },
                _ => Err(MultriplexError::InvalidControlMessage),
            }
        }

        fn new_cache() -> Arc<Container![Send + Sync]> {
            Arc::new(<Container![Send + Sync]>::new())
        }
    }
}
