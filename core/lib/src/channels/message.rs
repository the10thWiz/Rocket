use bytes::{Bytes, BytesMut};
use tokio::io::{AsyncRead, AsyncReadExt};
use tokio::sync::mpsc;
use ubyte::ByteUnit;
use websocket_codec::Opcode;
use websocket_codec::protocol::FrameHeader;

use crate::Data;

use super::{MAX_BUFFER_SIZE, WebsocketStatus};

/// A trait for types that can be sent on a websocket.
///
// This has default implementations for many common types, such as `String`, `Vec<u8>`, etc
/// There is a default implementation for [`crate::Data`] and `T: AsyncRead`. `Data` will
/// correctly return is_binary if the `Data` came from websocket handler. The default
/// implementation for `T: AsyncRead` always returns binary.
///
/// # Text vs Binary
///
/// The Websocket protocol requires Rocket to specify whether a message is text or binary. Rocket
/// implements this automatically where possible, but it's Rocket has not way to detect whether a
/// given message is binary or text solely based on the binary output. Most types will always turn
/// into binary or text, but it is possible for a type to be either text or binary depending on the
/// contents.
///
// TODO: implement `IntoMessage` on `Json` and other convience types
pub trait IntoMessage {
    /// Returns whether this message is binary, as opposed to text.
    ///
    /// Text, such as `String` or `Json` should return `false`.
    fn is_binary(&self) -> bool;

    /// Consumes the object, and returns a `mpsc::Receiver<Bytes>` that sends chunks of the
    /// message
    fn into_message(self) -> mpsc::Receiver<Bytes>;
}

impl IntoMessage for Data {
    fn is_binary(&self) -> bool {
        self.websocket_is_binary().unwrap_or(true)
    }

    fn into_message(self) -> mpsc::Receiver<Bytes> {
        self.open(ByteUnit::max_value()).into_message()
    }
}


impl<T: AsyncRead + Send + Unpin + 'static> IntoMessage for T {
    fn is_binary(&self) -> bool {
        true
    }

    fn into_message(mut self) -> mpsc::Receiver<Bytes> {
        let (tx, rx) = mpsc::channel(1);
        tokio::spawn(async move {
            let mut buf = BytesMut::with_capacity(MAX_BUFFER_SIZE);
            while let Ok(n) = self.read_buf(&mut buf).await {
                if n == 0 {
                    break;
                }
                let tmp = buf.split();
                let _e = tx.send(tmp.into()).await;
                if buf.capacity() <= 0 {
                    buf.reserve(MAX_BUFFER_SIZE);
                }
            }
        });
        rx
    }
}

// Compliler error, since AsyncRead could be implemented on String in future versions (probably of
// Tokio)
//impl IntoMessage for String {
    //fn is_binary(&self) -> bool {
        //false
    //}

    //fn into_message(self) -> mpsc::Receiver<Bytes> {
        //unimplemented!()
    //}
//}


/// Convience function to convert an `impl IntoMessage` into a `Message`
pub(crate) fn to_message(message: impl IntoMessage) -> WebsocketMessage {
    WebsocketMessage::new(message.is_binary(), message.into_message())
}

/// Semi-internal representation of a websocket message
///
/// This should typically never be constructed by hand, instead types that should be able to be
/// sent on a websocket channel should implement `IntoMessage`
#[derive(Debug)]
pub struct WebsocketMessage {
    header: FrameHeader,
    data: mpsc::Receiver<Bytes>,
}

impl WebsocketMessage {
    /// Create a new websocket message
    pub(crate) fn new(binary: bool, data: mpsc::Receiver<Bytes>) -> Self {
        Self {
            header: FrameHeader::new(false, 0, if binary {
                    Opcode::Binary.into()
                }else{
                    Opcode::Text.into()
                }, None, 0usize.into()),
            data,
        }
    }

    /// Creates a Close frame, with an optional status
    ///
    /// TODO: create seperate status struct
    pub(crate) fn close(status: Option<WebsocketStatus>) -> Self {
        let (tx, data) = mpsc::channel(1);
        if let Some(status) = status {
            let _e = tx.try_send(status.encode());
        }
        Self {
            header: FrameHeader::new(true, 0, Opcode::Close.into(), None, 0usize.into()),
            data
        }
    }

    /// Gets the Opcode of the message. Defaults to Opcode::Text, although it should never fail.
    ///
    /// TODO: move this into WebsocketMessage itself
    pub(crate) fn opcode(&self) -> Opcode {
        Opcode::try_from(self.header.opcode()).unwrap_or(Opcode::Text)
    }

    /// Converts this message into the internal parts
    ///
    /// See [`WebsocketMessage::from_parts`] for the reverse
    pub(crate) fn into_parts(self) -> (FrameHeader, mpsc::Receiver<Bytes>) {
        (self.header, self.data)
    }

    /// Converts the internal parts into a websocket message
    ///
    /// See [`WebsocketMessage::into_parts`] for the reverse
    pub(crate) fn from_parts(header: FrameHeader, data: mpsc::Receiver<Bytes>) -> Self {
        Self { header, data, }
    }
}

// Trivial conversion
impl IntoMessage for WebsocketMessage {
    fn is_binary(&self) -> bool {
        match Opcode::try_from(self.header.opcode()) {
            Some(Opcode::Text) => false,
            _ => true,
        }
    }

    fn into_message(self) -> mpsc::Receiver<Bytes> {
        self.data
    }
}

