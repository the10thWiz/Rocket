use std::convert::TryFrom;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::time::Duration;

use bytes::Bytes;
use bytes::BytesMut;
use rocket_http::uri::Origin;
use rocket_http::{Status, hyper::upgrade::Upgraded};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::join;
use tokio::time::sleep;
use tokio::time::timeout;
use tokio::sync::{mpsc, oneshot};
use tokio_util::codec::{Decoder, Encoder};
use websocket_codec::Opcode;
use websocket_codec::protocol::FrameHeader;

use crate::channels::MAX_BUFFER_SIZE;
use crate::channels::WebsocketMessage;
use crate::{Request, request::{FromRequest, Outcome}};

use crate::log::*;

use super::IntoMessage;
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
            remaining > 125
        {
            true
        } else if header.rsv() != 0x0 {
            true
        } else if Opcode::try_from(header.opcode()).is_none() {
            true
        } else {
            false
        }
    }

    async fn message_handler(
        upgrade_rx: oneshot::Receiver<Upgraded>,
        mut broker_tx: mpsc::Sender<WebsocketMessage>,
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
                        Err(e) => {
                            error_!("Websocket client broke protocol: {:?}", e);
                            break;
                        },
                    };
                    let fin = h.fin();
                    let close = h.opcode() == u8::from(Opcode::Close);
                    let mut remaining = usize::try_from(h.data_len())
                        .expect("Invalid Length in frame");
                    // Control frames may not have a length larger than 125 bytes
                    if Self::protocol_error(&h, remaining) {
                        let (tx, rx) = mpsc::channel(1);
                        let header = FrameHeader::new(
                            true,
                            0x0,
                            Opcode::Close.into(),
                            None,
                            0usize.into(),
                        );
                        let _e = broker_tx.send(WebsocketMessage::from_parts(header, rx)).await;
                        let _e = tx.send(super::PROTOCOL_ERROR.encode()).await;
                        break;
                    }
                    // TODO: remove expect
                    let mut mask: Option<u32> = h.mask().map(|m| m.into());
                    if let Some(data_rx) = data_rx.take() {
                        if h.opcode() == 0x0 {
                            error_!("Websocket client sent an unexepexted continue frame");
                        } else {
                            let message = WebsocketMessage::from_parts(h, data_rx);
                            let _e = message_tx.send(message).await;
                        }
                    } else if h.opcode() != 0x0 {
                        error_!("Websocket client sent an unexepexted frame");
                    }
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
                    if close {
                        break;
                    }
                    // Give time for writer?
                    //sleep(Duration::from_millis(10)).await;
                }
            };
            let writer = async move {
                let mut codec = websocket_codec::protocol::FrameHeaderCodec;
                let mut write = write;
                let mut write_buf = BytesMut::with_capacity(MAX_BUFFER_SIZE);
                while let Some(message) = broker_rx.recv().await {
                    let (header, mut rx) = message.into_parts();
                    if header.opcode() == u8::from(Opcode::Close) {
                        close_sent.store(true, atomic::Ordering::Release);
                    }
                    let mut running_header = Some(header);
                    while let Some(chunk) = rx.recv().await {
                        let mut parts = vec![chunk];
                        let mut fin = false;
                        // Buffer additional chunks if they are ready
                        loop {
                            // if the duration is zero, the rx will be polled at least once.
                            // However, because rx has backpressure, we want to suspend the current
                            // taks and allow new, ready chunks to be added.
                            match timeout(Duration::from_millis(10), rx.recv()).await {
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
            };
            //let reader = tokio::spawn(reader);
            //let writer = tokio::spawn(writer);
            let _e = join! {
                writer,
                reader,
            };
        }
    }
}

mod io_util {
    use std::{cell::UnsafeCell, pin::Pin, sync::Arc, task::{Context, Poll}};

    use tokio::{io::{AsyncRead, AsyncWrite, ReadBuf}, sync::{OwnedSemaphorePermit, Semaphore}};
    use tokio_util::sync::PollSemaphore;

    pub fn split<T>(stream: T) -> (ReadHalf<T>, WriteHalf<T>) {
        let semaphore = Arc::new(Semaphore::new(1));
        let t = Arc::new(UnsafeCell::new(stream));
        let read = Inner {
            semaphore: PollSemaphore::new(semaphore.clone()),
            t: t.clone(),
        };
        let write = Inner {
            semaphore: PollSemaphore::new(semaphore),
            t,
        };
        (ReadHalf { inner: read }, WriteHalf { inner: write })
    }

    pub struct ReadHalf<T> {
        inner: Inner<T>,
    }

    impl<T: AsyncRead> AsyncRead for ReadHalf<T> {
        fn poll_read(mut self: Pin<&mut Self>, cx: &mut Context<'_>, buf: &mut ReadBuf<'_>) -> Poll<std::io::Result<()>> {
            match self.inner.poll_lock(cx) {
                Poll::Pending => {
                    eprintln!("Reader waiting for lock");
                    Poll::Pending},
                Poll::Ready(g) => {
                    eprintln!("Reading");
                    g.inner.poll_read(cx, buf)},
            }
        }
    }

    pub struct WriteHalf<T> {
        inner: Inner<T>,
    }

    impl<T: AsyncWrite> AsyncWrite for WriteHalf<T> {
        fn poll_write(mut self: Pin<&mut Self>, cx: &mut Context<'_>, buf: &[u8]) -> Poll<std::io::Result<usize>> {
            match self.inner.poll_lock(cx) {
                Poll::Pending => {
                    eprintln!("Writer waiting for lock");
                    Poll::Pending},
                Poll::Ready(g) => {
                    eprintln!("Writing");
                    g.inner.poll_write(cx, buf)},
            }
        }
        fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
            match self.inner.poll_lock(cx) {
                Poll::Pending => Poll::Pending,
                Poll::Ready(g) => g.inner.poll_flush(cx),
            }
        }
        fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
            match self.inner.poll_lock(cx) {
                Poll::Pending => Poll::Pending,
                Poll::Ready(g) => g.inner.poll_shutdown(cx),
            }
        }
    }

    struct Inner<T> {
        semaphore: PollSemaphore,
        t: Arc<UnsafeCell<T>>,
    }

    impl<T> Inner<T> {
        fn poll_lock(&mut self, cx: &mut Context<'_>) -> Poll<Guard<'_, T>> {
            match self.semaphore.poll_acquire(cx) {
                Poll::Pending => Poll::Pending,
                Poll::Ready(Some(permit)) => Poll::Ready(Guard {
                    _permit: permit,
                    inner: unsafe { Pin::new_unchecked( &mut *self.t.get() ) }
                }),
                Poll::Ready(None) => todo!(),
            }
        }
    }

    struct Guard<'a, T> {
        _permit: OwnedSemaphorePermit,
        inner: Pin<&'a mut T>,
    }

    unsafe impl<T: Send> Send for ReadHalf<T> {}
    unsafe impl<T: Send> Send for WriteHalf<T> {}
    unsafe impl<T: Sync> Sync for ReadHalf<T> {}
    unsafe impl<T: Sync> Sync for WriteHalf<T> {}
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
    pub async fn close_with_status(&self, status: WebsocketStatus) {
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
