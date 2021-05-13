use std::{io::Cursor, net::SocketAddr, pin::Pin, sync::Arc};

use futures::{Future, FutureExt, SinkExt, StreamExt, stream::Stream};
use rocket_http::{Header, Method, Status, hyper::{self, HeaderValue, header::{CONNECTION, UPGRADE}, upgrade::OnUpgrade}, uri::Origin};
use tokio::sync::{mpsc, oneshot};
use tokio_tungstenite::{WebSocketStream, tungstenite::protocol::Role};

use crate::{Data, Orbit, Request, Response, Rocket, Route, router::Collisions};
use yansi::Paint;

use super::Message;

enum WebsocketMessage {
    /// Registers a websocket to recieve messages from a room
    ///
    /// Note: this should only be sent once per websocket connection
    Register(),
    /// Sends a message that should be forwarded to every socket listening
    Forward(Message, String),
}


#[derive(Clone, Debug)]
pub(crate) struct WebsocketRouter {
    transmitter: mpsc::UnboundedSender<WebsocketMessage>,
    routes: Vec<Route>,
}


impl WebsocketRouter {
    pub fn new() -> Self {
        let (transmitter, rx) = mpsc::unbounded_channel();
        tokio::spawn(Self::message_router(rx));
        Self{
            transmitter,
            routes: vec![],
        }
    }
    async fn message_router(reciever: mpsc::UnboundedReceiver<WebsocketMessage>) {
    }
    pub fn finalize(&mut self) -> Result<(), Collisions> {
        Ok(())
    } 
    pub fn add_route(&mut self, route: Route) {
        if route.websocket && route.method == Method::Get {
            self.routes.push(route);
            self.routes.sort_by_key(|r| r.rank);
        }
    }
    fn routes_match<'r, 'a: 'r>(
        &'a self,
        req: &'r Request<'r>,
    ) -> impl Iterator<Item = &'a Route> + 'r {
        // Note that routes are presorted by ascending rank on each `add`.
        self.routes.iter().filter(move |r| r.matches(req))
    }
}


async fn handle<Fut, T, F>(name: Option<&str>, run: F) -> Option<T>
    where F: FnOnce() -> Fut, Fut: Future<Output = T>,
{
    use std::panic::AssertUnwindSafe;

    macro_rules! panic_info {
        ($name:expr, $e:expr) => {{
            match $name {
                Some(name) => error_!("Handler {} panicked.", Paint::white(name)),
                None => error_!("A handler panicked.")
            };

            info_!("This is an application bug.");
            info_!("A panic in Rust must be treated as an exceptional event.");
            info_!("Panicking is not a suitable error handling mechanism.");
            info_!("Unwinding, the result of a panic, is an expensive operation.");
            info_!("Panics will severely degrade application performance.");
            info_!("Instead of panicking, return `Option` and/or `Result`.");
            info_!("Values of either type can be returned directly from handlers.");
            warn_!("A panic is treated as an internal server error.");
            $e
        }}
    }

    let run = AssertUnwindSafe(run);
    let fut = std::panic::catch_unwind(move || run())
        .map_err(|e| panic_info!(name, e))
        .ok()?;

    AssertUnwindSafe(fut)
        .catch_unwind()
        .await
        .map_err(|e| panic_info!(name, e))
        .ok()
}

impl WebsocketRouter {
    pub fn is_upgrade(&self, hyper_request: &hyper::Request<hyper::Body>) -> bool {
        hyper_request.method() == hyper::Method::GET &&
        Self::header_contains(hyper_request, "Connection", "upgrade") &&
        Self::header_contains(hyper_request, "Upgrade", "websocket")
    }
    fn header_contains(hyper_request: &hyper::Request<hyper::Body>, name: impl AsRef<str>, contains: impl AsRef<str>) -> bool {
        if let Some(value) = hyper_request.headers().get(name.as_ref()) {
            if let Ok(value) = value.to_str() {
                if value.to_lowercase().contains(contains.as_ref()) {
                    return true;
                }
            }
        }
        false
    }
    pub async fn route(rocket: Arc<Rocket<Orbit>>, mut hyper_request: hyper::Request<hyper::Body>, h_addr: SocketAddr, tx: oneshot::Sender<hyper::Response<hyper::Body>>) {
        let upgrade = hyper::upgrade::on(&mut hyper_request);
        let (h_parts, h_body) = hyper_request.into_parts();

        // Convert the Hyper request into a Rocket request.
        let req_res = Request::from_hyp(
            &rocket, h_parts.method, h_parts.headers, &h_parts.uri, h_addr
        );

        let mut req = match req_res {
            Ok(req) => req,
            Err(e) => {
                error!("Bad incoming request: {}", e);
                // TODO: We don't have a request to pass in, so we just
                // fabricate one. This is weird. We should let the user know
                // that we failed to parse a request (by invoking some special
                // handler) instead of doing this.
                let dummy = Request::new(&rocket, rocket_http::Method::Get, Origin::dummy());
                let r = rocket.handle_error(Status::BadRequest, &dummy).await;
                return rocket.send_response(r, tx).await;
            }
        };
        let mut data = Data::from(h_body);

        // Dispatch the request to get a response, then write that response out.
        let _token = rocket.preprocess_request(&mut req, &mut data).await;
        
        let mut response = None;
        let (channel_tx, channel_rx) = mpsc::unbounded_channel::<Message>();
        req.local_cache(|| Some(channel_tx));
        
        for route in rocket.websocket_router.routes_match(&req) {
            req.set_route(route);

            let name = route.name.as_deref();
            let res = handle(name, || route.websocket_handler.handle(&req, Message::Open())).await;
            // Successfully ran
            if let Some(Ok(_)) = res {
                response = Some((match Self::create_reponse(&req) {
                    Ok(res) => res,
                    Err(status) => Self::handle_error(status),
                }, route));
                break;
            }
        }
        if let Some((response, route)) = response {
            rocket.send_response(response, tx).await;
            Self::websocket_task(rocket.clone(), &req, upgrade, route, channel_rx).await;
        }else {
            let response = Self::handle_error(Status::NotFound);
            rocket.send_response(response, tx).await;
        }
    }

