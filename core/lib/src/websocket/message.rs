use std::io::Cursor;

use bytes::{Bytes, BytesMut};
use rocket_http::uri::Origin;
use tokio::io::{AsyncRead, AsyncReadExt};
use tokio::sync::mpsc;
use ubyte::ByteUnit;
use websocket_codec::Opcode;
use websocket_codec::protocol::FrameHeader;

use crate::Data;

use super::status::StatusError;
use super::{MAX_BUFFER_SIZE, status::WebSocketStatus};

/// A trait for types that can be sent on a webSocket.
///
// This has default implementations for many common types, such as `String`, `Vec<u8>`, etc
/// There is a default implementation for [`crate::Data`] and `T: AsyncRead`. `Data` will
/// correctly return is_binary if the `Data` came from webSocket handler. The default
/// implementation for `T: AsyncRead` always returns binary.
///
/// # Text vs Binary
///
/// The WebSocket protocol requires Rocket to specify whether a message is text or binary. Rocket
/// implements this automatically where possible, but it's Rocket has not way to detect whether a
/// given message is binary or text solely based on the binary output. Most types will always turn
/// into binary or text, but it is possible for a type to be either text or binary depending on the
/// contents.
///
/// # Notes for implementing `IntoMessage`
// TODO: implement `IntoMessage` on `Json` and other convience types
#[crate::async_trait]
pub trait IntoMessage {
    /// Returns whether this message is binary, as opposed to text.
    ///
    /// Text, such as `String` or `Json` should return `false`.
    fn is_binary(&self) -> bool;

    /// Consumes the object, and returns a `mpsc::Receiver<Bytes>` that sends chunks of the
    /// message
    async fn into_message(self, sender: mpsc::Sender<Bytes>);
}

#[crate::async_trait]
impl<'r> IntoMessage for Data<'r> {
    fn is_binary(&self) -> bool {
        self.was_ws_binary().unwrap_or(true)
    }

    async fn into_message(self, sender: mpsc::Sender<Bytes>) {
        into_message(self.open(ByteUnit::max_value()), sender).await;
    }
}

/// Helper function for implementing `IntoMessage`. Converts a type that implements AsyncRead into
/// `mpsc::Receiver<Bytes>`, the type `IntoMessage` requires.
pub async fn into_message<T: AsyncRead + Unpin>(mut t: T, tx: mpsc::Sender<Bytes>) {
    let mut buf = BytesMut::with_capacity(MAX_BUFFER_SIZE);
    while let Ok(n) = t.read_buf(&mut buf).await {
        if n == 0 {
            break;
        }
        let tmp = buf.split();
        let _e = tx.send(tmp.into()).await;
        if buf.capacity() <= 0 {
            buf.reserve(MAX_BUFFER_SIZE);
        }
    }
}

//impl<T: AsyncRead + Send + Unpin + 'static> IntoMessage for T {
    //fn is_binary(&self) -> bool {
        //true
    //}

    //fn into_message(mut self) -> mpsc::Receiver<Bytes> {
        //let (tx, rx) = mpsc::channel(1);
        //tokio::spawn(async move {
            //let mut buf = BytesMut::with_capacity(MAX_BUFFER_SIZE);
            //while let Ok(n) = self.read_buf(&mut buf).await {
                //if n == 0 {
                    //break;
                //}
                //let tmp = buf.split();
                //let _e = tx.send(tmp.into()).await;
                //if buf.capacity() <= 0 {
                    //buf.reserve(MAX_BUFFER_SIZE);
                //}
            //}
        //});
        //rx
    //}
//}

// Compliler error, since AsyncRead could be implemented on String in future versions (probably of
// Tokio)
// Alternative is implementing on every type manally (or macro), but this makes writing custom
// IntoMessage types harder.

//impl IntoMessage for String {
    //fn is_binary(&self) -> bool {
        //false
    //}

    //fn into_message(self) -> mpsc::Receiver<Bytes> {
        //into_message(Cursor::new(self))
    //}
//}

//impl IntoMessage for &str {
    //fn is_binary(&self) -> bool {
        //false
    //}

    //fn into_message(self) -> mpsc::Receiver<Bytes> {
        //into_message(Cursor::new(self.to_string()))
    //}
//}

//impl IntoMessage for Vec<u8> {
    //fn is_binary(&self) -> bool {
        //true
    //}

    //fn into_message(self) -> mpsc::Receiver<Bytes> {
        //into_message(Cursor::new(self))
    //}
//}

//impl IntoMessage for &[u8] {
    //fn is_binary(&self) -> bool {
        //true
    //}

    //fn into_message(self) -> mpsc::Receiver<Bytes> {
        //into_message(Cursor::new(self.to_vec()))
    //}
//}

