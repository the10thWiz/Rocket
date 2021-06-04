//! Rocket Websocket implementation
//! 
//! Websocket Routes are very similar to normal routes, and have many of the same properties.
//!
//! A simple echo handler:
//! ```ignore
//! #[message("/echo", "<data>")]
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
//! # Channel Guards
//!
//! TODO: implement websocket guards
//!
//! # Data
//!
//! Websocket messages will automatically be converted into any type that implements `FromData`,
//! which means pre-existing types like `Json`, or `&str` just work as expected.
//!
//! Send messages is slightly more complicated. In order to facilitate the use of existing
//! websocket libraries, Rocket needs to know whether a given message is Text or Binary. To this
//! end, Rocket has the `IntoMessage` trait. TODO: At the moment, this is only implemented for Data
//! and types that implement `AsyncRead`.
//!
//! # Interactions with other requests
//!
//! Websocket Routes automatically define a corresponding route for non-websocket requests. This
//! route is given a rank of 100, so it should almost never collide with non-websocket routes, and
//! returns an `UpgradeRequired` status.
//!
//! # Limits
//!
//! This websocket implementation doesn't define any size limits of it's own, other than a soft
//! maximum on the size of individual chunks when reading. Instead, size limits are handled by
//! the types that implement `FromData`.
// Autobahn testing
//
// 2.9 fails to send pong for ping
// 5.19/5.20 sometimes fail to send pongs & message in the correct order - this is likely a data
//   race between reading and my reply - I start replying right away, before I've finished reading
//   the frame
// 6.* fail since I don't check UTF-8 - should I?
// 7.1.5 fails since Rocket will forward part of a message back. This isn't an issue in most cases
//   (especially JSON or similar), where the entire message would need to be buffered before we can
//   start responding.
// 7.13.* we need to decide on Rocket's behaviour, this isn't defined in the spec
//
// 12.* & 13.* test compression, which we don't implement (yet)

mod router;
mod message;
mod broker;
mod channel;
mod status;
mod websocket;

pub(crate) use router::WebsocketRouter;
pub(crate) use message::to_message;
pub(crate) use broker::Broker;

pub use channel::{Channel, WebsocketChannel};
pub use message::{WebsocketMessage, IntoMessage};
pub use status::*;
pub use websocket::{FromWebsocket, Websocket};

pub use websocket_codec::Opcode;

/// Soft maximum buffer size
pub const MAX_BUFFER_SIZE: usize = 1024;
