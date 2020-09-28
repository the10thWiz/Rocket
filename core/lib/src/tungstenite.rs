//! WebSocket handling based on tungstenite crate.

use crate::request::{FromRequest, Outcome, Request};
use crate::response::Response;
use crate::{http, Data};

pub use tokio_tungstenite::{
    tungstenite::protocol::{self, WebSocketConfig},
    tungstenite::Error as WsError,
    WebSocketStream,
};

/// WebSocket Upgrade request headers.
///
/// This guard makes sure a valid upgrade headers are present inside a request.
pub struct WsUpgrade<'a> {
    sec_websocket_key: &'a str,
}

impl<'a> WsUpgrade<'a> {
    /// Replies to the Upgrade request with a handshake.
    pub fn accept(&self) -> Response<'a> {
        use http::Header;
        Response::build()
            .header(Header::new("Connection", "Upgrade"))
            .header(Header::new("Upgrade", "websocket"))
            .header(Header::new(
                "sec-websocket-accept",
                convert_key(self.sec_websocket_key),
            ))
            .status(http::Status::SwitchingProtocols)
            .finalize()
    }
}

#[crate::async_trait]
impl<'a, 'r> FromRequest<'a, 'r> for WsUpgrade<'a> {
    type Error = ();

    async fn from_request(request: &'a Request<'r>) -> Outcome<Self, Self::Error> {
        // FIXME: failure when
        //  - upgrade is missing (426 status)
        //  - or version is other than 13 (400 bad request)

        let get_header = |name| request.headers().get_one(name).unwrap_or_default();
        if !get_header("upgrade").eq_ignore_ascii_case("websocket")
            || get_header("sec-websocket-version") != "13"
        {
            return Outcome::Forward(());
        }

        if let Some(sec_websocket_key) = request.headers().get_one("sec-websocket-key") {
            Outcome::Success(WsUpgrade { sec_websocket_key })
        } else {
            Outcome::Forward(())
        }
    }
}

// TODO: eventually make the very same function in tungstenite public
fn convert_key(key: &str) -> String {
    use sha1::Digest;
    let mut sha1 = sha1::Sha1::default();
    sha1.input(key);
    sha1.input(&b"258EAFA5-E914-47DA-95CA-C5AB0DC85B11"[..]);
    base64::encode(&sha1.result())
}

/// Transforms underlying socket from Upgrade request into WebSocket stream.
pub async fn on_upgrade(
    data: Data,
    config: Option<WebSocketConfig>,
) -> Result<WebSocketStream<http::hyper::Upgraded>, http::hyper::Error> {
    let socket = data.into_hyper_body().on_upgrade().await?;
    Ok(WebSocketStream::from_raw_socket(socket, protocol::Role::Server, config).await)
}
