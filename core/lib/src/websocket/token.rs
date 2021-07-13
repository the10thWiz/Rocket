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

use std::{sync::Arc, time::{Duration, Instant}};

use dashmap::DashMap;
use rand::Rng;
use rocket_http::{ext::IntoOwned, uri::Origin};
use state::Container;

use crate::{request::FromRequest, response::Responder};

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
            request.rocket().websocket_tokens.create(
                Arc::clone(&request.state.cache),
                request.uri().clone().into_owned()
            )
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

#[derive(Debug)]
pub(crate) struct TokenRef {
    pub cache: Arc<Container![Send + Sync]>,
    pub uri: Origin<'static>,
    start: Instant,
    expired: bool,
}

impl TokenRef {
    pub fn expired(&self) -> bool {
        self.start.elapsed() > Duration::from_secs(30)
    }
}

// The choice of concurrent hashmap is hard. I don't actually need all of the normal operations,
// I just need some slightly interesting opterations:
//
// try_insert: insert, but fail if the item already exists
// remove: remove element, returning it as an owned value
// remove_all: remove every element that matches a condition, returning the removed values as owned

/// TokenTable is a wrapper around DashMap, a concurrent HashMap. It does do some locking
/// internally, however this wrapper never returns a reference into the map itself, so deadlocks
/// should not be possible.
#[derive(Debug)]
pub(crate) struct TokenTable {
    map: DashMap<String, TokenRef>,
}

/// Length of tokens to generate
const TOKEN_LEN: usize = 16;

impl TokenTable {
    /// Create a new empty TokenTable
    pub fn new() -> Self {
        Self {
            map: DashMap::new(),
        }
    }

    /// Creates a token, and saves the associated data into the table
    pub fn create(&self, cache: Arc<Container![Send + Sync]>, uri: Origin<'static>) -> String {
        // The token is not guaranteed to be unique, but it is likely given the use of rng.
        // Technically, it only needs to be unique for a short period of time (the time between
        // cleanups, hopefully this is enough)

        // thread_rng is lazily initialized, so this should be really cheap if the rng has already
        // been initialized on this thread.
        let token: String = rand::thread_rng()
            .sample_iter(rand::distributions::Alphanumeric)
            .take(TOKEN_LEN)
            .map(char::from)
            .collect();
        self.map.insert(token.clone(), TokenRef {
            cache,
            uri,
            start: Instant::now(),
            expired: false,
        });
        token
    }

    /// Attempts the get a value from a given URI.
    ///
    /// If the uri does not contain a token, None is returned
    ///
    /// None is also returned if the token is not valid
    pub fn from_uri(&self, uri: &Origin<'_>) -> Option<TokenRef> {
        let mut segs = uri.path().segments();
        if segs.next() == Some("websocket") && segs.next() == Some("token") {
            let token = segs.next()?;
            if segs.next() == None {
                return self.get(token);
            }
        }
        None
    }

    /// Gets the value of a given token. Returns None if the token is expired
    pub fn get(&self, token: &str) -> Option<TokenRef> {
        self.map.remove_if(token, |_k, v| !v.expired() && !v.expired).map(|(_k, v)| v)
    }

    /// Gets a list of TokenRefs that have expired. TokenRefs will only be returned exactly once by
    /// this function, even on repeated calls.
    // TODO: Schedule a periodic Removal procedure
    pub fn get_expired(&self) -> Vec<TokenRef> {
        let mut ret = vec![];
        self.map.alter_all(|_k, v| {
            if v.expired() && !v.expired {
                let start = v.start;
                ret.push(v);
                TokenRef {
                    cache: Arc::new(<Container![Send + Sync]>::new()),
                    uri: Origin::const_new("/", None),
                    start,
                    expired: true,
                }
            } else {
                v
            }
        });
        self.map.retain(|_k, v| !v.expired);
        ret
    }
}
