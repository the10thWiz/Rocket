use rocket_http::uri::Origin;
use tokio::sync::mpsc;

use super::message::WebSocketMessage;



pub struct WebSocket<'r> {
    topic: Origin<'r>,
    handle: mpsc::Sender<WebSocketMessage>,
}

impl<'r> WebSocket<'r> {
    pub(crate) fn from(topic: Origin<'r>, handle: mpsc::Sender<WebSocketMessage>) -> Self {
        Self { topic, handle }
    }

    pub fn topic(&self) -> &Origin<'r> {
        &self.topic
    }
}
