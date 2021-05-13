
use super::Result;

/// Analogous enum to tungstenite::protocol::Message,
///
/// but Ping, Pong and Close should be handled mostly automatically.
#[derive(Debug, Clone)]
pub enum Message {
    Open(),
    Text(String),
    Binary(Vec<u8>),
    Close(),
}

impl Message {
    /// Converts this message into the appropriate tungstenite message.
    ///
    /// Returns None if the message could not be converted.
    pub fn as_tungstenite(self) -> Option<tokio_tungstenite::tungstenite::Message> {
        match self {
            Self::Text(s) => Some(tokio_tungstenite::tungstenite::Message::Text(s)),
            Self::Binary(v) => Some(tokio_tungstenite::tungstenite::Message::Binary(v)),
            Self::Close() => Some(tokio_tungstenite::tungstenite::Message::Close(None)),
            Self::Open() => None,
        }
    }
    /// Converts from a tungstenite message into a Message.
    ///
    /// Returns an Error with the original message if the message type is not Text, Binary, or Close.
    /// All other message types should be handled automatically, and likely ignored.
    pub fn from_tungstenite(message: tokio_tungstenite::tungstenite::Message) -> std::result::Result<Self, tokio_tungstenite::tungstenite::Message> {
        match message {
            tokio_tungstenite::tungstenite::Message::Text(s) => Ok(Self::Text(s)),
            tokio_tungstenite::tungstenite::Message::Binary(s) => Ok(Self::Binary(s)),
            tokio_tungstenite::tungstenite::Message::Close(_) => Ok(Self::Close()),
            m => Err(m),
        }
    }
}

/// A type that can be converted from a Message; analogous to the FromData trait
pub trait FromMessage: Sized {
    fn from_message(message: Message) -> Result<Self>;
}

impl FromMessage for Message {
    fn from_message(message: Message) -> Result<Self> {
        Ok(message)
    }
}