    async fn websocket_task(rocket: Arc<Rocket<Orbit>>, request: &Request<'_>, on_upgrade: OnUpgrade, route: &Route, mut channel_rx: mpsc::UnboundedReceiver<Message>) {
        if let Ok(ws) = on_upgrade.await {
            let (mut ws_tx, mut ws_rx) = WebSocketStream::from_raw_socket(ws, Role::Server, None).await.split();
            let name = route.name.as_deref();
            let transmitter = rocket.websocket_router.transmitter.clone();
            let tmp = tokio::spawn(async move {
                while let Some(message) = channel_rx.recv().await {
                    if let Some(message) = message.as_tungstenite() {
                        let _ = ws_tx.send(message).await;
                    }
                }
            });
            while let Some(Ok(raw_message)) = ws_rx.next().await {
                match Message::from_tungstenite(raw_message) {
                    Ok(message) => {
                        let _ = handle(name, || route.websocket_handler.handle(&request, message)).await;
                        //if let Some(Ok(mut stream)) = res {
                            //while let Some(return_message) = stream.next().await {
                                //if let Some(raw_message) = return_message.as_tungstenite() {
                                    //if let Err(_) = ws.send(raw_message).await {
                                        //// We assume the connection closed
                                        //return;
                                    //}
                                //}
                            //}
                        //}
                    }
                    Err(tung_message) => {
                        match tung_message {
                            tokio_tungstenite::tungstenite::Message::Ping(_data) => (),
                            tokio_tungstenite::tungstenite::Message::Pong(_data) => (),
                            _ => unreachable!("Message::from_tungstenite should have accepted this"),
                        }
                    }
                }
            }
            tmp.abort();
        }
    }
    
    /// Turns a Sec-WebSocket-Key into a Sec-WebSocket-Accept.
    fn convert_key(input: &[u8]) -> String {
        use sha1::Digest;
        // ... field is constructed by concatenating /key/ ...
        // ... with the string "258EAFA5-E914-47DA-95CA-C5AB0DC85B11" (RFC 6455)
        const WS_GUID: &[u8] = b"258EAFA5-E914-47DA-95CA-C5AB0DC85B11";
        let mut sha1 = sha1::Sha1::default();
        sha1.update(input);
        sha1.update(WS_GUID);
        base64::encode(sha1.finalize())
    }
    /// Construct a rocket response from the given hyper request
    fn create_reponse<'_b>(request: &Request<'_>, ) -> Result<Response<'_b>, Status> {
        let key = request.headers().get_one("Sec-WebSocket-Key")
            .ok_or(Status::BadRequest)?;
        if request.headers().get_one("Sec-WebSocket-Version").map(|v| v.as_bytes()) != Some(b"13") {
            return Err(Status::BadRequest);
        }
        //let key = hyper_request.headers().get("Sec-WebSocket-Key")
            //.ok_or(Status::BadRequest)?;
        //if hyper_request.headers().get("Sec-WebSocket-Version").map(|v| v.as_bytes()) != Some(b"13") {
            //return Err(Status::BadRequest);
        //}

        let mut response = Response::build();
        response.status(Status::SwitchingProtocols);
        response.header(Header::new(CONNECTION.as_str(), "upgrade"));
        response.header(Header::new(UPGRADE.as_str(), "websocket"));
        response.header(Header::new("Sec-WebSocket-Accept", Self::convert_key(key.as_bytes())));
        response.sized_body(None, Cursor::new("Switching protocols to WebSocket"));
        Ok(response.finalize())
    }

    /// Construct a rocket response from the given hyper request
    fn handle_error<'_b>(status: Status) -> Response<'_b> {
        let mut response = Response::build();
        response.status(status);
        response.sized_body(None, Cursor::new("Upgrade failed"));
        response.finalize()
    }
}
