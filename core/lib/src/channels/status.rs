use bytes::{Bytes, BytesMut};

/// A websocket status code sent while closing the connection
pub struct WebsocketStatus {
    code: u16,
    reason: &'static str,
}

macro_rules! websocket_status_impl {
    ($($name:ident => $code:expr),*) => {
        $(
            /// Websocket pre-defined Status code
            pub const $name: WebsocketStatus = WebsocketStatus { code: $code, reason: "$name" };
        )*
    }
}

websocket_status_impl! {
    OK => 1000,
    GOING_AWAY => 1001,
    PROTOCOL_ERROR => 1002,
    UNKNOWN_MESSAGE_TYPE => 1003,
    RESERVED => 1004,
    NO_STATUS_CODE => 1005,
    ABNORMAL_CLOSE => 1006,
    INVALID_DATA_TYPE => 1007,
    POLICY_VIOLATION => 1008,
    MESSAGE_TOO_LARGE => 1009,
    EXTENSION_REQUIRED => 1010,
    INTERNAL_SERVER_ERROR => 1011,
    TLS_FAILURE => 1015
}

impl WebsocketStatus {
    /// Create a new status with a code and reason.
    ///
    /// # Panics
    ///
    /// Panics if the code is not in the range of `3000..=4999`.
    ///
    /// If you want to close with a pre-defined status code in the `1000..=2999` range, use one of
    /// the predefined constants in the channel module.
    pub fn new(code: u16, reason: &'static str) -> Self {
        match code {
            0000..=0999 => panic!("Status codes in the range 0-999 are not used"),
            1000..=2999 => panic!(
                "Status codes in the range 1000-2999 are reserved for the websocket protocol"
            ),
            3000..=3999 => (),
            4000..=4999 => (),
            _ => panic!("Cannot create a status code outside the allowed range"),
        }
        Self { code, reason }
    }

    /// Encodes this status code into a buffer
    ///
    /// Does not add any bytes to the buffer is `self` is not permitted to be returned by the
    /// server
    pub(crate) fn encode(&self) -> Bytes {
        match self.code {
            1005 | 1006 | 1010 | 1015 => return Bytes::new(),
            _ => (),
        }
        let mut buf = BytesMut::new();
        buf.extend(&self.code.to_ne_bytes());
        buf.extend(self.reason.bytes());
        buf.freeze()
    }
}
