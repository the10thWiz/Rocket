use std::{convert::TryInto, pin::Pin, sync::Arc, task::Poll};

use bytes::{BufMut, Bytes, BytesMut};
use futures::{Future, FutureExt, SinkExt, StreamExt, future::{pending, poll_fn}};
use rocket_http::{Status, hyper::upgrade::{Parts, Upgraded}};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWriteExt};
use tokio::{net::TcpStream, select, sync::{Mutex, mpsc, oneshot}};
use tokio_util::codec::{Decoder, Encoder, Framed, FramedParts};
use ubyte::ByteUnit;
use websocket_codec::{Error, Message, MessageCodec, Opcode};
use websocket_codec::protocol::{DataLength, FrameHeader, FrameHeaderCodec};

use crate::{Data, Request, request::{FromRequest, Outcome}};

/// A trait for types that can be sent on a websocket.
///
/// This has default implementations for many common types, such as `String`, `Vec<u8>`, etc
///
/// # Text vs Binary
/// The Websocket protocol requires Rocket to specify whether a message is text or binary. Rocket
/// implements this automatically where possible, but it's Rocket has not way to detect whether a
/// given message is binary or text solely based on the binary output. Most types will always turn
/// into binary or text, but it is possible for a type to be either text or binary depending on the
/// contents.
///
/// TODO: After contrib-graduation, implement `IntoMessage` on `Json`
pub trait IntoMessage {
    fn is_binary(&self) -> bool;
    fn into_message(self) -> mpsc::Receiver<Bytes>;
}

impl IntoMessage for Data {
    fn is_binary(&self) -> bool {
        true
    }
    fn into_message(mut self) -> mpsc::Receiver<Bytes> {
        self.open(ByteUnit::Byte(1024)).into_message()
    }
}

impl<T: AsyncRead + Send + Unpin + 'static> IntoMessage for T {
    fn is_binary(&self) -> bool {
        true
    }
    fn into_message(mut self) -> mpsc::Receiver<Bytes> {
        let (tx, rx) = mpsc::channel(1);
        tokio::spawn(async move {
            let mut buf = BytesMut::with_capacity(100);
            while let Ok(n) = self.read_buf(&mut buf).await {
                if n == 0 {
                    break;
                }
                let tmp = buf.split();
                let e = tx.send(tmp.into()).await;
            }
        });
        rx
    }
}


/// Convience function to convert an `impl IntoMessage` into a `Message`
pub(crate) fn to_message(message: impl IntoMessage) -> WebsocketMessage {
    WebsocketMessage::new(message.is_binary(), message.into_message())
}

#[derive(Debug)]
pub struct WebsocketMessage {
    header: FrameHeader,
    data: mpsc::Receiver<Bytes>,
}

impl WebsocketMessage {
    pub fn new(binary: bool, data: mpsc::Receiver<Bytes>) -> Self {
        Self {
            header: FrameHeader::new(false, 0, if binary {
                    Opcode::Binary.into()
                }else{
                    Opcode::Text.into()
                }, None, 0usize.into()),
            data,
        }
    }

    fn close(status: Option<Status>) -> Self {
            //Message::close(Some((status.code, status.reason().unwrap_or("").to_string())))
        let (tx, data) = mpsc::channel(1);
        if let Some(status) = status {
            let e = tx.try_send(status.to_string().into());
        }
        Self {
            header: FrameHeader::new(true, 0, Opcode::Close.into(), None, 0usize.into()),
            data
        }
    }

    pub(crate) fn opcode(&self) -> Opcode {
        Opcode::try_from(self.header.opcode()).unwrap_or(Opcode::Text)
    }
    pub(crate) fn into_bytes(self) -> mpsc::Receiver<Bytes> {
        self.data
    }
}

/// A Websocket connection, directly connected to a client.
///
/// Messages sent with the `send` method are only sent to one client, the one who sent the message.
/// This is also nessecary for subscribing clients to specific channels.
pub struct WebsocketChannel {
    //inner: Arc<Mutex<Option<InnerChannel>>>,
    inner: mpsc::Receiver<WebsocketMessage>,
    //channels: Option<()>,
    //reciever: Arc<Mutex<mpsc::UnboundedReceiver<Message>>>,
    sender: mpsc::Sender<WebsocketMessage>,
    //upgrade_tx: oneshot::Sender<Upgraded>
}

