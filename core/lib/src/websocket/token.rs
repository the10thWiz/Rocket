//! WebSocket Tokens
//!
//! These types implement a mechanism for using standard HTTP authentication for WebSockets. The
//! primary mechanism is the use of a token: a client must first obtain a token by authenticating
//! against an HTTP route, then use the token to connect to a WebSocket endpoint.

use std::{any::Any, time::{Duration, Instant}};

use dashmap::DashMap;
use rand::{Rng, SeedableRng, prelude::ThreadRng};
use rocket_http::{ext::IntoOwned, uri::Origin};

use crate::{request::FromRequest, response::Responder};

/// A WebSocketToken for use in authenticating WebSocket connections
pub struct WebSocketToken<T: Send + Sync> {
    data: T,
    uri: Option<Origin<'static>>,
}

impl<T: Send + Sync + 'static> WebSocketToken<T> {
    /// Create a new token with the provided Data
    pub fn new(data: T) -> Self {
        Self { data, uri: None, }
    }

    /// Creates a temporary Connection URI using the provided Request. The URI is returned as a
    /// String, although that is subject to change
    pub fn create_connection_uri(self, request: &crate::Request<'_>) -> String {
        let token = if let Some(uri) = self.uri {
            request.rocket().websocket_tokens.create(self.data, uri)
        } else {
            request.rocket().websocket_tokens.create(self.data, request.uri().clone().into_owned())
        };
        format!("/websocket/token/{}", token)
    }
}

impl<T: Send + Sync> WebSocketToken<T> {
    /// Gets the inner data saved by this token
    pub fn get_data(&self) -> &T {
        &self.data
    }
}

impl<'r, 'o: 'r, T: Send + Sync + 'static> Responder<'r, 'o> for WebSocketToken<T> {
    fn respond_to(self, request: &'r crate::Request<'_>) -> crate::response::Result<'o> {
        let token = if let Some(uri) = self.uri {
            request.rocket().websocket_tokens.create(self.data, uri)
        } else {
            request.rocket().websocket_tokens.create(self.data, request.uri().clone().into_owned())
        };
        format!("/websocket/token/{}", token).respond_to(request)
    }
}

pub(crate) struct InnerTokenData(Box<dyn Any + Send + Sync + 'static>);

impl std::fmt::Debug for InnerTokenData {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "InnerTokenData(?)")
    }
}

#[crate::async_trait]
impl<'r: 'o, 'o, T: Send + Sync + 'static> FromRequest<'r> for WebSocketToken<&'o T> {
    type Error = std::convert::Infallible;

    async fn from_request(request: &'r crate::Request<'_>)
        -> crate::request::Outcome<Self, Self::Error>
    {
        if let Some(Some(data)) = request.state.cache
            .try_get::<InnerTokenData>()
            .map(|i| i.0.downcast_ref::<T>())
        {
            crate::request::Outcome::Success(Self {
                data,
                uri: None,
            })
        } else {
            crate::request::Outcome::Forward(())
        }
    }
}

#[derive(Debug)]
pub(crate) struct TokenRef {
    pub data: InnerTokenData,
    pub uri: Origin<'static>,
    start: Instant,
    expired: bool,
}


impl TokenRef {
    pub fn expired(&self) -> bool {
        self.start.elapsed() > Duration::from_secs(30)
    }
}

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
    pub fn create<T: Send + Sync + 'static>(&self, val: T, uri: Origin<'static>) -> String {
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
            data: InnerTokenData(Box::new(val)),
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
                    data: InnerTokenData(Box::new(())),
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
