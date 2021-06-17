//! WebSocket Status codes
//!
//! The RFC defines a number of specific status codes, in the 1000 range, which are provided as
//! constants here. The RFC also defines ranges for the IANA to define status codes, and for
//! private use. Status codes can be created and used as needed, within the RFC's definitions.
//!
//! There are a number of status codes, as well as some ranges of codes that the RFC specifically
//! forbids servers from sending to clients. The `WebSocketStatus::encode` method implemenets this
//! by printing an error to the console, and encoding `INTERNAL_SERVER_ERROR`, indicating to the
//! client that something went wrong on the server.

use std::{borrow::Cow, string::FromUtf8Error};

use bytes::{Bytes, BytesMut};
use rocket_http::Status;

/// A webSocket status code sent while closing the connection
#[derive(Debug, Clone, Eq)]
pub struct WebSocketStatus<'a> {
    code: u16,
    reason: Cow<'a, str>,
}

impl<'a> PartialEq for WebSocketStatus<'a> {
    /// Status equality ignores the reason, since there is no offical, defined standard for what
    /// they are
    fn eq(&self, other: &Self) -> bool {
        self.code == other.code
    }
}

impl<'a> std::fmt::Display for WebSocketStatus<'a> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} {}", self.code, self.reason)
    }
}

/// Errors for parsing Status codes
#[derive(Debug)]
pub enum StatusError {
    /// A status code that is outside the range defined by the RFC
    OutOfRange,
    /// A status code specifically banned by the RFC
    IllegalStatus,
    /// A close frame with an incomplete code
    BadFrame,
    /// A close frame with a reason that isn't valid UTF-8
    Utf8Error(FromUtf8Error),
    /// An empty close frame
    NoStatus,
}

impl From<FromUtf8Error> for StatusError {
    fn from(e: FromUtf8Error) -> Self {
        Self::Utf8Error(e)
    }
}

macro_rules! websocket_status_impl {
    ($($name:ident => $code:expr),*) => {
        $(
            /// WebSocket pre-defined Status code
            #[allow(non_upper_case_globals)]
            pub const $name: WebSocketStatus<'static> = WebSocketStatus {
                code: $code,
                reason: Cow::Borrowed(stringify!($name))
            };
        )*
    }
}

impl<'a> WebSocketStatus<'a> {
    websocket_status_impl! {
        Ok => 1000,
        GoingAway => 1001,
        ProtocolError => 1002,
        UnknownMessageType => 1003,
        Reserved => 1004,
        NoStatusCode => 1005,
        AbnormalClose => 1006,
        InvalidDataType => 1007,
        PolicyViolation => 1008,
        MessageTooLarge => 1009,
        ExtensionRequired => 1010,
        InternalServerError => 1011,
        TlsFailure => 1015
    }

    /// Create a new status with a code and reason.
    ///
    /// # Panics
    ///
    /// Panics if the code is not in the range of `3000..=4999`.
    ///
    /// If you want to close with a pre-defined status code in the `1000..=2999` range, use one of
    /// the predefined constants in the channel module.
    pub fn new(code: u16, reason: Cow<'a, str>) -> Self {
        match code {
            0000..=0999 => panic!("Status codes in the range 0-999 are not used"),
            1000..=2999 => panic!(
                "Status codes in the range 1000-2999 are reserved for the WebSocket protocol"
            ),
            3000..=3999 => (),
            4000..=4999 => (),
            _ => panic!("Cannot create a status code outside the allowed range"),
        }
        Self { code, reason }
    }

    pub fn with_reason<'b>(&self, reason: Cow<'b, str>) -> WebSocketStatus<'b> {
        WebSocketStatus {
            code: self.code,
            reason,
        }
    }

    /// Internal method for creating status codes. This does not attach a reason, and allows codes
    /// outside of the normal range to be created. This is primarily useful for creating Status
    /// codes that represent HTTP statuses, and can later be converted into one.
    pub(crate) fn internal(code: u16) -> Self {
        Self { code, reason: Cow::Borrowed("") }
    }

    /// Encodes this status code into a buffer
    ///
    /// If self is not permitted to be sent by the server, the `InternalServerError` status code is
    /// encoded instead
    pub(crate) fn encode(&self) -> Bytes {
        match self.code {
            1005 | 1006 | 1010 | 1015 => {
                error_!("Status code {} is not permitted to be sent by a server", self.code);
                return Self::InternalServerError.encode()
            },
            _ => (),
        }
        //crate::log::info_!("Closing connection: {:?}", self);
        let mut buf = BytesMut::new();
        buf.extend(&self.code.to_be_bytes());
        buf.extend(self.reason.bytes());
        buf.freeze()
    }

    /// Gets the code sent with this status
    pub fn code(&self) -> u16 {
        self.code
    }

    /// Gets the code sent with this status
    pub fn reason(&'a self) -> &'a str {
        self.reason.as_ref()
    }

    pub(crate) fn to_http(&self) -> Result<Status, ()> {
        match self.code {
            0..=999 => Status::from_code(self.code).ok_or(()),
            1000 => Ok(Status::Ok),
            1002 => Ok(Status::BadRequest),
            1011 => Ok(Status::InternalServerError),
            _ => Err(()),
        }
    }
}

impl From<Status> for WebSocketStatus<'static> {
    fn from(s: Status) -> Self {
        match s.code {
            200 => Self::Ok,
            // TODO expand table
            _ => Self::NoStatusCode,
        }
    }
}

// Maybe remove leading zeros?
impl WebSocketStatus<'static> {
    pub(crate) fn decode(mut bytes: Bytes) -> Result<Self, StatusError> {
        if bytes.len() == 0 {
            Err(StatusError::NoStatus)
        } else if bytes.len() < 2 {
            Err(StatusError::BadFrame)
        } else {
            let code =  u16::from_be_bytes([bytes[0], bytes[1]]);
            match code {
                0000..=0999 => Err(StatusError::OutOfRange),
                1005 | 1006 => Err(StatusError::IllegalStatus),
                1000 | 1001 | 1002 | 1003 | 1004 | 1007 | 1008 | 1009 | 1010 | 1011 | 1015 =>
                Ok(Self {
                    code,
                    reason: Cow::Owned(String::from_utf8(bytes.split_off(2).to_vec())?),
                }),
                3000..=4999 => Ok(Self {
                    code,
                    reason: Cow::Owned(String::from_utf8(bytes.split_off(2).to_vec())?),
                }),
                0000..=2999 => Err(StatusError::OutOfRange),
                _ => Ok(Self {
                    code,
                    reason: Cow::Owned(String::from_utf8(bytes.split_off(2).to_vec())?),
                }),
            }
        }
    }
}