/// Soft maximum buffer size
const MAX_BUFFER_SIZE: usize = 1024;

impl WebsocketChannel {
    pub(crate) fn new() -> (Self, oneshot::Sender<Upgraded>) {
        let (broker_tx, broker_rx) = mpsc::channel(50);
        let (upgrade_tx, upgrade_rx) = oneshot::channel();
        let (message_tx, message_rx) = mpsc::channel(1);
        tokio::spawn(Self::message_handler(upgrade_rx, broker_rx, message_tx));
        (Self {
                inner: message_rx,
                //channels: Some(()),
                //reciever: Arc::new(Mutex::new(reciever)),
                sender: broker_tx,
        }, upgrade_tx)
    }

    async fn message_handler(upgrade_rx: oneshot::Receiver<Upgraded>,
                             mut broker_rx: mpsc::Receiver<WebsocketMessage>,
                             message_tx: mpsc::Sender<WebsocketMessage>) {
        // Get upgrade object (basically just a boxed handle to the tcp or tls stream)
        if let Ok(mut upgrade) = upgrade_rx.await {
            // build codec
            let tmp = websocket_codec::protocol::FrameHeaderCodec;
            let mut raw_ws = tmp.framed(upgrade).into_parts();

            let (mut data_tx, data_rx) = mpsc::channel(1);
            let mut data_rx = Some(data_rx);

            let mut outgoing_message: Option<WebsocketMessage> = None;
            loop {
                let broker_ready = outgoing_message.is_none();
                select! {
                    message = Self::read_header(&mut raw_ws) => {
                        println!("Raw message: {:?}", message);
                        if let Some(Ok(header)) = message {
                            let mask = header.mask().map(|u| u32::from(u).to_le_bytes());
                            // TODO avoid unwrap -> I think this should always succeed,
                            // although it might fail on 32 bit platforms or something.
                            let mut remaining = header.data_len().try_into().unwrap();
                            let fin = header.fin();
                            // Don't send continue frames
                            if let Some(data) = data_rx.take() {
                                message_tx.send(WebsocketMessage {
                                    header, data,
                                }).await;
                            }else if header.opcode() == 0x01 {
                                // TODO: handle error
                            }
                            let mut cur: usize = 0;
                            // Message contains ready bytes. For short messages,
                            // this might be the entire message
                            while remaining > 0 {
                                let mut message = raw_ws.read_buf.split_to(
                                    raw_ws.read_buf.len().min(remaining)
                                );
                                // TODO unmask message
                                remaining-= message.len();
                                if let Some(mask) = mask {
                                    for b in message.iter_mut() {
                                        *b ^= mask[cur];
                                        cur = (cur + 1) % 4;
                                    }
                                }
                                if let Err(b) = data_tx.send(message.into()).await {
                                    // TODO handle error
                                }

                                if remaining > 0 {
                        // The check shouldn't matter, but I think there are edge cases where
                        // attempting to read could be bad. In theory, I would expect the capacity
                        // of the read_buf to be zero, since we just took it all, and reserve(0)
                        // should be a noop - but I don't know if it is. If reserve(0) is a noop,
                        // the check should be unnessecary
                                    raw_ws.read_buf.reserve(remaining.min(MAX_BUFFER_SIZE));
                                    raw_ws.io.read_buf(&mut raw_ws.read_buf).await;
                                }
                            }
                            // If this is the final frame, reset data_tx and data_rx
                            if fin {
                                let (ndata_tx, ndata_rx) = mpsc::channel(1);
                                data_tx = ndata_tx;
                                data_rx = Some(ndata_rx);
                            }
                        }else {
                            break;
                        }
                    }
                    message = async {
                        if broker_ready {
                            broker_rx.recv().await
                        } else {
                            pending().await
                        }
                    } => {
                        if let Some(message) = message {
                            outgoing_message = Some(message);
                        }else {
                            // TODO handle error
                        }
                    }
                    data = async {
                        if let Some(data_rx) = &mut outgoing_message {
                            data_rx.data.recv().await
                        } else {
                            pending().await
                        }
                    } => {
                        if let Some(data) = data {
                            if let Some(message) = outgoing_message.take() {
                                let int_header = FrameHeader::new(false,
                                                                  message.header.rsv(),
                                                                  message.header.opcode(),
                                                                  message.header.mask(),
                                                                  data.len().into());
                                let e = raw_ws.codec.encode(int_header, &mut raw_ws.write_buf);
                                let e = raw_ws.io.write_all_buf(&mut raw_ws.write_buf).await;
                                let e = raw_ws.io.write_all(&data).await;
                                outgoing_message = Some(WebsocketMessage {
                                    // Next message will be a continue frame
                                    header: FrameHeader::new(false,
                                                             message.header.rsv(),
                                                             0x0,
                                                             message.header.mask(),
                                                             0usize.into()),
                                    data: message.data,
                                });
                            }
                        }else {
                            // TODO fid potential fix for sending zero size frame, maybe wrapping
                            // the Bytes in an enum to indicate if we are done? This should work
                            // though
                            if let Some(message) = outgoing_message.take() {
                                let int_header = FrameHeader::new(true,
                                                                  message.header.rsv(),
                                                                  message.header.opcode(),
                                                                  message.header.mask(),
                                                                  0usize.into());
                                let e = raw_ws.codec.encode(int_header, &mut raw_ws.write_buf);
                                let e = raw_ws.io.write_all_buf(&mut raw_ws.write_buf).await;
                            }
                        }
                    }
                }
            }
        }
    }

