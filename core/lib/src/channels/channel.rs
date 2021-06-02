use std::borrow::Cow;
use std::convert::TryInto;
use std::io;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::time::Duration;

use bytes::{Bytes, BytesMut};
use futures::future::pending;
use rocket_http::uri::Origin;
use rocket_http::{Status, hyper::upgrade::Upgraded};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWriteExt};
use tokio::{select, sync::{mpsc, oneshot}};
use tokio_util::codec::{Decoder, Encoder, FramedParts};
use ubyte::ByteUnit;
use websocket_codec::Opcode;
use websocket_codec::protocol::{FrameHeader, FrameHeaderCodec};

use crate::{Data, Request, request::{FromRequest, Outcome}};

use super::broker::Broker;

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
// TODO: implement `IntoMessage` on `Json` and other convience types
pub trait IntoMessage {
    fn is_binary(&self) -> bool;
    fn into_message(self) -> mpsc::Receiver<Bytes>;
}

impl IntoMessage for Data {
    fn is_binary(&self) -> bool {
        self.websocket_is_binary().unwrap_or(true)
    }

    fn into_message(self) -> mpsc::Receiver<Bytes> {
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
                let _e = tx.send(tmp.into()).await;
            }
        });
        rx
    }
}

// Compliler error, since AsyncRead could be implemented on String in future versions
//impl IntoMessage for String {
    //fn is_binary(&self) -> bool {
        //false
    //}

    //fn into_message(self) -> mpsc::Receiver<Bytes> {
        //unimplemented!()
    //}
//}


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
        let (tx, data) = mpsc::channel(1);
        if let Some(status) = status {
            let _e = tx.try_send(status.to_string().into());
        }
        Self {
            header: FrameHeader::new(true, 0, Opcode::Close.into(), None, 0usize.into()),
            data
        }
    }

    pub(crate) fn opcode(&self) -> Opcode {
        Opcode::try_from(self.header.opcode()).unwrap_or(Opcode::Text)
    }

    pub(crate) fn into_parts(self) -> (FrameHeader, mpsc::Receiver<Bytes>) {
        (self.header, self.data)
    }

    pub(crate) fn from_parts(header: FrameHeader, data: mpsc::Receiver<Bytes>) -> Self {
        Self { header, data, }
    }
}

impl IntoMessage for WebsocketMessage {
    fn is_binary(&self) -> bool {
        match Opcode::try_from(self.header.opcode()) {
            Some(Opcode::Text) => false,
            _ => true,
        }
    }

    fn into_message(self) -> mpsc::Receiver<Bytes> {
        self.data
    }
}

/// A Websocket connection, directly connected to a client.
///
/// Messages sent with the `send` method are only sent to one client, the one who sent the message.
/// This is also nessecary for subscribing clients to specific channels.
pub struct WebsocketChannel {
    inner: mpsc::Receiver<WebsocketMessage>,
    sender: mpsc::Sender<WebsocketMessage>,
    should_send_close: Arc<AtomicBool>,
}

/// Soft maximum buffer size
const MAX_BUFFER_SIZE: usize = 1024;

struct RunningMessage {
    current: BytesMut,
    remaining: usize,
    cur: usize,
    mask: [u8; 4],
    fin: bool,
}

impl RunningMessage {
    fn handle_xor(&mut self) {
        for b in self.current.iter_mut() {
            *b ^= self.mask[self.cur];
            self.cur = (self.cur + 1) % 4;
        }
    }

    fn xored(mut self) -> Self {
        self.handle_xor();
        self
    }
}

impl WebsocketChannel {
    pub(crate) fn new() -> (Self, oneshot::Sender<Upgraded>) {
        let (broker_tx, broker_rx) = mpsc::channel(50);
        let (upgrade_tx, upgrade_rx) = oneshot::channel();
        let (message_tx, message_rx) = mpsc::channel(1);
        let should_send_close = Arc::new(AtomicBool::new(true));
        tokio::spawn(Self::message_handler(upgrade_rx, broker_rx, message_tx, should_send_close.clone()));
        (Self {
                inner: message_rx,
                sender: broker_tx,
                should_send_close,
        }, upgrade_tx)
    }

    /// Gets the handle to subscribe this channel to a descriptor
    pub(crate) fn subscribe_handle(&self) -> mpsc::Sender<WebsocketMessage> {
        self.sender.clone()
    }

    /// Get the next message from this client.
    ///
    /// This method also forwards messages sent from any channels the client is subscribed to
    pub(crate) async fn next(&mut self) -> Option<WebsocketMessage> {
        self.inner.recv().await
    }

