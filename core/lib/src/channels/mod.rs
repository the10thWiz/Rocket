//! Rocket WebSocket implementation
//!
//! WebSocket Routes are very similar to normal routes, and have many of the same properties.
//!
//! A simple echo handler:
//! ```rust
//! # #[macro_use] extern crate rocket;
//! # use rocket::channels::Channel;
//! # use rocket::Data;
//! #[message("/echo", data = "<data>")]
//! async fn echo(data: Data, websocket: Channel<'_>) {
//!     websocket.send(data).await;
//! }
//! ```
//! TODO: potenially change the data attribute to require `data = `
//!
//! The only required handler is the `message` handler, which is called for each incomming message.
//! There are also `join` and `leave` handlers, which are called when a client initially connects.
//!
//! # Join and Leave events
//!
//! It is important to note that the Join and Leave events are not required. However, Rocket will
//! reject a connection if a Join handler fails. This might be slightly unintuitive, but any
//! important properties of the connection should be checked by request guards in the message
//! handler.
//!
//! Leave will always be triggered when a connection ends, regardless of whether the client or
//! server initiated the close.
//!
//! # Channel Guards
//!
//! Channel Guards are *slightly* different from Request Guards. Every Request Guard is a
//! Channel Guard, but not every Channel Guard is a Request Guard. This means that any Request
//! Guard you've been using will just work as a Channel Guard. However, there are Channel Guards
//! that cannot be Request Guards, such as the `Channel` object. This distinction is primarily
//! useful for the `Channel` object, since it makes including it in a non-WebSocket route a
//! compiler error.
//!
//! # Data
//!
//! WebSocket messages will automatically be converted into any type that implements `FromData`,
//! which means pre-existing types like `Json`, or `&str` just work as expected.
//!
//! Send messages is slightly more complicated. In order to facilitate the use of existing
//! WebSocket libraries, Rocket needs to know whether a given message is Text or Binary. To this
//! end, Rocket has the `IntoMessage` trait.
//!
// TODO: initially, this is only implemented for Data and types that implement `AsyncRead`.
// The current implementation doesn't have AsyncRead, since I don't think it's a good idea, but
// rather I'm providing as many types as possible, including String, Vec, Json, MsgPack, etc.
//! This is implmeneted on a number of types, including Json, MsgPack, String, Vec, etc.
//!
//! # Interactions with other requests
//!
//! WebSocket Routes automatically define a corresponding route for non-WebSocket requests. This
//! route is given a rank of 100, so it should almost never collide with non-WebSocket routes, and
//! returns an `UpgradeRequired` status.
//!
//! # Limits
//!
//! This WebSocket implementation doesn't define any size limits of it's own, other than a soft
//! maximum on the size of individual chunks when reading. Instead, size limits are handled by
//! the types that implement `FromData`.
//!
//! # Topics & Broker
//!
//! Rocket has an Broker implementation that allows sending messages to multiple clients at the
//! same time. To identify groups of clients, Rockets uses the Origin Uri the client connected to.
//! For example, if a client connects to `ws://example.com/main/lobby`, it would be routed to a
//! message handler for `/main/lobby`, and Rocket would use `/main/lobby` as the topic the client
//! is subscribed to. Messages can be broadcast to a topic via the `Channel` object, or the `Broker`
//! object. Outside of a WebSocket handler, you have to use the `Broker`, which can be obtained
//! from the Rocket instance.
//!
//! The `Channel::broadcast` method will broadcast the message to the current topic, i.e. the one
//! the client is subscribed to. `Channel::broadcast_to` & `Broker::broadcast_to` will broadcast to
//! a specified topic. The `uri!` macro is the prefered way to create Origin Uris (Topics), but it
//! is also possible to parse Uris on the fly if needed. Refer to
//! [`Origin`](crate::http::uri::Origin) for more information.
//!
//! # TODO for potential improvements
//!
//! - [ ] Better panic handling: At the moment, nothing is reported to the client, and the
//! connection continues as normal
//! - [x] Identify potential panics in Rocket
//! - [ ] Full Autobahn compliance: Potential exceptions are possible, since Rocket is very async
//! & very fast
//! - [x] Topic multiplexing; rocket-multiplex
//!     - [ ] Testing
//!     - [ ] Subprotocols
//!     - [ ] Topic length limit on initial connect
//! - [-] Rocket specific Opcode type, since Rocket handles many of them already
//!     The `websocket-codec` Opcode type is no longer re-exported, since it was only used
//!     internally
//! - [ ] Full examples for the guide
//! - [x] Broker improvements
//! - [x] Finalize data portion of the attribute macros
//! - [ ] Finalize the `IntoMessage` implementations
//! - [ ] Subprotocol support. Depends on rocket-multiplex support for subprotocols
//!
//! - [x] `data = `
//! - [x] async `broadcast`s
//! - [x] Require message channel guards to pass
//!
//! ## Nice to haves
//!
//! - [x] Check for vaid UTF-8 in Text messages
//! - [ ] ChannelLocal (probably just RequestLocal)
//! - [ ] Fairings for Rocket-multiplex
//!
//! ## Implementation details
//!
//! These are things that can be added in a minor patch since they don't cause breaking changes
//!
//! - [ ] Compression
//! - [ ] HTTP/2 WebSockets
//!
//! # Autobahn testing
//!
//! There are a number of tests where Rocket fails because of the nature of it's design. Rocket is
//! async, and doesn't need to buffer the entire message in memory before it can start sending it
//! back. This means that pongs often don't arrive in the order Autobahn expects, and Rocket can
//! start echoing a message before it has finished recieving it.
//!
//! - [x] (5.19, 5.20) sometimes fail to send pongs & message in the correct order - this is likely a data
//!   race between reading and replying - it starts replying right away, before it's finished reading
//!   the frame
//!     Interestingly, the `Case Expectation` provided by Autobahn seems to indicate that our
//!     behaviour is acceptable. Case 5.20 notes that 5.19 and 5.20 should behave the same, but
//!     this is not always the case. Based on some peliminary testing, is looks like 5.20 always
//!     fails, but 5.19 occasionally passes. The only difference between the two cases is that 5.20
//!     splits the messages up and sends them byte by byte.
//!     Curiously, there is one line in channel.rs which effects these tests. When the timeout is
//!     zero, 5.20 occasionally fails, when it is greater than zero, 5.19 occasionally fails.
//!     Increeasing the timeout to 10+ms causes both to consitently fail. I don't think we can
//!     expect consistent behaviour out of these test.
//! - [x] (7.1.5) fails since Rocket will forward part of a message back. This is because Rocket will
//!     consider the frame to have ended, despite the fact that it didn't. I don't think there is a
//!     way to handle this correctly using the current model, since the server side handler is
//!     already running by the time the error happens.
//!     Can we get away with just not sending a fin frame? Yes! I added an atomic bool that tracks
//!     whether we have recieved a close frame from the client, and just don't send any more frames
//!     other than close frames.
//!
//! - [x] (6.\*) fail since the echo server doesn't check UTF-8
//! - [ ] (12.\*, 13.\*) test compression, which we don't implement (yet)
//!
//! There are also a number of informational tests, which don't have correct behaviour specified by
//! the RFC, but we should look at them and decide whether our current behaviour is good enough

mod router;
mod message;
mod broker;
mod channel;
mod websocket;

pub(crate) use router::*;
pub(crate) use message::to_message;
pub(crate) use channel::WebSocketChannel;

pub mod status;

pub use channel::Channel;
pub use message::{WebSocketMessage, IntoMessage, into_message};
pub use broker::Broker;
//pub use status::*;
pub use websocket::{FromWebSocket, WebSocket};

/// Soft maximum buffer size
pub const MAX_BUFFER_SIZE: usize = 1024;

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
    /// `MULTIPLEX_CONTROL_STR`, but as a `&'static [u8]`
    pub const MULTIPLEX_CONTROL_CHAR: &'static [u8] = MULTIPLEX_CONTROL_STR.as_bytes();
}
