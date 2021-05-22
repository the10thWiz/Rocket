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

use tokio::sync::mpsc;

use super::{WebsocketChannel, websockets::{IntoMessage, WebsocketMessage, to_message}};

/// A channel descriptor trait. This allows a `Channel` to have multiple
/// sub channels, and to control how messages are shared between clients.
///
/// This is different from Eq, because it does not need to be communicative,
/// so `a.matches(b)` is often not the same as `b.matches(a)`. This is
/// especially true for the stock implementations of `Option` and `Vec`,
/// which both implement non communicative matching methods.
///
/// The default implementation for Option allows a message sent to `None` to
/// match any `Some(T)`, but not the other way around. If they are both `Some(T)`,
/// it delegates to `T`'s match implementation. The default implementation for
/// `Vec` acts somewhat similarly; a shorter `Vec` can match a longer `Vec` as long
/// as every element in `other` matches the corresponding element in `self`.
///
/// All other std types that have ChannelDescriptor implemented just use `==` to compare
/// them.
///
/// A simple Phoenix like example could look like this:
///
/// ```rust
/// struct Topic(String, Option<String>);
/// ```
///
/// This means a message to the topic `("hello", None)` would not just be sent to clients subscribed
/// to the topic `("hello", None)`, but also to any clients subscribed to `("hello", Some(_))`.
/// However, the reverse is not true - a message to `("hello", Some("main"))` would not be sent to
/// any client except those subscribed to `("hello", Some("main"))`.
///
/// # WIP
/// This trait, and the associated features are still a work in progress.
/// We may wish to require type to implement `Eq`, which could make
/// unsubscribing easier. In general, I don't expect it to cause any issues,
/// since most types which you would want to implement this on, `Eq` can just
/// be derived.
///
/// We should add implementations for any other types people are likely to want,
/// that we can provide sensible default implementations for.
///
/// It would be nice to be able to provide a default implementation of this
/// trait with a derive macro.
pub trait Topic: Send + 'static {
    /// When called, self always refers to the ChannelDescriptor that the
    /// client subscribed to, and other is the ChannelDescriptor that the
    /// message was sent to.
    fn matches(&self, other: &Self) -> bool;
}

macro_rules! derive_via_eq {
    ($($type:ident),*) => {
        $(
        impl Topic for $type {
            fn matches(&self, other: &Self) -> bool {
                self == other
            }
        }
        )*
    };
}

derive_via_eq!(String);
derive_via_eq!(usize, u8, u16, u32, u64, u128);
derive_via_eq!(isize, i8, i16, i32, i64, i128);

impl<T> Topic for Vec<T>
    where T: Topic
{
    fn matches(&self, other: &Self) -> bool {
        // Disallow matches when other is longer
        self.len() >= other.len() &&
            self.iter().zip(other.iter()).all(|(s, o)| s.matches(o))
    }
}

impl<T> Topic for Option<T>
    where T: Topic
{
    fn matches(&self, other: &Self) -> bool {
        if let Some(s) = self.as_ref() {
            if let Some(o) = other.as_ref() {
                // If both are some, attempt the match with T
                s.matches(o)
            }else {
                // If self is Some, but other is None, never match
                false
            }
        }else {
            // If self is None, other only matches when it is also None
            other.is_none()
        }
    }
}

/// Internal enum for sharing messages between clients
enum BrokerMessage<T: Topic + 'static> {
    /// Registers a websocket to recieve messages from a room
    ///
    /// Note: this should only be sent once per websocket connection
    Register(T, mpsc::Sender<WebsocketMessage>),

    /// Removes a previously registered listener
    ///
    /// Note, this will remove all matching listeners, since there is no Eq bounds
    Unregister(T, mpsc::Sender<WebsocketMessage>),

    /// Removes all previously registered listeners for this client
    UnregisterAll(mpsc::Sender<WebsocketMessage>),

    /// Sends a message that should be forwarded to every socket listening
    Forward(T, WebsocketMessage),
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
#[derive(Clone)]
pub struct Broker<T: Topic> {
    channels: mpsc::UnboundedSender<BrokerMessage<T>>,
}