    /// Gets the should_send_close flag. This is set to false if we have already sent a close
    /// message
    pub(crate) fn should_send_close(&self) -> bool {
        self.should_send_close.load(atomic::Ordering::Acquire)
    }

    async fn message_handler(
        upgrade_rx: oneshot::Receiver<Upgraded>,
        mut broker_rx: mpsc::Receiver<WebsocketMessage>,
        message_tx: mpsc::Sender<WebsocketMessage>,
        should_send_close: Arc<AtomicBool>,
    ) {
        // Get upgrade object (basically just a boxed handle to the tcp or tls stream)
        if let Ok(upgrade) = upgrade_rx.await {
            // build codec
            let tmp = websocket_codec::protocol::FrameHeaderCodec;
            let mut raw_ws = tmp.framed(upgrade).into_parts();

            let (mut data_tx, data_rx) = mpsc::channel(1);
            let mut data_rx = Some(data_rx);

            let mut outgoing_message: Option<WebsocketMessage> = None;
            let mut running_message: Option<RunningMessage> = None;
            loop {
                let broker_ready = outgoing_message.is_none();
                let next_message = running_message.is_none();
                //println!("Sending Ready: {}", broker_ready);
                //println!("Recv Ready: {}", next_message);
                select! {
                    message = async {
                        if next_message {
                            Self::read_header(&mut raw_ws).await
                        }else {
                            pending().await
                        }
                    } => {
                        if let Some(Ok(header)) = message {
                            //println!("Recv: {:?}", header);
                            Self::send_message(
                                header,
                                &mut raw_ws,
                                &message_tx,
                                &mut data_tx,
                                &mut data_rx,
                                &mut running_message
                            ).await;
                        }else {
                            // TODO handle close
                            //println!("Closing ws");
                            break;
                        }
                    }
                    _ = async {
                        if let Some(running) = &mut running_message {
                            Self::continue_message(running, &data_tx).await
                        } else {
                            pending().await
                        }
                    } => {
                        let _e = Self::read_next_part(
                                &mut running_message,
                                &mut raw_ws,
                                &mut data_tx,
                                &mut data_rx,
                            ).await;
                    }
                    message = async {
                        if broker_ready {
                            broker_rx.recv().await
                        } else {
                            pending().await
                        }
                    } => {
                        if let Some(message) = message {
                            if message.header.opcode() == u8::from(Opcode::Close) {
                                should_send_close.store(false, atomic::Ordering::Release);
                            }
                            //println!("Send: {:?}", message);
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
                            //println!("Forwarding {} bytes of data", data.len());
                            if let Some(message) = outgoing_message.take() {
                                let mut data = vec![data];
                                let mut fin = false;
                                if let Some(data_rx) = &mut outgoing_message {
                                    loop {
                                        // We only wait 10ms for each chunk to be sent
                                        let tmp = tokio::time::timeout(Duration::from_millis(10), data_rx.data.recv()).await;
                                        if let Ok(Some(next)) = tmp {
                                            data.push(next);
                                        } else if let Ok(None) = tmp {
                                            fin = true;
                                            break;
                                        } else {
                                            break;
                                        }
                                    }
                                }
                                let int_header = FrameHeader::new(fin,
                                                                  message.header.rsv(),
                                                                  message.header.opcode(),
                                                                  message.header.mask(),
                                                                  data.iter().map(|s| s.len()).sum::<usize>().into());
                                let _e = raw_ws.codec.encode(int_header, &mut raw_ws.write_buf);
                                let _e = raw_ws.io.write_all_buf(&mut raw_ws.write_buf).await;
                                for data in data {
                                    let _e = raw_ws.io.write_all(&data).await;
                                }
                                if !fin {
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
                            }
                        } else {
                            // This will sometimes send an empty frame to end a message. However,
                            // if the sending half of the data channel is dropped immediately after
                            // the final block was sent, this might be avoided
                            //println!("Compeleting message");
                            if let Some(message) = outgoing_message.take() {
                                let int_header = FrameHeader::new(true,
                                                                  message.header.rsv(),
                                                                  message.header.opcode(),
                                                                  message.header.mask(),
                                                                  0usize.into());
                                let _e = raw_ws.codec.encode(int_header, &mut raw_ws.write_buf);
                                let _e = raw_ws.io.write_all_buf(&mut raw_ws.write_buf).await;
                            }
                        }
                    }
                }
            }
            //println!("Loop completed");
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

    async fn send_message(
        header: FrameHeader,
        raw_ws: &mut FramedParts<Upgraded, FrameHeaderCodec>,
        message_tx: &mpsc::Sender<WebsocketMessage>,
        _data_tx: &mut mpsc::Sender<Bytes>,
        data_rx: &mut Option<mpsc::Receiver<Bytes>>,
        running_message: &mut Option<RunningMessage>,
    ) {
        let mask = header.mask().map(|u| u32::from(u).to_le_bytes());
        // TODO avoid unwrap -> I think this should always succeed,
        // although it might fail on 32 bit platforms or something.
        let remaining = header.data_len().try_into().unwrap();
        let fin = header.fin();
        // Don't send continue frames
        if let Some(data) = data_rx.take() {
            let _e = message_tx.send(WebsocketMessage {
                header, data,
            }).await;
        }else if header.opcode() == 0x01 {
            // TODO: handle error
        }
        *running_message = Some(
            RunningMessage {
                current: raw_ws.read_buf.split_to(
                    raw_ws.read_buf.len().min(remaining)
                ),
                remaining,
                cur: 0,
                mask: mask.unwrap_or([0; 4]),
                fin,
            }.xored()
        );
    }

    async fn continue_message(
        running_message: &mut RunningMessage,
        data_tx: &mpsc::Sender<Bytes>,
    ) {
        let len = running_message.current.len();
        //println!("{} Bytes of {} available to send", len, running_message.remaining);
        if running_message.current.len() > 0 {
            // Record how much will be sent
            let _e = data_tx.send(running_message.current.clone().into()).await;
            // Discard sent bytes
            running_message.remaining-= running_message.current.len();
            let _ = running_message.current.split();
            if let Err(e) = _e {
                //println!("Reciever hung up");
            }
        }
    }

    async fn read_next_part(
        running_message: &mut Option<RunningMessage>,
        raw_ws: &mut FramedParts<Upgraded, FrameHeaderCodec>,
        data_tx: &mut mpsc::Sender<Bytes>,
        data_rx: &mut Option<mpsc::Receiver<Bytes>>,
    ) -> io::Result<()> {
        if let Some(running) = running_message {
            if running.remaining > 0 {
                // Reserve more space
                raw_ws.read_buf.reserve(running.remaining.min(MAX_BUFFER_SIZE));
                let tmp = raw_ws.read_buf.len();
                //println!("Reading with {} bytes in buffer", tmp);
                let len = raw_ws.io.read_buf(&mut raw_ws.read_buf).await?;
                //println!("Read {} bytes, advanced buf by {}", len, raw_ws.read_buf.len() - tmp);
                running.current = raw_ws.read_buf.split_to(
                    raw_ws.read_buf.len().min(running.remaining)
                );
                running.handle_xor();
                //println!("Current is {} bytes", running.current.len());
            }else {
                // If the current frame is final, reset the data params
                if running.fin {
                    let (ndata_tx, ndata_rx) = mpsc::channel(1);
                    *data_tx = ndata_tx;
                    *data_rx = Some(ndata_rx);
                }
                *running_message = None;
            }
        }
        Ok(())
    }
}

#[derive(Clone)]
pub(crate) struct InnerChannel {
    client: mpsc::Sender<WebsocketMessage>,
    broker: Broker,
}

impl InnerChannel {
    pub(crate) fn from_websocket(chan: &WebsocketChannel, broker: &Broker) -> Self {
        Self{
            client: chan.subscribe_handle(),
            broker: broker.clone(),
        }
    }
}

pub struct Channel<'r> {
    client: mpsc::Sender<WebsocketMessage>,
    broker: Broker,
    uri: &'r Origin<'r>
}

impl<'r> Channel<'r> {
    fn from(inner: InnerChannel, uri: &'r Origin<'r>) -> Self {
        Self {
            client: inner.client,
            broker: inner.broker,
            uri,
        }
    }

    /// Sends a raw Message to the client
    pub(crate) async fn send_raw(&self, message: WebsocketMessage) {
        let _e = self.client.send(message).await;
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

    pub async fn broadcast(&self, message: impl IntoMessage) {
        println!("Broadcasting");
        self.broker.send(&self.uri, to_message(message));
    }
}

#[crate::async_trait]
impl<'r> FromRequest<'r> for Channel<'r> {
    type Error = &'static str;
    async fn from_request(request: &'r Request<'_>) -> Outcome<Self, Self::Error> {
        let tmp: Option<InnerChannel> = request.local_cache(|| None).clone();
        if let Some(tmp) = tmp {
            Outcome::Success(Self::from(tmp, request.uri()))
        }else {
            Outcome::Failure((Status::InternalServerError, "Websockets not initialized"))
        }
    }
}
