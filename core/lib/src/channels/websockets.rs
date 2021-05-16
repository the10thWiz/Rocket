use std::sync::Arc;

use bytes::Bytes;
use futures::{SinkExt, StreamExt};
use rocket_http::{Status, hyper::upgrade::{Parts, Upgraded}};
use tokio::{net::TcpStream, select, sync::{Mutex, mpsc}};
use tokio_util::codec::Framed;
use websocket_codec::{Message, MessageCodec, Error};

use crate::{Request, request::{FromRequest, Outcome}};

/// A trait for types that can be sent on a websocket.
///
/// This has default implementations for many common types, such as `String`, `Vec<u8>`, etc
///
/// # Text vs Binary
/// The Websocket protocol requires Rocket to specify whether a message is text or binary. Rocket
/// implements this automatically where possible, but it's Rocket has not way to detect whether a
/// given message is binary or text solely based on the binary output. Most types will always turn
/// into binary or text, but it is possible for a type to be either text or binary depending on the
/// contents.
///
/// TODO: After contrib-graduation, implement `IntoMessage` on `Json`
pub trait IntoMessage {
    type B: Into<Bytes>;
    fn is_binary(&self) -> bool;
    fn into_bytes(self) -> Self::B;
}

impl IntoMessage for String {
    type B = Self;
    fn is_binary(&self) -> bool {
        false
    }

    fn into_bytes(self) -> Self::B {
        self
    }
}

impl IntoMessage for &String {
    type B = String;
    fn is_binary(&self) -> bool {
        false
    }

    fn into_bytes(self) -> Self::B {
        self.clone()
    }
}

impl IntoMessage for &str {
    type B = String;
    fn is_binary(&self) -> bool {
        false
    }

    fn into_bytes(self) -> Self::B {
        self.to_string()
    }
}

/// Convience function to convert an `impl IntoMessage` into a `Message`
pub(crate) fn to_message(message: impl IntoMessage) -> Message {
    if message.is_binary() {
        Message::new(websocket_codec::Opcode::Binary, message.into_bytes()).unwrap()
    }else {
        Message::new(websocket_codec::Opcode::Text, message.into_bytes()).unwrap()
    }
}

/// A Websocket connection, directly connected to a client.
///
/// Messages sent with the `send` method are only sent to one client, the one who sent the message.
/// This is also nessecary for subscribing clients to specific channels.
#[derive(Clone)]
pub struct Channel {
    inner: Arc<Mutex<Option<Framed<Upgraded, MessageCodec>>>>,
    channels: Option<()>,
    reciever: Arc<Mutex<mpsc::UnboundedReceiver<Message>>>,
    sender: mpsc::UnboundedSender<Message>,
}

impl Channel {
    /// Create a fake reciever, for detecting internal errors in the FromRequest implementation
    pub(crate) fn empty() -> Self {
        let (sender, reciever) = mpsc::unbounded_channel();
        Self {
            inner: Arc::new(Mutex::new(None)),
            channels: None,
            reciever: Arc::new(Mutex::new(reciever)),
            sender,
        }
    }

    /// Create a real reciever, without an inner channel
    pub(crate) fn new() -> Self {
        let (sender, reciever) = mpsc::unbounded_channel();
        Self {
            inner: Arc::new(Mutex::new(None)),
            channels: Some(()),
            reciever: Arc::new(Mutex::new(reciever)),
            sender,
        }
    }

    /// Add the inner channel
    pub(crate) async fn add_inner(&self, upgrade: Upgraded) {
        let tmp: Result<Parts<TcpStream>, Upgraded> = upgrade.downcast();
        if let Ok(parts) = tmp {
            //parts.io.write
        }
        //let mut stream = self.inner.lock().await;
        //*stream = Some(inner);
    }

    /// Gets the handle to subscribe this channel to a descriptor
    pub(crate) fn subscribe_handle(&self) -> mpsc::UnboundedSender<Message> {
        self.sender.clone()
    }

    /// Get the next message from this client.
    ///
    /// This method also forwards messages sent from any channels the client is subscribed to
    pub(crate) async fn next(&self) -> Option<Result<Message, Error>> {
        let mut lock = self.inner.lock().await;
        let stream = lock.as_mut().unwrap();

        let mut recv = self.reciever.lock().await;
        loop {
            select! {
                m = stream.next() => return m,
                Some(m) = recv.recv() => {
                    if let Err(e) = stream.send(m).await {
                        return Some(Err(e));
                    }
                },
            }
        }
    }

    /// Sends a raw Message to the client
    pub(crate) async fn send_raw(&self, message: Message) {
        let mut lock = self.inner.lock().await;
        let stream = lock.as_mut().unwrap();
        let _ = stream.send(message).await;
    }

    /// Send a message to the specific client connected to this websocket
    pub async fn send(&self, message: impl IntoMessage) {
        self.send_raw(to_message(message)).await
    }

    /// Sends a close notificaiton to the client, so no new messages will arive
    pub async fn close(&self) {
        self.send_raw(Message::close(None)).await
    }

    /// Sends a close notificaiton to the client, along with a reason for the close
    pub async fn close_with_status(&self, status: Status) {
        self.send_raw(
            Message::close(Some((status.code, status.reason().unwrap_or("").to_string())))
        ).await
    }
}

#[crate::async_trait]
impl<'r> FromRequest<'r> for Channel {
    type Error = &'static str;
    async fn from_request(request: &'r Request<'_>) -> Outcome<Self, Self::Error> {
        let tmp = request.local_cache(|| Self::empty()).clone();
        if tmp.channels.is_some() {
            Outcome::Success(tmp)
        }else {
            Outcome::Failure((Status::InternalServerError, "Websockets not initialized"))
        }
    }
}
