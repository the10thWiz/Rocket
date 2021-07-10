//! WebSocket Tokens
//!
//! These types implement a mechanism for using standard HTTP authentication for WebSockets. The
//! primary mechanism is the use of a token: a client must first obtain a token by authenticating
//! against an HTTP route, then use the token to connect to a WebSocket endpoint.
//!
//! The primary type this module exports is `WebSocketToken`, which is likely the only type
//! you will need to interact with.
//!
//! WebSocket authentication is somewhat difficult, since the WebSocket protocol defines no
//! standard authentication methods, and most client libraries don't have the flexibility
//! required to actually implement authentication. The most common solution is the use a of a
//! token, obtained from a separate HTTP route. This allows the use of any Authentication protocol
//! your server already implements, and makes the code fairly simple.
//!
//! The Rocket code for creating a Token, and providing it to the user looks like this:
//!
//! ```rust,no_run
//! #[get("/listen")]
//! fn auth() -> WebSocketToken<()> {
//!     WebSocketToken::new(())
//! }
//! ```
//!
//! `WebSocketToken` implements responder, and simply provides the token to the user to connect.
//! The generic parameter on `WebSocketToken` is an associated data type. The data type here is
//! `()`, but a more practical use case might use a `User` struct or something similiar.
//!
//! On the client side, when the client makes a GET request to '/listen', the response will look
//! something like `/websocket/token/asdkasdidjdown7d`. The client should then open a websocket
//! connection to that url, e.g. with `new WebSocket('ws://' + document.location.host + url)` in
//! JS. On Rocket's side, this connection will be routed to event handlers as `/listen`, since that
//! was the uri used to connect to the authentication route.
//!
//! In order to ensure the client has authenticated, WebSocketToken is also a Request guard,
//! although it does need to be marked as a reference:
//!
//! ```rust,no_run
//! #[message("/listen", data = "<data>")]
//! fn message(data: Data<'_>, _token: WebSocketToken<&()>) {
//! }
//! ```
//!
//! This is actually handled via the request's local cache, and any values stored in the
//! authentication request's local cache will also be in the websocket event's local cache.

use std::{any::Any, net::SocketAddr, ptr::NonNull, sync::{Arc, atomic::{AtomicPtr, AtomicUsize}}, time::{Duration, Instant}};

use dashmap::DashMap;
use rand::Rng;
use rocket_http::{Accept, ContentType, HeaderMap, ext::IntoOwned, uri::Origin};
use state::{Container, Storage};

use crate::{Request, request::FromRequest, response::Responder};

/// A WebSocketToken for use in authenticating WebSocket connections
pub struct WebSocketToken<T: Send + Sync + 'static> {
    data: T,
    uri: Option<Origin<'static>>,
}

impl<T: Send + Sync + 'static> WebSocketToken<T> {
    /// Create a new token with the provided Data
    pub fn new(data: T) -> Self {
        Self { data, uri: None, }
    }

    /// Create a new token with the provided Data, that will direct the client to the provided URI
    pub fn with_uri<'a>(uri: impl Into<Origin<'a>>, data: T) -> Self {
        Self { data, uri: Some(uri.into().into_owned()), }
    }

    // Creates a temporary Connection URI using the provided Request. The URI is returned as a
    // String, although that is subject to change
    //pub fn create_connection_uri(self, request: &crate::Request<'_>) -> String {
        //let token = if let Some(uri) = self.uri {
            //request.rocket().websocket_tokens.create(self.data, uri)
        //} else {
            //request.rocket().websocket_tokens.create(self.data, request.uri().clone().into_owned())
        //};
        //format!("/websocket/token/{}", token)
    //}

    /// Gets the inner data saved by this token
    pub fn get_data(&self) -> &T {
        &self.data
    }
}

/// WebSocketToken Newtype wrapper to protect the value held in the Request local cache
struct WebSocketTokenWrapper<T: Send + Sync + 'static>(WebSocketToken<T>);

impl<'r, 'o: 'r, T: Send + Sync + 'static> Responder<'r, 'o> for WebSocketToken<T> {
    fn respond_to(mut self, request: &'r crate::Request<'_>) -> crate::response::Result<'o> {
        let token = if let Some(uri) = self.uri.take() {
            request.rocket().websocket_tokens.create(Arc::clone(&request.state.cache), uri)
        } else {
            request.rocket().websocket_tokens.create(Arc::clone(&request.state.cache), request.uri().clone().into_owned())
        };
        request.local_cache(|| WebSocketTokenWrapper(self));
        format!("/websocket/token/{}", token).respond_to(request)
    }
}

#[crate::async_trait]
impl<'r: 'o, 'o, T: Send + Sync + 'static> FromRequest<'r> for &'r WebSocketToken<T> {
    type Error = std::convert::Infallible;

