use std::collections::HashMap;

use tokio::sync::mpsc;
use websocket_codec::Message;


pub trait ChannelDescriptor {
    fn matches(&self, other: &dyn ChannelDescriptor) -> bool;
}

pub(crate) enum WebsocketMessage {
    /// Registers a websocket to recieve messages from a room
    ///
    /// Note: this should only be sent once per websocket connection
    Register(String, mpsc::UnboundedSender<Message>),
    /// Sends a message that should be forwarded to every socket listening
    Forward(String, Message),
}


pub struct Channel {
}

impl Channel {
    pub(crate) async fn channel_task(mut rx: mpsc::UnboundedReceiver<WebsocketMessage>) {
        let mut map = HashMap::new();
        while let Some(wsm) = rx.recv().await {
            match wsm {
                WebsocketMessage::Register(room, tx) => map.entry(room).or_insert(vec![]).push(tx),
                WebsocketMessage::Forward(room, message) => if let Some(v) = map.get(&room) {
                    for tx in v {
                        let _ = tx.send(message.clone());
                    }
                },
            }
        }
    }
}