/// Convience function to convert an `impl IntoMessage` into a `Message`
pub(crate) async fn to_message(message: impl IntoMessage, message_tx: &mpsc::Sender<WebSocketMessage>) {
    let (tx, rx) = mpsc::channel(1);
    if let Ok(()) = message_tx.send(WebSocketMessage::new(message.is_binary(), rx)).await {
        message.into_message(tx).await;
    }
}

/// Semi-internal representation of a webSocket message
///
/// This should typically never be constructed by hand, instead types that should be able to be
/// sent on a webSocket channel should implement `IntoMessage`
#[derive(Debug)]
pub struct WebSocketMessage {
    header: FrameHeader,
    topic: Option<Origin<'static>>,
    data: mpsc::Receiver<Bytes>,
}

impl WebSocketMessage {
    /// Create a new webSocket message
    pub(crate) fn new(binary: bool, data: mpsc::Receiver<Bytes>) -> Self {
        Self {
            header: FrameHeader::new(false, 0, if binary {
                    Opcode::Binary.into()
                }else{
                    Opcode::Text.into()
                }, None, 0usize.into()),
            topic: None,
            data,
        }
    }

    /// Creates a Close frame, with an optional status
    ///
    /// TODO: create seperate status struct
    pub(crate) fn close(status: Option<WebSocketStatus<'_>>) -> Self {
        let (tx, data) = mpsc::channel(3);
        if let Some(status) = status {
            let _e = tx.try_send(status.encode());
        }
        Self {
            header: FrameHeader::new(true, 0, Opcode::Close.into(), None, 0usize.into()),
            topic: None,
            data
        }
    }

    /// Gets the Opcode of the message. Defaults to Opcode::Pong, although it should never fail.
    ///
    /// This should only return Text, Binary, and Close, since all other opcodes should be handled
    /// by the channel itself.
    pub(crate) fn opcode(&self) -> Opcode {
        Opcode::try_from(self.header.opcode()).unwrap_or(Opcode::Pong)
    }

    /// Converts this message into the internal parts
    ///
    /// See [`WebSocketMessage::from_parts`] for the reverse
    pub(crate) fn into_parts(self)
        -> (FrameHeader, Option<Origin<'static>>, mpsc::Receiver<Bytes>)
    {
        (self.header, self.topic, self.data)
    }

    /// Converts the internal parts into a webSocket message
    ///
    /// See [`WebSocketMessage::into_parts`] for the reverse
    pub(crate) fn from_parts(
        header: FrameHeader,
        topic: Option<Origin<'static>>,
        data: mpsc::Receiver<Bytes>
    ) -> Self {
        Self { header, topic, data, }
    }

    /// Set the topic of this message
    pub(crate) fn with_topic(mut self, topic: Origin<'static>) -> Self {
        self.topic = Some(topic);
        self
    }

    /// Gets the inner data channel
    pub(crate) fn inner(self) -> mpsc::Receiver<Bytes> {
        self.data
    }

    pub(crate) fn default_response(status: Result<WebSocketStatus<'_>, StatusError>) -> Self {
        match status {
            // Specific matches
            Ok(s) if s == WebSocketStatus::Ok => Self::close(Some(WebSocketStatus::Ok)),
            Ok(s) if s == WebSocketStatus::GoingAway => Self::close(Some(WebSocketStatus::Ok)),
            Ok(s) if s == WebSocketStatus::ExtensionRequired => Self::close(Some(WebSocketStatus::Ok)),
            Ok(s) if s == WebSocketStatus::UnknownMessageType => Self::close(Some(WebSocketStatus::Ok)),
            Ok(s) if s == WebSocketStatus::InvalidDataType => Self::close(Some(WebSocketStatus::Ok)),
            Ok(s) if s == WebSocketStatus::PolicyViolation => Self::close(Some(WebSocketStatus::Ok)),
            Ok(s) if s == WebSocketStatus::MessageTooLarge => Self::close(Some(WebSocketStatus::Ok)),
            Ok(s) if s == WebSocketStatus::InternalServerError => Self::close(Some(WebSocketStatus::Ok)),
            // 3000..=3999 is defined by the IANA, 4000..=4999 is private use
            Ok(s) if (3000..=4999).contains(&s.code()) => Self::close(Some(WebSocketStatus::Ok)),
            // If the frame was empty (not malformed), we response with Ok
            Err(StatusError::NoStatus) => Self::close(Some(WebSocketStatus::Ok)),
            // Default to protocol error
            _ => Self::close(Some(WebSocketStatus::ProtocolError)),
        }
    }
}

// Trivial conversion
//impl IntoMessage for WebSocketMessage {
    //fn is_binary(&self) -> bool {
        //match Opcode::try_from(self.header.opcode()) {
            //Some(Opcode::Text) => false,
            //_ => true,
        //}
    //}

    //fn into_message(self) -> mpsc::Receiver<Bytes> {
        //self.data
    //}
//}

