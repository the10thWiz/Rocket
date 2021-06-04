use std::convert::TryFrom;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::time::Duration;

use bytes::Bytes;
use bytes::BytesMut;
use rocket_http::uri::Origin;
use rocket_http::hyper::upgrade::Upgraded;
use tokio::io::AsyncWrite;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::join;
use tokio::select;
use tokio::time::timeout;
use tokio::sync::{mpsc, oneshot};
use tokio_util::codec::{Decoder, Encoder};
use websocket_codec::Opcode;
use websocket_codec::protocol::FrameHeader;
use websocket_codec::protocol::FrameHeaderCodec;

use crate::channels::MAX_BUFFER_SIZE;
use crate::channels::WebsocketMessage;
use crate::request::Outcome;

use crate::log::*;

use super::FromWebsocket;
use super::IntoMessage;
use super::Websocket;
use super::WebsocketStatus;
use super::broker::Broker;
use super::to_message;

/// A Websocket connection, directly connected to a client.
///
/// Messages sent with the `send` method are only sent to one client, the one who sent the message.
/// This is also nessecary for subscribing clients to specific channels.
pub struct WebsocketChannel {
    inner: mpsc::Receiver<WebsocketMessage>,
    sender: mpsc::Sender<WebsocketMessage>,
    close_sent: Arc<AtomicBool>,
}

