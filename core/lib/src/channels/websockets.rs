use std::sync::Arc;

use bytes::Bytes;
use futures::{SinkExt, StreamExt};
use rocket_http::{Status, hyper::upgrade::Upgraded};
use tokio::{select, sync::{Mutex, mpsc}};
use tokio_util::codec::Framed;
use websocket_codec::{Message, MessageCodec, Error};

use crate::{Request, request::{FromRequest, Outcome}};

use super::channel::WebsocketMessage;

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

#[derive(Clone)]
pub struct Websocket {
    inner: Arc<Mutex<Option<Framed<Upgraded, MessageCodec>>>>,
    channels: Option<mpsc::UnboundedSender<WebsocketMessage>>,
}

impl Websocket {
    pub(crate) fn empty() -> Self {
        Self {
            inner: Arc::new(Mutex::new(None)),
            channels: None,
        }
    }

    pub(crate) fn new(channels: mpsc::UnboundedSender<WebsocketMessage>) -> Self {
        Self {
            inner: Arc::new(Mutex::new(None)),
            channels: Some(channels),
        }
    }

    pub(crate) async fn add_inner(&self, inner: Framed<Upgraded, MessageCodec>) {
        let mut stream = self.inner.lock().await;
        *stream = Some(inner);
    }

    pub(crate) async fn next(&self) -> Option<Result<Message, Error>> {
        let mut lock = self.inner.lock().await;
        let stream = lock.as_mut().unwrap();
        stream.next().await
    }

    pub(crate) async fn send_raw(&self, message: Message) {
        let mut lock = self.inner.lock().await;
        let stream = lock.as_mut().unwrap();
        let _ = stream.send(message).await;
    }

    fn to_message(message: impl IntoMessage) -> Message {
        if message.is_binary() {
            Message::new(websocket_codec::Opcode::Binary, message.into_bytes()).unwrap()
        }else {
            Message::new(websocket_codec::Opcode::Text, message.into_bytes()).unwrap()
        }
    }

    pub async fn send(&self, message: impl IntoMessage) {
        let mut lock = self.inner.lock().await;
        let stream = lock.as_mut().unwrap();
        let _ = stream.send(Self::to_message(message)).await;
    }

    pub async fn send_to(&self, channel_id: (), message: impl IntoMessage) {
    }

    pub async fn close(&self) {
        let mut lock = self.inner.lock().await;
        let stream = lock.as_mut().unwrap();
        let _ = stream.send(Message::close(None)).await;
    }

    pub async fn close_with_status(&self, status: Status) {
        let mut lock = self.inner.lock().await;
        let stream = lock.as_mut().unwrap();
        let _ = stream.send(Message::close(Some((status.code, status.reason().unwrap_or("").to_string())))).await;
    }
}

#[crate::async_trait]
impl<'r> FromRequest<'r> for Websocket {
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

pub struct WebsocketHandle {
}
