

pub(crate) enum WebsocketMessage {
    /// Registers a websocket to recieve messages from a room
    ///
    /// Note: this should only be sent once per websocket connection
    Register(),
    /// Sends a message that should be forwarded to every socket listening
    Forward((), ()),
}


pub struct Channel {
}