impl WebsocketChannel {
    pub(crate) fn new() -> (Self, oneshot::Sender<Upgraded>) {
        let (broker_tx, broker_rx) = mpsc::channel(50);
        let (upgrade_tx, upgrade_rx) = oneshot::channel();
        let (message_tx, message_rx) = mpsc::channel(1);
        let should_send_close = Arc::new(AtomicBool::new(false));
        tokio::spawn(Self::message_handler(
                upgrade_rx,
                broker_tx.clone(),
                broker_rx,
                message_tx,
                should_send_close.clone()
            ));
        (Self {
                inner: message_rx,
                sender: broker_tx,
                close_sent: should_send_close,
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

    /// Gets the close_sent flag. This function returns true iff we have not sent a close message
    /// yet, i.e. we should send a close message.
    pub(crate) fn should_send_close(&self) -> bool {
        !self.close_sent.load(atomic::Ordering::Acquire)
    }

    fn protocol_error(header: &FrameHeader, remaining: usize) -> bool {
        if (header.opcode() == u8::from(Opcode::Close) ||
            header.opcode() == u8::from(Opcode::Ping) ||
            header.opcode() == u8::from(Opcode::Pong)) &&
            (remaining > 125 || !header.fin())
        {
            true
        } else if header.rsv() != 0x0 {
            true
        } else if Opcode::try_from(header.opcode()).is_none() && header.opcode() != 0x0 {
            true
        } else {
            false
        }
    }

    pub(crate) async fn close(broker_tx: &mpsc::Sender<WebsocketMessage>, status: WebsocketStatus<'_>) {
        let (tx, rx) = mpsc::channel(1);
        let header = FrameHeader::new(
            true,
            0x0,
            Opcode::Close.into(),
            None,
            0usize.into(),
        );
        //info_!("Close frame header: {:?}", header);
        let _e = broker_tx.send(WebsocketMessage::from_parts(header, rx)).await;
        let _e = tx.send(status.encode()).await;
    }

    async fn message_handler(
        upgrade_rx: oneshot::Receiver<Upgraded>,
        broker_tx: mpsc::Sender<WebsocketMessage>,
        mut broker_rx: mpsc::Receiver<WebsocketMessage>,
        message_tx: mpsc::Sender<WebsocketMessage>,
        close_sent: Arc<AtomicBool>,
    ) {
        if let Ok(upgrade) = upgrade_rx.await {
            // Split creates a mutex to syncronize access to the underlying io stream
            // This should be okay, since the lock shouldn't be in high contention, although it
            // make cause issues with trying to send and recieve messages at the same time.
            let (read, write) = tokio::io::split(upgrade);
            // The reader & writer only hold the lock while polling - so there should be NO
            // contention at all. This is important, since I believe the mutex uses a spin-lock (at
            // least in the version I looked at). Since the writer and reader are executed
            // concurrently, but not in parallel (by the join! macro), they should never be polled
            // at the same time.
            let (ping_tx, ping_rx) = mpsc::channel(1);
            let reader = async move {
                let mut codec = websocket_codec::protocol::FrameHeaderCodec;
                let mut read = read;
                let mut read_buf = BytesMut::with_capacity(MAX_BUFFER_SIZE);
                let (mut data_tx, data_rx) = mpsc::channel(3);
                let mut data_rx = Some(data_rx);
                loop {
                    let h = match codec.decode(&mut read_buf) {
                        Ok(Some(h)) => h,
                        Ok(None) => {
                            read_buf.reserve(1);
                            match read.read_buf(&mut read_buf).await {
                                Ok(0) => break,//warn_!("Potential EOF from client"),
                                Ok(_n) => (),
                                Err(e) =>
                                    error_!(
                                        "Io error occured during websocket messages: {:?}",
                                        e
                                    ),
                            }
                            continue;
                        },
                        Err(_e) => {
                            //error_!("Websocket client broke protocol: {:?}", e);
                            Self::close(&broker_tx, super::PROTOCOL_ERROR).await;
                            break;
                        },
                    };
                    // A frame has been decoded
                    // Save some attributes
                    let fin = h.fin();
                    let close = h.opcode() == u8::from(Opcode::Close);
                    let mut mask: Option<u32> = h.mask().map(|m| m.into());
                    let mut remaining = if let Ok(r) = usize::try_from(h.data_len()) {
                        r
                    }else {
                        // Send a protocol error if the datalength isn't valid
                        Self::close(&broker_tx, super::PROTOCOL_ERROR).await;
                        break;
                    };
                    // Checks for some other protocol errors
                    if Self::protocol_error(&h, remaining) {
                        Self::close(&broker_tx, super::PROTOCOL_ERROR).await;
                        break;
                    }
                    if h.opcode() == u8::from(Opcode::Ping) {
                        // Read data, and send on ping_tx
                        // Note that remaining MUST be less than 125, otherwise it's a protocol
                        // error - checked by Self::protocol_error
                        // Avoid overflow
                        if read_buf.capacity() < remaining {
                            read_buf.reserve(remaining - read_buf.capacity());
                        }
                        while remaining > read_buf.len() {
                            let _e = read.read_buf(&mut read_buf).await;
                        }
                        let mut chunk = read_buf.split_to(remaining);
                        if let Some(mask) = &mut mask {
                            for b in chunk.iter_mut() {
                                *b ^= *mask as u8;
                                *mask = mask.rotate_right(8);
                            }
                        }
                        let _e = ping_tx.send(chunk.freeze()).await;
                        continue;
                    } else if h.opcode() == u8::from(Opcode::Pong) {
                        if remaining > read_buf.capacity() {
                            read_buf.reserve(remaining - read_buf.capacity());
                        }
                        while remaining > read_buf.len() {
                            let _e = read.read_buf(&mut read_buf).await;
                        }
                        continue;
                    }
                    // If there is a frame to continue, it must be continued
                    if let Some(data_rx) = data_rx.take() {
                        if h.opcode() == 0x0 {
                            Self::close(&broker_tx, super::PROTOCOL_ERROR).await;
                            break;
                        } else {
                            let message = WebsocketMessage::from_parts(h, data_rx);
                            let _e = message_tx.send(message).await;
                        }
                    } else if h.opcode() == u8::from(Opcode::Close) {
                        // This will disrupt handling of later messages, but since this was a close
                        // frame, we drop later messages anyway.
                        let (tx, rx) = mpsc::channel(3);
                        data_tx = tx;
                        let message = WebsocketMessage::from_parts(h, rx);
                        let _e = message_tx.send(message).await;
                    } else if h.opcode() != 0x0 {
                        Self::close(&broker_tx, super::PROTOCOL_ERROR).await;
                        break;
                    }
                    // Read and forward data - note that this will read (and discard) any data that
                    // the server decided to ignore
                    while remaining > 0 {
                        let mut chunk = read_buf.split_to(read_buf.len().min(remaining));
                        if let Some(mask) = &mut mask {
                            for b in chunk.iter_mut() {
                                *b ^= *mask as u8;
                                *mask = mask.rotate_right(8);
                            }
                        }
                        remaining -= chunk.len();
                        let _e = data_tx.send(chunk.into()).await;
                        if read_buf.len() < remaining {
                            read_buf.reserve(MAX_BUFFER_SIZE.min(remaining));
                            let _e = read.read_buf(&mut read_buf).await;
                        }
                    }
                    if fin {
                        // If this was the final frame, prepare for the next message
                        let (tx, rx) = mpsc::channel(3);
                        // explicitly drop data_tx
                        //let _e = data_tx.send(Bytes::new()).await;
                        //drop(data_tx);
                        data_tx = tx;
                        data_rx = Some(rx);
                    }
                    // If this was a close frame, stop recieving messages
                    if close {
                        break;
                    }
                }
                read
            };
            let writer = async move {
                // Explicit moves - probably not needed
                let mut write = write;
                let mut ping_rx = ping_rx;
                let mut codec = websocket_codec::protocol::FrameHeaderCodec;
                let mut write_buf = BytesMut::with_capacity(MAX_BUFFER_SIZE);
                while let Some(message) = Self::await_or_ping(&mut broker_rx, &mut ping_rx, &mut write, &mut write_buf, &mut codec).await {
                    let (header, mut rx) = message.into_parts();
                    if header.opcode() == u8::from(Opcode::Close) {
                        close_sent.store(true, atomic::Ordering::Release);
                    }
                    let mut running_header = Some(header);
                    while let Some(chunk) = Self::await_or_ping(&mut rx, &mut ping_rx, &mut write, &mut write_buf, &mut codec).await {
                        let mut parts = vec![chunk];
                        let mut fin = false;
                        // Buffer additional chunks if they are ready
                        loop {
                            // if the duration is zero, the rx will be polled at least once.
                            // However, because rx has backpressure, we want to suspend the current
                            // taks and allow new, ready chunks to be added.
                            match timeout(Duration::from_millis(3), rx.recv()).await {
                                Ok(Some(chunk)) => parts.push(chunk),
                                Ok(None) => {
                                    fin = true;
                                    break;
                                },
                                Err(_e) => break,
                            }
                        }
                        if let Some(header) = running_header.take() {
                            if header.mask().is_some() {
                                error_!("Websocket messages should not be sent with masks");
                            }
                            let int_header = FrameHeader::new(
                                    fin,
                                    header.rsv(),
                                    header.opcode(),
                                    None,
                                    parts.iter().map(|s| s.len()).sum::<usize>().into(),
                                );
                            let _e = codec.encode(int_header, &mut write_buf);
                            let _e = write.write_all_buf(&mut write_buf).await;
                            for mut chunk in parts {
                                let _e = write.write_all_buf(&mut chunk).await;
                            }
                            if !fin {
                                running_header = Some(FrameHeader::new(
                                        false,
                                        header.rsv(),
                                        0x0,// Continue frame
                                        header.mask(),
                                        0usize.into(), // We're going to ignore this anyway
                                    ));
                            }else {
                                break;
                            }
                        }else {
                            todo!("")
                        }
                    }
                    // If no data was sent, we send the header with an empty body anyway
                    if let Some(header) = running_header.take() {
                        if header.mask().is_some() {
                            error_!("Websocket messages should not be sent with masks");
                        }
                        let int_header = FrameHeader::new(
                                true,
                                header.rsv(),
                                header.opcode(),
                                None,
                                0usize.into(),
                            );
                        let _e = codec.encode(int_header, &mut write_buf);
                        let _e = write.write_all_buf(&mut write_buf).await;
                    }
                    if close_sent.load(atomic::Ordering::Acquire) {
                        break;
                    }
                }
                if !close_sent.load(atomic::Ordering::Acquire) {
                    warn_!("Writer task did not send close");
                }
                write
            };
            //let reader = tokio::spawn(reader);
            //let writer = tokio::spawn(writer);
            let _e = join! {
                writer,
                reader,
            };
        }
    }

    /// This waits for a message on `f`, but will forward pings to the client if any are sent. This
    /// allows pings & pongs to go between other messages
    async fn await_or_ping<T, W: AsyncWrite + Unpin>(
        f: &mut mpsc::Receiver<T>,
        ping_rx: &mut mpsc::Receiver<Bytes>,
        writer: &mut W,
        write_buf: &mut BytesMut,
        codec: &mut FrameHeaderCodec,
    ) -> Option<T> {
        loop {
            select! {
                o = f.recv() => break o,
                Some(p) = ping_rx.recv() => {
                    let pong_header = FrameHeader::new(
                        true,
                        0x0,
                        Opcode::Pong.into(),
                        None,
                        p.len().into()
                    );
                    let _e = codec.encode(pong_header, write_buf);
                    let _e = writer.write_all_buf(write_buf).await;
                    let _e = writer.write_all(&p).await;
                },
            }
        }
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

/// A websocket Channel. This can be used to message the client that sent a message (namely, the
/// one you are responding to), as well as broadcast messages to other clients. Note that every
/// client will recieve a broadcast, including the client that triggered the broadcast.
///
/// The lifetime parameter should typically be <'_>. This is nessecary since the object contains
/// references (which it might be possible to remove in later versions), and recent versions of
/// Rust recommend not allowing implicit elided lifetimes.
///
/// This is only valid as a Request Guard on Websocket handlers (`message`, `join`, and `leave`).
/// TODO: change implementation to verify this at compile time, rather than runtime.
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
    pub async fn close_with_status(&self, status: WebsocketStatus<'_>) {
        self.send_raw(
            WebsocketMessage::close(Some(status))
        ).await
    }

    /// Broadcasts `message` to every client connected to this topic. Topics are identified by
    /// the Origin URL used to connect to the server.
    pub async fn broadcast(&self, message: impl IntoMessage) {
        self.broker.send(&self.uri, to_message(message));
    }

    /// Broadcasts `message` to every client connected to `topic`. Topics are identified by
    /// the Origin URL used to connect to the server.
    pub async fn broadcast_to(&self, topic: &Origin<'_>, message: impl IntoMessage) {
        self.broker.send(topic, to_message(message));
    }
}

#[crate::async_trait]
impl<'r> FromWebsocket<'r> for Channel<'r> {
    type Error = &'static str;

    async fn from_websocket(request: &'r Websocket<'_>) -> Outcome<Self, Self::Error> {
        Outcome::Success(Self::from(request.inner_channel(), request.topic()))
    }
}

