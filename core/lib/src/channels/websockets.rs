use std::sync::Arc;

use bytes::{Bytes, BytesMut};
use futures::{SinkExt, StreamExt};
use rocket_http::{Status, hyper::upgrade::{Parts, Upgraded}};
use tokio::{io::AsyncReadExt, net::TcpStream, select, sync::{Mutex, mpsc, oneshot}};
use tokio_util::codec::{Decoder, Framed};
use websocket_codec::{Error, Message, MessageCodec, protocol::{FrameHeader, FrameHeaderCodec}};

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

//struct InnerChannel {
    //inner: Parts<TcpStream>,
    //current_frame: Option<(websocket_codec::protocol::FrameHeader, usize)>,
    //reciever: mpsc::UnboundedReceiver<Message>,
//}

//impl InnerChannel {
    //fn new(inner: Parts<TcpStream>, reciever: mpsc::UnboundedReceiver<Message>) -> Self {
        //Self {
            //inner, 
            //reciever,
            //current_frame: None,
        //}
    //}
    //async fn next(&mut self) {
        ////AsyncReadExt
        ////let tmp = self.inner.read(&mut self.read_buf).await;
        ////let tmp = self.frame_codec.decode(&mut self.read_buf);
        ////FrameHeader::pa
    //}
    //async fn send(&mut self) {
    //}
//}

/// A Websocket connection, directly connected to a client.
///
/// Messages sent with the `send` method are only sent to one client, the one who sent the message.
/// This is also nessecary for subscribing clients to specific channels.
pub struct WebsocketChannel {
    //inner: Arc<Mutex<Option<InnerChannel>>>,
    inner: mpsc::Receiver<Message>,
    //channels: Option<()>,
    //reciever: Arc<Mutex<mpsc::UnboundedReceiver<Message>>>,
    sender: mpsc::Sender<Message>,
    upgrade_tx: oneshot::Sender<Upgraded>
}

impl WebsocketChannel {
    pub(crate) fn new() -> Self {
        let (broker_tx, broker_rx) = mpsc::channel(10);
        let (upgrade_tx, upgrade_rx) = oneshot::channel();
        let (message_tx, message_rx) = mpsc::channel(0);
        tokio::spawn(Self::message_handler(upgrade_rx, broker_rx, message_tx));
        Self {
            inner: message_rx,
            //channels: Some(()),
            //reciever: Arc::new(Mutex::new(reciever)),
            sender: broker_tx,
            upgrade_tx,
        }
    }

    async fn message_handler(upgrade_rx: oneshot::Receiver<Upgraded>, broker_rx: mpsc::Receiver<Message>, message_tx: mpsc::Sender<Message>) {

    }

    /// Gets the handle to subscribe this channel to a descriptor
    pub(crate) fn subscribe_handle(&self) -> mpsc::Sender<Message> {
        self.sender.clone()
    }

    pub(crate) fn upgraded(&self, upgrade: Upgraded) {
        self.upgrade_tx.send(upgrade);
    }

    /// Get the next message from this client.
    ///
    /// This method also forwards messages sent from any channels the client is subscribed to
    pub(crate) async fn next(&mut self) -> Option<Message> {
        self.inner.recv().await
    }
}

#[derive(Clone)]
pub struct Channel(mpsc::Sender<Message>);

impl Channel {
    pub(crate) fn from_websocket(chan: &WebsocketChannel) -> Self {
        Self(chan.subscribe_handle())
    }

    /// Sends a raw Message to the client
    pub(crate) async fn send_raw(&self, message: Message) {
        let _ = self.0.send(message).await;
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
        let tmp = request.local_cache(|| None).clone();
        if let Some(tmp) = tmp {
            Outcome::Success(tmp)
        }else {
            Outcome::Failure((Status::InternalServerError, "Websockets not initialized"))
        }
    }
}
