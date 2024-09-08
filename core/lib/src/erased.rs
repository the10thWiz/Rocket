use std::io;
use std::mem::transmute;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Poll, Context};

use futures::future::BoxFuture;
use http::request::Parts;
use tokio::io::{AsyncRead, ReadBuf};

use crate::catcher::TypedError;
use crate::data::{Data, IoHandler, RawStream};
use crate::{Request, Response, Rocket, Orbit};

// TODO: Magic with trait async fn to get rid of the box pin.
// TODO: Write safety proofs.

macro_rules! static_assert_covariance {
    ($($T:tt)*) => (
        const _: () = {
            fn _assert_covariance<'x: 'y, 'y>(x: &'y $($T)*<'x>) -> &'y $($T)*<'y> { x }
        };
    )
}

#[derive(Debug)]
pub struct ErasedRequest {
    // XXX: SAFETY: This (dependent) field must come first due to drop order!
    request: Request<'static>,
    _rocket: Arc<Rocket<Orbit>>,
    _parts: Box<Parts>,
}

impl Drop for ErasedRequest {
    fn drop(&mut self) { }
}

pub struct ErasedError<'r> {
    error: Option<Pin<Box<dyn TypedError<'r> + 'r>>>,
}

impl<'r> ErasedError<'r> {
    pub fn new() -> Self {
        Self { error: None }
    }

    pub fn write(&mut self, error: Option<Box<dyn TypedError<'r> + 'r>>) {
        // SAFETY: To meet the requirements of `Pin`, we never drop
        // the inner Box. This is enforced by only allowing writing
        // to the Option when it is None.
        assert!(self.error.is_none());
        if let Some(error) = error {
            self.error = Some(unsafe { Pin::new_unchecked(error) });
        }
    }

    pub fn is_some(&self) -> bool {
        self.error.is_some()
    }

    pub fn get(&'r self) -> Option<&'r dyn TypedError<'r>> {
        self.error.as_ref().map(|e| &**e)
    }
}

// TODO: #[derive(Debug)]
pub struct ErasedResponse {
    // XXX: SAFETY: This (dependent) field must come first due to drop order!
    response: Response<'static>,
    // XXX: SAFETY: This (dependent) field must come second due to drop order!
    error: ErasedError<'static>,
    _request: Arc<ErasedRequest>,
}

impl Drop for ErasedResponse {
    fn drop(&mut self) { }
}

pub struct ErasedIoHandler {
    // XXX: SAFETY: This (dependent) field must come first due to drop order!
    io: Box<dyn IoHandler + 'static>,
    _request: Arc<ErasedRequest>,
}

impl Drop for ErasedIoHandler {
    fn drop(&mut self) { }
}

impl ErasedRequest {
    pub fn new(
        rocket: Arc<Rocket<Orbit>>,
        parts: Parts,
        constructor: impl for<'r> FnOnce(
            &'r Rocket<Orbit>,
            &'r Parts
        ) -> Request<'r>,
    ) -> ErasedRequest {
        let rocket: Arc<Rocket<Orbit>> = rocket;
        let parts: Box<Parts> = Box::new(parts);
        let request: Request<'_> = {
            let rocket: &Rocket<Orbit> = &rocket;
            // SAFETY: The `Request` can borrow from `Rocket` because it has a stable
            // address (due to `Arc`)  and it is kept alive by the containing
            // `ErasedRequest`. The `Request` is always dropped before the
            // `Arc<Rocket>` due to drop order.
            let rocket: &'static Rocket<Orbit> = unsafe { transmute(rocket) };
            let parts: &Parts = &parts;
            // SAFETY: Same as above, but for `Box<Parts>`.
            let parts: &'static Parts = unsafe { transmute(parts) };
            constructor(rocket, parts)
        };

        ErasedRequest { _rocket: rocket, _parts: parts, request, }
    }

    pub fn inner(&self) -> &Request<'_> {
        static_assert_covariance!(Request);
        &self.request
    }

    pub async fn into_response<T, D>(
        self,
        raw_stream: D,
        preprocess: impl for<'r, 'x> FnOnce(
            &'r Rocket<Orbit>,
            &'r mut Request<'x>,
            &'r mut Data<'x>,
            &'r mut ErasedError<'r>,
        ) -> BoxFuture<'r, T>,
        dispatch: impl for<'r> FnOnce(
            T,
            &'r Rocket<Orbit>,
            &'r Request<'r>,
            Data<'r>,
            &'r mut ErasedError<'r>,
        ) -> BoxFuture<'r, Response<'r>>,
    ) -> ErasedResponse
        where T: Send + Sync + 'static,
              D: for<'r> Into<RawStream<'r>>
    {
        let mut data: Data<'_> = Data::from(raw_stream);
        // SAFETY: At this point, ErasedRequest contains a request, which is permitted
        // to borrow from `Rocket` and `Parts`. They both have stable addresses (due to
        // `Arc` and `Box`), and the Request will be dropped first (due to drop order).
        // SAFETY: Here, we place the `ErasedRequest` (i.e. the `Request`) behind an `Arc`
        // to ensure it has a stable address, and we again use drop order to ensure the `Request`
        // is dropped before the values that can borrow from it.
        let mut parent = Arc::new(self);
        // SAFETY: This error is permitted to borrow from the `Request` (as well as `Rocket` and
        // `Parts`).
        let mut error = ErasedError { error: None };
        let token: T = {
            let parent: &mut ErasedRequest = Arc::get_mut(&mut parent).unwrap();
            let rocket: &Rocket<Orbit> = &parent._rocket;
            let request: &mut Request<'_> = &mut parent.request;
            let data: &mut Data<'_> = &mut data;
            // SAFETY: As below, `error` must be reborrowed with the correct lifetimes.
            preprocess(rocket, request, data, unsafe { transmute(&mut error) }).await
        };

        let parent = parent;
        let response: Response<'_> = {
            let parent: &ErasedRequest = &parent;
            // SAFETY: This static reference is immediatly reborrowed for the correct lifetime.
            // The Response type is permitted to borrow from the `Request`, `Rocket`, `Parts`, and
            // `error`. All of these types have stable addresses, and will not be dropped until
            // after Response, due to drop order.
            let parent: &'static ErasedRequest = unsafe { transmute(parent) };
            let rocket: &Rocket<Orbit> = &parent._rocket;
            let request: &Request<'_> = &parent.request;
            // SAFETY: As above, `error` must be reborrowed with the correct lifetimes.
            dispatch(token, rocket, request, data, unsafe { transmute(&mut error) }).await
        };

        ErasedResponse {
            error,
            _request: parent,
            response,
        }
    }
}

