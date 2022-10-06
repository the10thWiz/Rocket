use std::{
    fmt::{Debug, Display},
    num::NonZeroU64,
    sync::atomic::AtomicU64, convert::Infallible,
};

use super::{FromRequest, Outcome, Request};

/// Opaque Request ID type. Every incoming request is assigned a unique ID, which can be used to
/// identify which log messages are related to a specific request
///
/// NOTE:
///     Although the RequestId itself is unique for every request, the `Display` implementation
///     only prints part of the whole id. This is enough to differentiate requests nearby, but not
///     enough to differntiate all requests Rocket serves.
///
/// This type is designed only for use when logging. As such, there are no methods provided to
/// get the internal id value, just `Display` & `Debug` implementations. Future implementations may
/// choose to implement more functionality, such as `Hash` & `Eq`.
#[derive(Clone, Copy)]
pub struct RequestId {
    // NonZeroU64, so that Option<RequestId> can be optimized to a single 64 bit value
    id: NonZeroU64,
}

impl RequestId {
    /// Convert a number to an id
    ///
    /// # Panics
    /// panics if the id is zero
    #[cfg(test)]
    pub(crate) fn from(id: u64) -> Self {
        Self { id: NonZeroU64::new(id).unwrap() }
    }
}

const MAX_PRINT_ID: u64 = 0x10000;

impl Display for RequestId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Avoid printing more than 4 digits by specifying a maximum id. Mod is used to apply a
        // wrapping, i.e. the log will wrap requests periodically. It's unlikely a server can
        // handle 65536 requests concurrently, so this should be enough for to differntiate any
        // request.
        write!(f, "Req {:X}>", self.id.get() % MAX_PRINT_ID)
    }
}

impl Debug for RequestId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Request ID {{ {:X} }}", self.id)
    }
}

#[crate::async_trait]
impl<'r> FromRequest<'r> for RequestId {
    type Error = Infallible;

    async fn from_request(request: & 'r Request<'_>) -> Outcome<Self,Self::Error> {
        Outcome::Success(request.request_id())
    }
}

/// Struct to generate request ids
#[derive(Debug)]
pub(crate) struct RequestIdGenerator {
    next: AtomicU64,
}

impl RequestIdGenerator {
    /// Construct a default generator starting at one
    pub(crate) fn new() -> Self {
        Self {
            next: AtomicU64::new(1),
        }
    }

    /// Get the next ID from the generator
    pub(crate) fn next(&self) -> RequestId {
        // SAFETY: self.next is initalized to one, and only ever incremented. Since self.next is a
        // u64, overflow can be considered impossible.
        RequestId {
            id: unsafe {
                NonZeroU64::new_unchecked(self.next.fetch_add(1, atomic::Ordering::AcqRel))
            },
        }
    }
}

impl Default for RequestIdGenerator {
    fn default() -> Self {
        Self::new()
    }
}

/// Get the id of the current request, if such a request exists.
///
/// ```rust
/// #[get("/")]
/// fn get_id() -> String {
///     format!("{}", current_request().unwrap())
/// }
/// ```
// TODO: add test to doc comment
pub fn current_request() -> Option<RequestId> {
    CURRENT_REQUEST.try_with(|id| *id).ok()
}

tokio::task_local! {
    pub static CURRENT_REQUEST: RequestId;
}