impl<T: Topic> Broker<T> {
    /// Creates a new channel, and starts the nessecary tasks in the background. The task will
    /// automatically end as soon as every handle on this channel has been dropped.
    pub fn new() -> Self {
        let (sender, reciever) = mpsc::unbounded_channel();
        tokio::spawn(Self::channel_task(reciever));
        Self {
            channels: sender,
        }
    }

    /// Sends a message to all clients subscribed to this channel using descriptor `id`
    ///
    /// See ChannelDescriptor for more info on the matching process
    ///
    /// ** WIP: NO-OP **
    pub fn send(&self, id: impl Into<T>, message: impl IntoMessage) {
        let _ = self.channels.send(BrokerMessage::Forward(id.into(), to_message(message)));
    }

    /// Subscribes the client to this channel using the descriptor `id`
    ///
    /// See ChannelDescriptor for more info on the matching process
    pub fn subscribe(&self, id: impl Into<T>, channel: &WebsocketChannel) {
        let _ = self.channels.send(
            BrokerMessage::Register(id.into(), channel.subscribe_handle())
        );
    }

    /// Unsubscribes the client from this channel using the descriptor `id`
    ///
    /// # Note
    /// This will unsubscribe this client from EVERY descriptor that matches `id`, not just one
    /// that is exactly equal.
    /// See ChannelDescriptor for more info on the matching process
    pub fn unsubscribe(&self, id: impl Into<T>, channel: &WebsocketChannel) {
        let _ = self.channels.send(
            BrokerMessage::Unregister(id.into(), channel.subscribe_handle())
        );
    }

    /// Unsubscribes the client from any messages on this channel
    ///
    /// The client is automatically unsubscribed if they are disconnected, so this does not need
    /// to be called when the client is disconnecting
    pub fn unsubscribe_all(&self, channel: &WebsocketChannel) {
        let _ = self.channels.send(BrokerMessage::UnregisterAll(channel.subscribe_handle()));
    }

    /// Channel task for tracking subscribtions and forwarding messages
    async fn channel_task(mut rx: mpsc::UnboundedReceiver<BrokerMessage<T>>) {
        let mut subs = ChannelMap::new(100);
        while let Some(wsm) = rx.recv().await {
            match wsm {
                BrokerMessage::Register(room, tx) => subs.insert(tx, room),
                BrokerMessage::Forward(room, message) => (),//subs.send(room, message),
                BrokerMessage::Unregister(room, tx) => subs.remove_value(tx, room),
                BrokerMessage::UnregisterAll(tx) => subs.remove_key(tx),
            }
        }
    }
}

/// Convient struct for holding channel subscribtions
struct ChannelMap<T: Topic>(Vec<(mpsc::Sender<WebsocketMessage>, Vec<T>)>);

impl<T: Topic> ChannelMap<T> {
    /// Create map with capactity
    fn new(capacity: usize) -> Self {
        Self(Vec::with_capacity(capacity))
    }

    /// Add `descriptor` to the list of subscriptions for `tx`
    fn insert(&mut self, tx: mpsc::Sender<WebsocketMessage>, descriptor: T) {
        for (t, v) in self.0.iter_mut() {
            if t.same_channel(&tx) {
                v.push(descriptor);
                return;
            }
        }
        self.0.push((tx, vec![descriptor]));
    }

    /// Remove every descriptor `tx` is subscribed to
    fn remove_key(&mut self, tx: mpsc::Sender<WebsocketMessage>) {
        self.0.retain(|(t, _)| !t.same_channel(&tx));
    }

    /// Remove every descriptor that `descriptor` matches and `tx` is subscribed to
    fn remove_value(&mut self, tx: mpsc::Sender<WebsocketMessage>, descriptor: T) {
        for (t, v) in self.0.iter_mut() {
            if t.same_channel(&tx) {
                v.retain(|d| !d.matches(&descriptor));
                return;
            }
        }
    }

    // /// Forward a message to every client that is subscribed to a descriptor that matches
    // /// `descriptor`
    //fn send(&mut self, descriptor: T, message: WebsocketMessage) {
        //self.0.retain(|(t, v)| {
            //if v.iter().any(|r| r.matches(&descriptor)) {
                //// message.clone() should be very cheap, since it uses `Bytes` internally to store
                //// the raw data
                //if let Err(_) = t.send(message.clone()) {
                    //// An error is only returned when the websocket has closed
                    //return false;
                //}
            //}
            //true
        //});
    //}
}
