use crate::{Request, request::{FromRequest, Outcome}};
use async_trait::async_trait;
use rocket_http::{Status, hyper::upgrade::Upgraded};
use tokio::sync::mpsc;
use tokio_tungstenite::WebSocketStream;

use super::Message;

pub struct Websocket<'r> {
    tx: &'r mpsc::UnboundedSender<Message>,
}

impl<'r> Websocket<'r> {
    pub fn send(&self, message: Message) {
        let error = self.tx.send(message);
        println!("transfer result: {:?}", error);
    }
}

#[async_trait]
impl<'r> FromRequest<'r> for Websocket<'r> {
    type Error = ();
    async fn from_request(request: &'r Request<'_>) -> Outcome<Self, Self::Error> {
        let websocket: &Option<mpsc::UnboundedSender<Message>> = request.local_cache(|| None);
        if let Some(tx) = websocket {
            Outcome::Success(Self {tx })
        }else {
            Outcome::Failure((Status::InternalServerError, ()))
        }
    }
}
