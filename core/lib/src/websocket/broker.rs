//! Phoenix-like channels for Rocket websockets.
//!
//! The implementation is somewhat complex, but quite flexible. A `Channel` object is created to
//! share messages using a subscription based model. A client can subscribe to a specific
//! descriptor in a channel, using the methods provided by the `Channel` object. See the
//! documentation for the `ChannelDescriptor` trait for more information on the matching process.
//!
//! Typically, a Channel will be created and added to the state that Rocket manages. This is
//! nessecary since Rocket needs to know what type you would like to use as the `ChannelDescriptor`,
//! and it also allows mutiple channels, depending on the descriptor type.

use std::{hash::Hash, sync::{Arc, Weak}};

use bytes::BytesMut;
use rocket_http::{Method, ext::IntoOwned, uri::Origin};
use tokio::{io::AsyncReadExt, sync::{OnceCell, mpsc}};

use crate::{Request, Rocket, request::{FromRequest, Outcome}, response::Responder, Orbit};

use super::{Protocol, channel::WebSocketChannel, message::{MAX_BUFFER_SIZE, WebSocketMessage}};

/// Internal enum for sharing messages between clients
enum BrokerMessage {
    /// Registers a websocket to recieve messages from a room
    ///
    /// Note: this should only be sent once per websocket connection
    Register(Origin<'static>, mpsc::Sender<WebSocketMessage>, Protocol),

    /// Removes a previously registered listener
    ///
    /// Note, this will remove all matching listeners, since there is no Eq bounds
    Unregister(Origin<'static>, mpsc::Sender<WebSocketMessage>),

    /// Removes all previously registered listeners for this client
    UnregisterAll(mpsc::Sender<WebSocketMessage>),

    /// Sends a message that should be forwarded to every socket listening
    Forward(Origin<'static>, WebSocketMessage),
}

/// A channel for sharing messages between multiple clients, and the central server.
///
/// This should typically be created, and added to Rocket's managed state, where it
/// can be accessed via the state request guard. `Channel` also implements clone, and
/// acts as a handle to the internal channels, which allows messages to be generated
/// and sent outside of Rocket request handlers.
///
/// See the examples for how to use Channel.
/// TODO: Create examples
pub struct Broker {
    rocket: Arc<OnceCell<Weak<Rocket<Orbit>>>>,
    channels: mpsc::UnboundedSender<BrokerMessage>,
}

/// Potential errors arrising from sending a message on the broker.
#[derive(Debug, Clone, Copy, Hash, PartialEq, Eq)]
pub enum BrokerError {
    /// Rocket is not yet running
    RocketNotRunning,
    /// The responder failed to produce a message
    ResponderFailed,
}

impl Broker {
    /// Creates a new channel, and starts the nessecary tasks in the background. The task will
    /// automatically end as soon as every handle on this channel has been dropped.
    pub(crate) fn new() -> Self {
        let (sender, reciever) = mpsc::unbounded_channel();
        tokio::spawn(Self::channel_task(reciever));
        Self {
            rocket: Arc::new(OnceCell::new()),
            channels: sender,
        }
    }

    /// Sends a message to all clients subscribed to the channel using descriptor `id`
    pub(crate) async fn broadcast_to(&self, id: &Origin<'_>, message: WebSocketMessage)
        -> Result<(), ()>
    {
        self.channels.send(BrokerMessage::Forward(id.clone().into_owned(), message))
            .map_err(|_| ())
    }

    /// Sends a message to all clients subscribed to the channel using descriptor `id`
    ///
    /// In general, it should never be nessecary to construct a topic id by hand. The `uri!` macro
    /// allows constructing arbitrary topics from topics at compile time, see [`uri!`](crate::uri)
    /// for more info. Within a websocket event handler, using `&Origin<'_>` as a request guard
    /// will get the topic of the event currently being handled, and can be passed as is to
    /// broadcast.
    pub async fn broadcast<'r, 'o: 'r>(&'r self, id: impl AsRef<Origin<'_>>, message: impl Responder<'r, 'o>) -> Result<(), BrokerError> {
        let rocket = self.rocket
            .get()
            .expect("No rocket added")
            .upgrade()
            .ok_or(BrokerError::RocketNotRunning)?;
        // This works, but it's not ideal. However, we cannot send message as a dyn Responder<'r,
        // 'o>, since the type isn't 'static. However, it may be possible to find a better way to
        // handle this, potentially using a
        let request = Request::new(
            rocket.as_ref(),
            Method::Get,
            id.as_ref().clone().into_owned(),
        );

        // Safety: the borrow is only used by the resonse created from message, which is consumed &
        // dropped before the end of the function. The request isn't dropped until the end of the
        // fn. This relies on the fact that values are dropped in the reverse order they were
        // defined. This MUST be mem::transmute, since it avoids using a raw pointer and making the
        // future !Send.
        //
        // This 'static is likely unnessecary, but I don't think there is an easy way around it.
        let borrow: &'r Request<'static> = unsafe { std::mem::transmute(&request) };
        let _ = request; // Shadow request to make sure it cannot be modified or moved
        #[allow(unused_variables)]
        let request = 0usize;
        match message.respond_to(borrow) {
            Ok(mut res) => {
                let binary = if let Some(utf8) = res.content_type().map(|c| c.is_utf8()).flatten() {
                    !utf8
                } else {
                    true // TODO: sniffing?
                };
                let t = res.body_mut();
                let (tx, rx) = mpsc::channel(1);
                if let Ok(()) = self.broadcast_to(borrow.uri(), WebSocketMessage::new(binary, rx)).await {
                    let mut buf = BytesMut::with_capacity(MAX_BUFFER_SIZE);
                    while let Ok(n) = t.read_buf(&mut buf).await {
                        if n == 0 {
                            break;
                        }
                        let tmp = buf.split();
                        let _e = tx.send(tmp.into()).await;
                        if buf.capacity() <= 0 {
                            buf.reserve(MAX_BUFFER_SIZE);
                        }
                    }
                }
                Ok(())
            },
            Err(_s) => Err(BrokerError::ResponderFailed),
        }
    }

    /// Subscribes the client to this channel using the descriptor `id`
    pub(crate) async fn subscribe(
        &self,
        id: &Origin<'_>,
        channel: &WebSocketChannel,
    ) {
        let _ = self.channels.send(BrokerMessage::Register(
            id.clone().into_owned(),
            channel.subscribe_handle(),
            channel.extensions().protocol()
        ));
    }

    /// Unsubscribes the client from this channel using the descriptor `id`
    ///
    /// # Note
    /// This will unsubscribe this client from EVERY descriptor that matches `id`
    #[allow(unused)]
    pub(crate) async fn unsubscribe(&self, id: &Origin<'_>, channel: &WebSocketChannel) {
        let _ = self.channels.send(
            BrokerMessage::Unregister(id.clone().into_owned(), channel.subscribe_handle())
        );
    }

    /// Unsubscribes the client from any messages on this channel
    ///
    /// The client is automatically unsubscribed if they are disconnected, so this does not need
    /// to be called when the client is disconnecting
    pub(crate) async fn unsubscribe_all(&self, channel: &WebSocketChannel) {
        let _ = self.channels.send(BrokerMessage::UnregisterAll(channel.subscribe_handle()));
    }

    /// Channel task for tracking subscribtions and forwarding messages
    async fn channel_task(mut rx: mpsc::UnboundedReceiver<BrokerMessage>) {
        let mut subs = ChannelMap::new(100);
        while let Some(wsm) = rx.recv().await {
            match wsm {
                BrokerMessage::Register(room, tx, protocol) => subs.insert(tx, room, protocol),
                BrokerMessage::Forward(room, message) => subs.send(room, message).await,
                BrokerMessage::Unregister(room, tx) => subs.remove_value(tx, room),
                BrokerMessage::UnregisterAll(tx) => subs.remove_key(tx),
            }
            // TODO make this happen less often
            subs.cleanup();
        }
    }

    /// Creates a new handle to this broker.
    pub fn clone(&self) -> Self {
        Self {
            rocket: Arc::clone(&self.rocket),
            channels: self.channels.clone(),
        }
    }

    /// Initialize the broker with a weak reference to Rocket
    ///
    /// # Panics
    ///
    /// Panics if this is called more than once
    pub(crate) fn with_rocket(&self, rocket: Weak<Rocket<Orbit>>) {
        self.rocket.set(rocket).expect("The broker was already initialized!");
    }
}

impl std::fmt::Debug for Broker {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Broker {{}}")
    }
}

#[crate::async_trait]
impl<'r> FromRequest<'r> for Broker {
    type Error = std::convert::Infallible;

    async fn from_request(request: &'r Request<'_>) -> Outcome<Self, Self::Error> {
        Outcome::Success(request.rocket().broker())
    }
}

/// Convient struct for holding channel subscribtions
struct ChannelMap(Vec<(mpsc::Sender<WebSocketMessage>, Vec<Origin<'static>>, Protocol)>);

impl ChannelMap {
    /// Create map with capactity
    fn new(capacity: usize) -> Self {
        Self(Vec::with_capacity(capacity))
    }

    /// Add `descriptor` to the list of subscriptions for `tx`
    fn insert(
        &mut self,
        tx: mpsc::Sender<WebSocketMessage>,
        descriptor: Origin<'static>,
        protocol: Protocol,
    ) {
        for (t, v, _) in self.0.iter_mut() {
            if t.same_channel(&tx) {
                v.push(descriptor);
                return;
            }
        }
        self.0.push((tx, vec![descriptor], protocol));
    }

    /// Remove every descriptor `tx` is subscribed to
    fn remove_key(&mut self, tx: mpsc::Sender<WebSocketMessage>) {
        self.0.retain(|(t, _, _)| !t.same_channel(&tx));
    }

    /// Remove every descriptor that `descriptor` matches and `tx` is subscribed to
    fn remove_value(&mut self, tx: mpsc::Sender<WebSocketMessage>, descriptor: Origin<'static>) {
        for (t, v, _) in self.0.iter_mut() {
            if t.same_channel(&tx) {
                v.retain(|d| d != &descriptor);
                return;
            }
        }
    }

    /// Forward a message to every client that is subscribed to a descriptor that matches
    /// `descriptor`
    async fn send(&mut self, descriptor: Origin<'static>, message: WebSocketMessage) {
        let mut chs = vec![];
        let (header, _, mut data) = message.into_parts();
        for (t, v, p) in self.0.iter() {
            if v.iter().any(|r| r == &descriptor) {
                // message.clone() should be very cheap, since it uses `Bytes` internally to store
                // the raw data
                let (data_tx, data_rx) = mpsc::channel(2);
                if let Ok(()) = t.send(WebSocketMessage::from_parts(
                    header.clone(),
                    p.with_topic(&descriptor),
                    data_rx
                )).await {
                    chs.push(data_tx);
                }
            }
        }

        tokio::spawn(async move {
            while let Some(next) = data.recv().await {
                for ch in chs.iter() {
                    // TODO prevent a slow client from blocking others
                    let _e = ch.send(next.clone()).await;
                }
            }
        });
    }

    fn cleanup(&mut self) {
        self.0.retain(|(t, _, _)| !t.is_closed());
    }
}