    async fn read_header(raw_ws: &mut FramedParts<Upgraded, FrameHeaderCodec>)
        -> Option<Result<FrameHeader, websocket_codec::Error>>
    {
        loop {
            match raw_ws.codec.decode(&mut raw_ws.read_buf) {
                Ok(Some(header)) => return Some(Ok(header)),
                Ok(None) => {
                    raw_ws.read_buf.reserve(1);
                    match raw_ws.io.read_buf(&mut raw_ws.read_buf).await {
                        //Ok(0) => return None,
                        Err(e) => return Some(Err(Box::new(e))),
                        _ => (),
                    }
                },
                Err(e) => return Some(Err(e)),
            }
        }
    }

    /// Gets the handle to subscribe this channel to a descriptor
    pub(crate) fn subscribe_handle(&self) -> mpsc::Sender<WebsocketMessage> {
        self.sender.clone()
    }

    //pub(crate) fn upgraded(&self, upgrade: Upgraded) {
        //self.upgrade_tx.send(upgrade);
    //}

    /// Get the next message from this client.
    ///
    /// This method also forwards messages sent from any channels the client is subscribed to
    pub(crate) async fn next(&mut self) -> Option<WebsocketMessage> {
        self.inner.recv().await
    }
}

#[derive(Clone)]
pub struct Channel(mpsc::Sender<WebsocketMessage>);

impl Channel {
    pub(crate) fn from_websocket(chan: &WebsocketChannel) -> Self {
        Self(chan.subscribe_handle())
    }

    /// Sends a raw Message to the client
    pub(crate) async fn send_raw(&self, message: WebsocketMessage) {
        let _ = self.0.send(message).await;
    }

    /// Send a message to the specific client connected to this websocket
    pub async fn send(&self, message: impl IntoMessage) {
        self.send_raw(to_message(message)).await
    }

    /// Sends a close notificaiton to the client, so no new messages will arive
    pub async fn close(&self) {
        self.send_raw(WebsocketMessage::close(None)).await
    }

    /// Sends a close notificaiton to the client, along with a reason for the close
    pub async fn close_with_status(&self, status: Status) {
        self.send_raw(
            WebsocketMessage::close(Some(status))
        ).await
    }
}

#[crate::async_trait]
impl<'r> FromRequest<'r> for Channel {
    type Error = &'static str;
    async fn from_request(request: &'r Request<'_>) -> Outcome<Self, Self::Error> {
        let tmp = request.local_cache(|| None).clone();
        if let Some(tmp) = tmp {
            Outcome::Success(tmp)
        }else {
            Outcome::Failure((Status::InternalServerError, "Websockets not initialized"))
        }
    }
}