impl ErasedResponse {
    pub fn inner(&self) -> &Response<'_> {
        static_assert_covariance!(Response);
        &self.response
    }

    pub fn with_inner_mut<'a, T>(
        &'a mut self,
        f: impl for<'r> FnOnce(&'a mut Response<'r>) -> T
    ) -> T {
        static_assert_covariance!(Response);
        f(&mut self.response)
    }

    pub fn make_io_handler<'a, T: 'static>(
        &'a mut self,
        constructor: impl for<'r> FnOnce(
            &'r Request<'r>,
            &'a mut Response<'r>,
        ) -> Option<(T, Box<dyn IoHandler + 'r>)>
    ) -> Option<(T, ErasedIoHandler)> {
        // SAFETY: If an error has been thrown, the `IoHandler` could
        // technically borrow from it, so we must ensure that this is
        // not the case. This could be handled safely by changing `error`
        // to be an `Arc` internally, and cloning the Arc to get a copy
        // (like `ErasedRequest`), however it's unclear this is actually
        // useful, and we can avoid paying the cost of an `Arc`
        if self.error.is_some() {
            warn!("Attempting to upgrade after throwing a typed error is not supported");
            return None;
        }
        let parent: Arc<ErasedRequest> = self._request.clone();
        let io: Option<(T, Box<dyn IoHandler + '_>)> = {
            let parent: &ErasedRequest = &parent;
            // SAFETY: As in other cases, the request is kept alive by the `Erased...`
            // type.
            let parent: &'static ErasedRequest = unsafe { transmute(parent) };
            let request: &Request<'_> = &parent.request;
            constructor(request, &mut self.response)
        };

        io.map(|(v, io)| (v, ErasedIoHandler { _request: parent, io }))
    }
}

impl ErasedIoHandler {
    pub fn with_inner_mut<'a, T: 'a>(
        &'a mut self,
        f: impl for<'r> FnOnce(&'a mut Box<dyn IoHandler + 'r>) -> T
    ) -> T {
        fn _assert_covariance<'x: 'y, 'y>(
            x: &'y Box<dyn IoHandler + 'x>
        ) -> &'y Box<dyn IoHandler + 'y> { x }

        f(&mut self.io)
    }

    pub fn take<'a>(&'a mut self) -> Box<dyn IoHandler + 'a> {
        fn _assert_covariance<'x: 'y, 'y>(
            x: &'y Box<dyn IoHandler + 'x>
        ) -> &'y Box<dyn IoHandler + 'y> { x }

        self.with_inner_mut(|handler| std::mem::replace(handler, Box::new(())))
    }
}

impl AsyncRead for ErasedResponse {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        self.get_mut().with_inner_mut(|r| Pin::new(r.body_mut()).poll_read(cx, buf))
    }
}