    async fn from_request(request: &'r crate::Request<'_>)
        -> crate::request::Outcome<Self, Self::Error>
    {
        if let Some(data) = request.state.cache
            .try_get::<WebSocketTokenWrapper<T>>()
        {
            crate::request::Outcome::Success(&data.0)
        } else {
            crate::request::Outcome::Forward(())
        }
    }
}

/// Length of tokens to generate
const TOKEN_LEN: usize = 16;

// The choice of concurrent hashmap is hard. I don't actually need all of the normal operations,
// I just need some slightly interesting opterations:
//
// try_insert: insert, but fail if the item already exists
// remove: remove element, returning it as an owned value
// remove_all: remove every element that matches a condition, returning the removed values as owned
//
// Maybe I can do something dumb with Atomics and Boxes...

pub(crate) struct InnerRef {
    pub cache: Arc<Container![Send + Sync]>,
    pub uri: Origin<'static>,
}

struct TokenRef {
    /// MUST be constructed by Box and unique, or Null
    inner: AtomicPtr<InnerRef>,
    start: Instant,
}

impl TokenRef {
    pub fn new(cache: Arc<Container![Send + Sync]>, uri: Origin<'static>) -> Self {
        Self {
            inner: AtomicPtr::new(Box::into_raw(Box::new(InnerRef {
                cache,
                uri,
            }))),
            start: Instant::now(),
        }
    }

    pub fn get_inner(&self) -> Option<InnerRef> {
        let old = self.inner.swap(std::ptr::null_mut(), atomic::Ordering::AcqRel);
        // Safety:
        // Since the atomic pointer was swapped with null, any swaps after the first MUST
        // get null.
        // Therefore, we are either the only thread with this pointer, or it's null
        // - we have the pointer: we own it, and can convert it to a box safely.
        // - We got null: NonNull short circuits and returns None
        NonNull::new(old).map(|p| *unsafe { Box::from_raw(p.as_ptr()) })
    }

    pub fn expired(&self) -> bool {
        self.start.elapsed() > Duration::from_secs(30)
    }

    pub fn retired(&self) -> bool {
        self.inner.load(atomic::Ordering::Acquire).is_null()
    }

    fn clone(&self) -> Self {
        Self {
            inner: AtomicPtr::new(self.inner.load(atomic::Ordering::Acquire)),
            start: self.start,
        }
    }
}

/// An internal type that represents the current set of unprocessed tokens
///
/// Internally, it's a wrapper around
pub(crate) struct TokenTable {
    map: flurry::HashMap<String, TokenRef>,
    count: AtomicUsize,
}

impl std::fmt::Debug for TokenTable {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "TokenTable {{ map: .., count: .. }}")
    }
}

impl TokenTable {
    pub fn new() -> Self {
        Self {
            map: flurry::HashMap::new(),
            count: AtomicUsize::new(0),
        }
    }

    /// Attempts the get a value from a given URI.
    ///
    /// If the uri does not contain a token, None is returned
    ///
    /// None is also returned if the token is not valid
    pub fn from_uri(&self, uri: &Origin<'_>) -> Option<InnerRef> {
        let mut segs = uri.path().segments();
        if segs.next() == Some("websocket") && segs.next() == Some("token") {
            let token = segs.next()?;
            if segs.next() == None && uri.query() == None {
                // Check that it actually looks like a token
                if token.len() == TOKEN_LEN && token.chars().all(char::is_alphanumeric) {
                    return self.get(token);
                }
            }
        }
        None
    }

    fn gen_token() -> String {
        rand::thread_rng()
            .sample_iter(rand::distributions::Alphanumeric)
            .take(TOKEN_LEN)
            .map(char::from)
            .collect()
    }

    pub fn create(&self, cache: Arc<Container![Send + Sync]>, uri: Origin<'static>) -> String {
        let mut token_ref = TokenRef::new(cache, uri);
        loop {
            let token = Self::gen_token();
            match self.map.pin().try_insert(token.clone(), token_ref) {
                Ok(_) => break token,
                Err(e) => token_ref = e.not_inserted,
            }
        }
    }

    pub fn get(&self, token: &str) -> Option<InnerRef> {
        self.map.pin().remove(token).map(|v| {
            if !v.expired() {
                v.get_inner()
            } else {
                // Re-insert value into the map - The key is chosen arbitrarily, but it
                // specifically doesn't look like a token, so it wil not be fetched again
                self.map.pin().insert(
                    format!("#{}", self.count.fetch_add(1, atomic::Ordering::AcqRel)),
                    v.clone()
                );
                None
            }
        }).flatten()
    }

    pub fn get_expired(&self, vec: &mut Vec<InnerRef>) {
        self.map.pin()
            .iter()
            .filter_map(|(_k, v)| {
                if v.expired() {
                    v.get_inner()
                } else {
                    None
                }
            })
            .for_each(|i| vec.push(i));
        self.map.pin().retain(|_k, v| !v.retired());
    }
}

