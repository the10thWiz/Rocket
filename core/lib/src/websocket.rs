use std::{collections::HashMap, io::Cursor, sync::Arc};
use std::pin::Pin;

use futures::{Future, SinkExt, StreamExt};
use rocket_http::{Header, Status, hyper::{header::{CONNECTION, UPGRADE}, upgrade::Upgraded}};
use async_trait::async_trait;
use unique_id::{Generator, random::RandomGenerator};

use crate::{Request, Response, request::{FromRequest, Outcome}, response::{self, Responder, upgrade::UpgradeResponder}};
use tokio::{select, sync::{mpsc, oneshot}};
use tokio_tungstenite::{WebSocketStream, tungstenite::protocol::{Role, WebSocketConfig}};

#[derive(Debug)]
pub enum WebSocketError {
    StreamEnd,
    Tungstenite(tokio_tungstenite::tungstenite::Error)
}

type Result<T> = std::result::Result<T, WebSocketError>;

#[derive(PartialEq, Eq, Debug, Copy, Clone, Hash)]
pub struct WebsocketId(u128);

pub struct WebSocketConnection(WebSocketStream<Upgraded>, WebsocketId);

impl WebSocketConnection {
    fn into_inner(self) -> (WebSocketStream<Upgraded>, WebsocketId) {
        (self.0, self.1)
    }
}

/// Analogous enum to tungstenite::protocol::Message,
///
/// but Ping, Pong and Close should be handled mostly automatically.
#[derive(Debug, Clone)]
pub enum Message {
    Text(String),
    Binary(Vec<u8>),
    Close(),
}

