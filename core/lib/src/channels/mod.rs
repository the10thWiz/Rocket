//! Rocket Websocket implementation
//!
//! TODO: add actual docs & example code

mod router;
mod message;
mod broker;
mod channel;

pub(crate) use router::WebsocketRouter;
pub(crate) use message::to_message;
pub(crate) use broker::Broker;

pub use channel::{Channel, WebsocketChannel};
pub use message::{WebsocketMessage, IntoMessage};
//pub use broker::Broker;

pub use websocket_codec::Opcode;

/// Soft maximum buffer size
pub const MAX_BUFFER_SIZE: usize = 1024;
