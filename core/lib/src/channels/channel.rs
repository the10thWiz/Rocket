use std::convert::TryFrom;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::time::Duration;

use bytes::Bytes;
use bytes::BytesMut;
use futures::Future;
use rocket_http::ext::IntoOwned;
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
use state::Container;

use crate::channels::MAX_BUFFER_SIZE;
use crate::channels::rocket_multiplex::MULTIPLEX_CONTROL_CHAR;
use crate::channels::WebSocketMessage;
use crate::request::Outcome;

use crate::log::*;

use super::FromWebSocket;
use super::IntoMessage;
use super::Protocol;
use super::WebSocket;
use super::status::{self, WebSocketStatus};
use super::broker::Broker;
use super::to_message;

/// An internal representation of a WebSocket connection
pub(crate) struct WebSocketChannel {
    inner: mpsc::Receiver<WebSocketMessage>,
    sender: mpsc::Sender<WebSocketMessage>,
    close_sent: Arc<AtomicBool>,
}

impl WebSocketChannel {
    pub fn new() -> (Self, oneshot::Sender<Upgraded>) {
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
    pub fn subscribe_handle(&self) -> mpsc::Sender<WebSocketMessage> {
        self.sender.clone()
    }

    /// Get the next message from this client.
    ///
    /// This method also forwards messages sent from any channels the client is subscribed to
    pub async fn next(&mut self) -> Option<WebSocketMessage> {
        self.inner.recv().await
    }

    /// Gets the close_sent flag. This function returns true iff we have not sent a close message
    /// yet, i.e. we should send a close message.
    pub fn should_send_close(&self) -> bool {
        !self.close_sent.load(atomic::Ordering::Acquire)
    }

    // TODO maybe avoid repeated if blocks?
    fn protocol_error(header: &FrameHeader, remaining: usize) -> bool {
        if (header.opcode() == u8::from(Opcode::Close) ||
            header.opcode() == u8::from(Opcode::Ping) ||
            header.opcode() == u8::from(Opcode::Pong)) &&
            (remaining > 125 || !header.fin())
        {
            true
        } else if header.rsv() != 0x0 {
            //eprintln!("{:b}", header.rsv());
            true
        } else if Opcode::try_from(header.opcode()).is_none() && header.opcode() != 0x0 {
            true
        } else {
            false
        }
    }

    pub async fn close(
        broker_tx: &mpsc::Sender<WebSocketMessage>,
        status: WebSocketStatus<'_>
    ) {
        let (tx, rx) = mpsc::channel(1);
        let header = FrameHeader::new(
            true,
            0x0,
            Opcode::Close.into(),
            None,
            0usize.into(),
        );
        let _e = broker_tx.send(WebSocketMessage::from_parts(header, None, rx)).await;
        let _e = tx.send(status.encode()).await;
    }

    async fn message_handler(
        upgrade_rx: oneshot::Receiver<Upgraded>,
        broker_tx: mpsc::Sender<WebSocketMessage>,
        mut broker_rx: mpsc::Receiver<WebSocketMessage>,
        message_tx: mpsc::Sender<WebSocketMessage>,
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

            let recv_close = Arc::new(AtomicBool::new(false));
            let recv_close2 = recv_close.clone();
            let (ping_tx, ping_rx) = mpsc::channel(1);
            let reader = async move {
                let recv_close = recv_close;
                let mut codec = websocket_codec::protocol::FrameHeaderCodec;
                let mut read = read;
                let mut read_buf = BytesMut::with_capacity(MAX_BUFFER_SIZE);
                let (mut data_tx, data_rx) = mpsc::channel(3);
                let mut data_rx = Some(data_rx);
                let mut validator = Utf8Validator::new();
                let mut utf8 = false;
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
                                        "Io error occured during WebSocket messages: {:?}",
                                        e
                                    ),
                            }
                            continue;
                        },
                        Err(e) => {
                            error_!("WebSocket client broke protocol: {:?}", e);
                            Self::close(&broker_tx, status::PROTOCOL_ERROR).await;
                            break;
                        },
                    };
                    // A frame has been decoded
                    // Save some attributes
                    let fin = h.fin();
                    if h.opcode() == u8::from(Opcode::Close) {
                        recv_close.store(true, atomic::Ordering::Release);
                    }
                    let mut mask: Option<u32> = h.mask().map(|m| m.into());
                    let mut remaining = if let Ok(r) = usize::try_from(h.data_len()) {
                        r
                    }else {
                        // Send a protocol error if the datalength isn't valid
                        Self::close(&broker_tx, status::PROTOCOL_ERROR).await;
                        break;
                    };
                    // Checks for some other protocol errors
                    if Self::protocol_error(&h, remaining) {
                        warn_!("Remote Protocol Error");
                        Self::close(&broker_tx, status::PROTOCOL_ERROR).await;
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
                        // discard the pong message
                        let _ = read_buf.split_to(remaining);
                        continue;
                    }
                    // If there is a frame to continue, it must be continued
                    if let Some(data_rx) = data_rx.take() {
                        if h.opcode() == 0x0 {
                            error_!("Unexpected continue frame");
                            Self::close(&broker_tx, status::PROTOCOL_ERROR).await;
                            break;
                        } else {
                            utf8 = h.opcode() == u8::from(Opcode::Text);
                            let message = WebSocketMessage::from_parts(h, None, data_rx);
                            let _e = message_tx.send(message).await;
                        }
                    } else if h.opcode() == u8::from(Opcode::Close) {
                        // This will disrupt handling of later messages, but since this was a close
                        // frame, we drop later messages anyway.
                        let (tx, rx) = mpsc::channel(3);
                        data_tx = tx;
                        let message = WebSocketMessage::from_parts(h, None, rx);
                        let _e = message_tx.send(message).await;
                        utf8 = false;
                    } else if h.opcode() != 0x0 {
                        error_!("Expected continue frame");
                        Self::close(&broker_tx, status::PROTOCOL_ERROR).await;
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
                        if utf8 && !validator.validate(&chunk) {
                            error_!("WebSocket client sent Invalid UTF-8");
                            Self::close(&broker_tx, status::INVALID_DATA_TYPE).await;
                            break;
                        }
                        remaining -= chunk.len();
                        let _e = data_tx.send(chunk.into()).await;
                        if read_buf.len() < remaining {
                            read_buf.reserve(MAX_BUFFER_SIZE.min(remaining));
                            let _e = read.read_buf(&mut read_buf).await;
                        }
                    }
                    if validator.error() {
                        // Break above only breaks out of the inner loop
                        break;
                    }
                    if fin {
                        if utf8 && !validator.fin() {
                            error_!("WebSocket client sent Incomplete UTF-8");
                            Self::close(&broker_tx, status::INVALID_DATA_TYPE).await;
                            // Note that combining any two valid UTF-8 sequences must still be a
                            // valid UTF-8 sequence. Therefore, the validator doesn't need to be
                            // recreated. This is easy to see when looking at the implementation,
                            // since the validator just holds the number of continue sequences seen
                            break;
                        }
                        // If this was the final frame, prepare for the next message
                        let (tx, rx) = mpsc::channel(3);
                        // explicitly drop data_tx
                        //let _e = data_tx.send(Bytes::new()).await;
                        //drop(data_tx);
                        data_tx = tx;
                        data_rx = Some(rx);
                    }
                    // If this was a close frame, stop recieving messages
                    if recv_close.load(atomic::Ordering::Acquire) {
                        break;
                    }
                }
                // If we broke out at any point, we should set the flag to prevent messages in
                // flight from sending invalid frames
                recv_close.store(true, atomic::Ordering::Release);
                // This return to try and prevent dropping early
                read
            };

            let writer = async move {
                // Explicit moves - probably not needed
                let recv_close = recv_close2;
                let mut write = write;
                let mut ping_rx = ping_rx;
                let mut codec = websocket_codec::protocol::FrameHeaderCodec;
                let mut write_buf = BytesMut::with_capacity(MAX_BUFFER_SIZE);
                while let Some(message) = Self::await_or_ping(
                    &mut broker_rx,
                    &mut ping_rx,
                    &mut write,
                    &mut write_buf,
                    &mut codec).await
                {
                    let (header, mut topic, mut rx) = message.into_parts();
                    if header.opcode() == u8::from(Opcode::Close) {
                        close_sent.store(true, atomic::Ordering::Release);
                    }
                    let mut running_header = Some(header);
                    while let Some(chunk) = Self::await_or_ping(
                        &mut rx,
                        &mut ping_rx,
                        &mut write,
                        &mut write_buf,
                        &mut codec).await
                    {
                        let mut parts = vec![chunk];
                        let mut fin = false;
                        // Buffer additional chunks if they are ready
                        loop {
                            // if the duration is zero, the rx will be polled at least once.
                            // However, because rx has backpressure, we want to suspend the current
                            // taks and allow new, ready chunks to be added.
                            // However, the backpressure can be taken advantage of to make 5.19 &
                            // 5.20 pass
                            match timeout(Duration::from_millis(0), rx.recv()).await {
                                Ok(Some(chunk)) => parts.push(chunk),
                                Ok(None) => {
                                    fin = true;
                                    break;
                                },
                                Err(_e) => break,
                            }
                        }
                        if let Some(header) = running_header.take() {
                            // Prevent message from finishing if the closing handshake has already
                            // started
                            if recv_close.load(atomic::Ordering::Acquire)
                                && header.opcode() != u8::from(Opcode::Close)
                            {
                                break;
                            }
                            if header.mask().is_some() {
                                error_!("WebSocket messages should not be sent with masks");
                            }
                            let topic_buf = topic.take().map(|topic| topic.to_string());
                            let int_header = FrameHeader::new(
                                    fin,
                                    header.rsv(),
                                    header.opcode(),
                                    None,
                                    // .chain is used to add the length of the topic (if any)
                                    parts.iter()
                                        .map(|s| s.len())
                                        .chain(topic_buf.iter().map(|b| b.len() + 2))
                                        .sum::<usize>().into(),
                                );
                            let _e = codec.encode(int_header, &mut write_buf);
                            let _e = write.write_all_buf(&mut write_buf).await;

                            // Write out the topic if we have one. This can only be present once
                            // (since it is replaced with None by take), so it will only be sent
                            // once at the beginning of a message. Naked messages will send topic
                            // as None, and therefore nothing will be appended
                            if let Some(topic) = topic_buf {
                                let _e = write.write_all(topic.as_bytes()).await;
                                let _e = write.write_all(MULTIPLEX_CONTROL_CHAR).await;
                            }

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
                            unreachable!()
                        }
                    }
                    // If no data was sent, we send the header with an empty body anyway
                    if let Some(header) = running_header.take() {
                        if header.mask().is_some() {
                            error_!("WebSocket messages should not be sent with masks");
                        }
                        // Only write the fin frame if a close hasn't been received
                        // if opcode = close => always write
                        // if opcode != close, recv_close = false => write
                        // if opcode != close, recv_close = true => ignore
                        if header.opcode() == u8::from(Opcode::Close)
                            || !recv_close.load(atomic::Ordering::Acquire)
                        {
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
                    }
                    if close_sent.load(atomic::Ordering::Acquire) {
                        break;
                    }
                }
                if !close_sent.load(atomic::Ordering::Acquire) {
                    warn_!("Writer task did not send close");
                }
                let _e = write.shutdown().await;
                write
            };
            //let reader = tokio::spawn(reader);
            //let writer = tokio::spawn(writer);
            let (_w, _r) = join! {
                reader,
                writer,
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

enum Utf8Validator {
    Start,
    Continue,
    ContinueTwo,
    ContinueThree,
    E0,
    ED,
    F0,
    F4,
    Error,
}

impl Utf8Validator {
    fn new() -> Self {
        Self::Start
    }

    fn set_error(&mut self) -> bool {
        *self = Self::Error;
        false
    }

    fn validate(&mut self, bytes: &[u8]) -> bool {
        for b in bytes.iter() {
            match self {
                Self::Start => match b {
                    0x00..=0x7F => (),
                    0xC2..=0xDF => *self = Self::Continue,
                    0xE1..=0xEC => *self = Self::ContinueTwo,
                    0xEE..=0xEF => *self = Self::ContinueTwo,
                    0xF1..=0xF3 => *self = Self::ContinueThree,
                    0xE0 => *self = Self::E0,
                    0xED => *self = Self::ED,
                    0xF0 => *self = Self::F0,
                    0xF4 => *self = Self::F4,
                    _ => return self.set_error(),
                }
                Self::Continue => match b {
                    0x80..=0xBF => *self = Self::Start,
                    _ => return self.set_error(),
                }
                Self::ContinueTwo => match b {
                    0x80..=0xBF => *self = Self::Continue,
                    _ => return self.set_error(),
                }
                Self::ContinueThree => match b {
                    0x80..=0xBF => *self = Self::ContinueTwo,
                    _ => return self.set_error(),
                }
                Self::E0 => match b {
                    0xA0..=0xBF => *self = Self::Continue,
                    _ => return self.set_error(),
                },
                Self::ED => match b {
                    0x80..=0x9F => *self = Self::Continue,
                    _ => return self.set_error(),
                },
                Self::F0 => match b {
                    0x90..=0xBF => *self = Self::ContinueTwo,
                    _ => return self.set_error(),
                },
                Self::F4 => match b {
                    0x80..=0x8F => *self = Self::ContinueTwo,
                    _ => return self.set_error(),
                },
                Self::Error => return false,
            }
        }
        true
    }

    fn fin(&self) -> bool {
        matches!(self, Self::Start)
    }

    fn error(&self) -> bool {
        matches!(self, Self::Error)
    }
}

#[derive(Clone)]
pub(crate) struct InnerChannel {
    client: mpsc::Sender<WebSocketMessage>,
    broker: Broker,
    protocol: Protocol,
}

impl InnerChannel {
    pub(crate) fn from_websocket(
        chan: &WebSocketChannel,
        broker: &Broker,
        protocol: Protocol
    ) -> Self {
        Self{
            client: chan.subscribe_handle(),
            broker: broker.clone(),
            protocol,
        }
    }
}

/// A WebSocket Channel. This can be used to message the client that sent a message (namely, the
/// one you are responding to), as well as broadcast messages to other clients. Note that every
/// client will recieve a broadcast, including the client that triggered the broadcast.
///
/// The lifetime parameter should typically be <'_>. This is nessecary since the object contains
/// references (which it might be possible to remove in later versions), and recent versions of
/// Rust recommend not allowing implicit elided lifetimes.
///
/// This is only valid as a Request Guard on WebSocket handlers (`message`, `join`, and `leave`).
pub struct Channel<'r> {
    client: mpsc::Sender<WebSocketMessage>,
    broker: Broker,
    uri: &'r Origin<'r>,
    protocol: Protocol,
    cache: Arc<Container![Send + Sync]>,
}

impl<'r> Channel<'r> {
    //fn from(inner: InnerChannel, uri: &'r Origin<'r>) -> Self {
        //Self {
            //client: inner.client,
            //broker: inner.broker,
            //uri,
            //protocol: inner.protocol,
            //cache,
        //}
    //}

    /// Sends a raw Message to the client
    pub(crate) async fn send_raw(&self, message: WebSocketMessage) {
        let _e = self.client.send(message).await;
    }

    /// Send a message to the specific client connected to this webSocket
    pub async fn send(&self, message: impl IntoMessage) {
        match self.protocol {
            Protocol::Naked => self.send_raw(to_message(message)).await,
            Protocol::Multiplexed => self.send_raw(
                to_message(message).with_topic(self.uri.clone().into_owned())
            ).await,
        }
    }

    /// Sends a close notificaiton to the client, so no new messages will arive
    pub async fn close(&self) {
        self.send_raw(WebSocketMessage::close(None)).await
    }

    /// Sends a close notificaiton to the client, along with a reason for the close
    pub async fn close_with_status(&self, status: WebSocketStatus<'_>) {
        self.send_raw(
            WebSocketMessage::close(Some(status.to_owned()))
        ).await
    }

    /// Broadcasts `message` to every client connected to this topic. Topics are identified by
    /// the Origin URL used to connect to the server.
    pub async fn broadcast(&self, message: impl IntoMessage) {
        self.broker.send(&self.uri, to_message(message)).await;
    }

    /// Broadcasts `message` to every client connected to `topic`. Topics are identified by
    /// the Origin URL used to connect to the server.
    pub async fn broadcast_to(&self, topic: &Origin<'_>, message: impl IntoMessage) {
        self.broker.send(topic, to_message(message)).await;
    }

    /// Retrieves the cached value for type `T` from the channel-local cached
    /// state of `self`. If no such value has previously been cached for this
    /// request, `f` is called to produce the value which is subsequently
    /// returned.
    ///
    /// Different values of the same type _cannot_ be cached without using a
    /// proxy, wrapper type. To avoid the need to write these manually, or for
    /// libraries wishing to store values of public types, use the
    /// [`local_cache!`](crate::request::local_cache) macro to generate a
    /// locally anonymous wrapper type, store, and retrieve the wrapped value
    /// from request-local cache.
    ///
    /// # Example
    ///
    /// ```rust
    /// # let c = rocket::local::blocking::Client::debug_with(vec![]).unwrap();
    /// # let request = c.get("/");
    /// // The first store into local cache for a given type wins.
    /// let value = request.local_cache(|| "hello");
    /// assert_eq!(*request.local_cache(|| "hello"), "hello");
    ///
    /// // The following return the cached, previously stored value for the type.
    /// assert_eq!(*request.local_cache(|| "goodbye"), "hello");
    /// ```
    #[inline]
    pub fn local_cache<T, F>(&self, f: F) -> &T
        where F: FnOnce() -> T,
              T: Send + Sync + 'static
    {
        self.cache.try_get()
            .unwrap_or_else(|| {
                self.cache.set(f());
                self.cache.get()
            })
    }

    /// Retrieves the cached value for type `T` from the channel-local cached
    /// state of `self`. If no such value has previously been cached for this
    /// request, `fut` is `await`ed to produce the value which is subsequently
    /// returned.
    ///
    /// # Example
    ///
    /// ```rust
    /// # use rocket::Request;
    /// # type User = ();
    /// async fn current_user<'r>(request: &Request<'r>) -> User {
    ///     // validate request for a given user, load from database, etc
    /// }
    ///
    /// # rocket::async_test(async move {
    /// # let c = rocket::local::asynchronous::Client::debug_with(vec![]).await.unwrap();
    /// # let request = c.get("/");
    /// let current_user = request.local_cache_async(async {
    ///     current_user(&request).await
    /// }).await;
    /// # })
    #[inline]
    pub async fn local_cache_async<'a, T, F>(&'a self, fut: F) -> &'a T
        where F: Future<Output = T>,
              T: Send + Sync + 'static
    {
        match self.cache.try_get() {
            Some(s) => s,
            None => {
                self.cache.set(fut.await);
                self.cache.get()
            }
        }
    }
}

#[crate::async_trait]
impl<'r> FromWebSocket<'r> for Channel<'r> {
    type Error = &'static str;

    async fn from_websocket(request: &'r WebSocket<'_>) -> Outcome<Self, Self::Error> {
        let inner = request.inner_channel();
        Outcome::Success(Self {
            client: inner.client,
            broker: inner.broker,
            uri: request.topic(),
            protocol: inner.protocol,
            cache: Arc::clone(&request.request().state.cache),
        })
        //Outcome::Success(Self::from(request.inner_channel(), request.topic()))
    }
}