impl Message {
    pub fn as_tungstenite(self) -> tokio_tungstenite::tungstenite::Message {
        match self {
            Self::Text(s) => tokio_tungstenite::tungstenite::Message::Text(s),
            Self::Binary(v) => tokio_tungstenite::tungstenite::Message::Binary(v),
            Self::Close() => tokio_tungstenite::tungstenite::Message::Close(None),
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

/// Internal upgrade mechanism
pub async fn accept() -> (oneshot::Receiver<Pin<Box<WebSocketConnection>>>, WebsocketResponder)
{
    //let socket = data.into_hyper_body().on_upgrade().await;
    //let hyper_body = upgrade::on(data.into_hyper_body().ok_or(Status::UpgradeRequired)?);
    let (tx, rx) = oneshot::channel();
    (rx, WebsocketResponder {
        inner: tx, config: None, id: None,
    })
}

pub struct WebsocketResponder {
    inner: oneshot::Sender<Pin<Box<WebSocketConnection>>>,
    config: Option<WebSocketConfig>,
    id: Option<WebsocketId>,
}

impl<'r, 'o: 'r> Responder<'r, 'o> for WebsocketResponder {
    fn respond_to(mut self, request: &'r Request<'_>) -> response::Result<'o> {
        let key = request.headers().get_one("Sec-WebSocket-Key")
            .ok_or(Status::BadRequest)?;
        if request.headers().get_one("Sec-WebSocket-Version").map(|v| v.as_bytes()) != Some(b"13") {
            return Err(Status::BadRequest);
        }
        self.id = Some(*request.local_cache(|| WebsocketId(0)));

        let mut response = Response::build();
        response.status(Status::SwitchingProtocols);
        response.header(Header::new(CONNECTION.as_str(), "upgrade"));
        response.header(Header::new(UPGRADE.as_str(), "websocket"));
        response.header(Header::new("Sec-WebSocket-Accept", convert_key(key.as_bytes())));
        response.sized_body(None, Cursor::new("Switching protocols to WebSocket"));

        response.upgrade(Box::new(self));

        Ok(response.finalize())
    }
}

#[async_trait]
impl UpgradeResponder for WebsocketResponder {
    async fn on_upgrade(mut self: Box<Self>, upgrade_obj: Upgraded) -> std::io::Result<()> {
	let stream = WebSocketStream::from_raw_socket(upgrade_obj, Role::Server, self.config.take()).await;
	self.inner.send(Box::pin(WebSocketConnection(stream, self.id.expect("No id set")))).map_err(|_|std::io::Error::new(std::io::ErrorKind::Other, "Upgrade reciever hung up"))
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

#[derive(Clone)]
pub struct Websocket {
    router: WebsocketRouter,
    id: WebsocketId,
}

impl Websocket {
    pub async fn send(&self, message: Message) {
        self.router.send(message, self.id);
    }
}

#[async_trait]
impl<'r> FromRequest<'r> for Websocket {
    type Error = ();
    async fn from_request(request: &'r Request<'_>) -> Outcome<Self, Self::Error> {
        let router = request.rocket().state::<WebsocketRouter>().expect("WebsocketRouter is not being managed").clone();
        let id = *request.local_cache(|| router.get_next_id());

        Outcome::Success(Self {
            router, id,
        })
    }
}

pub type Handler = Pin<Box<dyn Fn(WebsocketReciever, &mpsc::UnboundedSender<WebsocketUpdate>) -> Pin<Box<dyn Future<Output = ()> + Send>> + Send + Sync>>;

pub enum WebsocketUpdate {
    Registering(oneshot::Receiver<Pin<Box<WebSocketConnection>>>, Handler),
    Registered(WebsocketId, mpsc::UnboundedSender<Message>),
    Unregister(WebsocketId),
    Message(Message, WebsocketId),
    MessageMany(Message, ()),
}

pub struct WebsocketReciever {
    message_rx: mpsc::UnboundedReceiver<Message>,
    ws: WebSocketStream<Upgraded>,
}

pub enum MessageGen {
    FromClient(Message),
    ToClient(Message),
}

impl WebsocketReciever {
    pub async fn next(&mut self) -> Result<Message> {
        loop {
            select! {
                m = self.message_rx.recv() => {
                    if let Some(m) = m {
                        let _ = self.ws.send(m.as_tungstenite()).await;
                    }
                },
                m = self.ws.next() => {
                    match m {
                        Some(Ok(m)) => {
                            match Message::from_tungstenite(m) {
                                Ok(m) => return Ok(m),
                                Err(m) => println!("Unknown message {:?}", m),
                            }
                        },
                        None => return Err(WebSocketError::StreamEnd),
                        Some(Err(e)) => return Err(WebSocketError::Tungstenite(e)),
                    }
                }
            }
        }
    }
}

#[derive(Clone, Debug)]
pub(crate) struct WebsocketRouter {
    transmitter: mpsc::UnboundedSender<WebsocketUpdate>,
    ids: Arc<RandomGenerator>,
}


impl WebsocketRouter {
    pub(crate) fn new() -> Self {
        let (transmitter, mut rx) = mpsc::unbounded_channel();
        let main_tx = transmitter.clone();
        tokio::spawn(async move {
            let mut clients = HashMap::new();
            while let Some(message) = rx.recv().await {
                match message {
                    WebsocketUpdate::Registering(ws_res, handler) => {
                        let tx = main_tx.clone();
                        let (message_tx, message_rx) = mpsc::unbounded_channel();
                        tokio::spawn(async move {
                            let (ws, id) = match ws_res.await {
                                Ok(v) => Pin::into_inner(v).into_inner(),
                                Err(_e) => {
                                    println!("Failed to get inner ws: {:?}", _e);
                                    return;},
                            };
                            println!("Sending: {:?}", id);
                            let _ = tx.send(WebsocketUpdate::Registered(id, message_tx));
                            let _ = handler(WebsocketReciever{ws, message_rx}, &tx).await;
                            //loop {
                                //select! {
                                    //m = message_rx.recv() => if let Some(m) = m {
                                        //let _ = ws.send(m.as_tungstenite()).await;
                                    //}else {
                                        //break;
                                    //},
                                    //m = ws.next() => if let Some(Ok(m)) = m {
                                        //match Message::from_tungstenite(m) {
                                            //Ok(m) => {
                                                //let _ = handler(m).await;
                                            //},
                                            //Err(_m) => (),
                                        //}
                                    //}else {
                                        //break;
                                    //},
                                //}
                            //}
                            let _ = tx.send(WebsocketUpdate::Unregister(id));
                        });
                    },
                    WebsocketUpdate::Registered(id, message_tx) => {
                        println!("Register: {:?}", id);
                        clients.insert(id, message_tx);},
                    WebsocketUpdate::Unregister(id) => {clients.remove(&id);},
                    WebsocketUpdate::Message(message, id) => {
                        if let Some(tx) = clients.get(&id) {
                            if let Err(_) = tx.send(message) {
                                clients.remove(&id);
                            }
                        }
                    },
                    _ => (),
                }
            }
        });
        Self {
            transmitter,
            ids: Arc::new(RandomGenerator::default()),
        }
    }
    pub async fn register_websocket_handler(&self, rx: oneshot::Receiver<Pin<Box<WebSocketConnection>>>, handler: Handler) {
        let _res = self.transmitter.send(WebsocketUpdate::Registering(rx, handler));
    }
    fn get_next_id(&self) -> WebsocketId  {
        WebsocketId(self.ids.next_id())
    }
    /// Send a message to a specific websocket
    fn send(&self, message: Message, target: WebsocketId) {
        let _ = self.transmitter.send(WebsocketUpdate::Message(message, target));
    }
}
