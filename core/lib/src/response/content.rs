//! Contains types that set the Content-Type of a response.
//!
//! # Usage
//!
//! Each type in this module is a `Responder` that wraps an existing
//! `Responder`, overwriting the `Content-Type` of the response but otherwise
//! delegating the response to the wrapped responder. As a convenience,
//! `(ContentType, R)` where `R: Responder` is _also_ a `Responder` that
//! overrides the `Content-Type` to the value in `.0`:
//!
//! ```rust
//! # use rocket::get;
//! use rocket::http::ContentType;
//!
//! #[get("/")]
//! fn index() -> (ContentType, &'static str) {
//!     (ContentType::HTML, "Is this HTML? <p>Sure, why not!</p>")
//! }
//! ```
//!
//! # Example
//!
//! The following snippet creates a `RawHtml` response from a string. Normally,
//! raw strings set their response Content-Type to `text/plain`. By using the
//! `RawHtml` content response, the Content-Type will be set to `text/html`
//! instead:
//!
//! ```rust
//! use rocket::response::content;
//!
//! let response = content::RawHtml("<h1>Hello, world!</h1>");
//! ```

use rocket_http::Header;

use crate::fairing::{Fairing, Info, Kind};
use crate::request::Request;
use crate::response::{self, Response, Responder};
use crate::http::ContentType;

macro_rules! ctrs {
    ($($name:ident: $ct:ident, $name_str:expr, $ct_str:expr),+) => {
        $(
            #[doc="Override the `Content-Type` of the response to <b>"]
            #[doc=$name_str]
            #[doc="</b>, or <i>"]
            #[doc=$ct_str]
            #[doc="</i>."]
            ///
            /// Delegates the remainder of the response to the wrapped responder.
            ///
            /// **Note:** Unlike types like [`Json`](crate::serde::json::Json)
            /// and [`MsgPack`](crate::serde::msgpack::MsgPack), this type _does
            /// not_ serialize data in any way. You should _always_ use those
            /// types to respond with serializable data. Additionally, you
            /// should _always_ use [`NamedFile`](crate::fs::NamedFile), which
            /// automatically sets a `Content-Type`, to respond with file data.
            #[derive(Debug, Clone, PartialEq)]
            pub struct $name<R>(pub R);

            /// Sets the Content-Type of the response then delegates the
            /// remainder of the response to the wrapped responder.
            impl<'r, 'o: 'r, R: Responder<'r, 'o>> Responder<'r, 'o> for $name<R> {
                fn respond_to(self, req: &'r Request<'_>) -> response::Result<'o> {
                    (ContentType::$ct, self.0).respond_to(req)
                }
            }
        )+
    }
}

ctrs! {
    // FIXME: Add a note that this is _not_ `serde::Json`.
    RawJson: JSON, "JSON", "application/json",
    RawXml: XML, "XML", "text/xml",
    RawMsgPack: MsgPack, "MessagePack", "application/msgpack",
    RawHtml: HTML, "HTML", "text/html",
    RawText: Text, "plain text", "text/plain",
    RawCss: CSS, "CSS", "text/css",
    RawJavaScript: JavaScript, "JavaScript", "application/javascript"
}

impl<'r, 'o: 'r, R: Responder<'r, 'o>> Responder<'r, 'o> for (ContentType, R) {
    fn respond_to(self, req: &'r Request<'_>) -> response::Result<'o> {
        Response::build()
            .merge(self.1.respond_to(req)?)
            .header(self.0)
            .ok()
    }
}

/// Fairing to set a default accepted content type for incoming requests.
///
/// This sets the `Accept` header of any incoming request if it is not already set. The default
/// catcher, and some other types of responses inspect this header to select an appropriate content
/// type.
///
/// # Example
///
/// ```rust
/// # use rocket::{Rocket, uri};
/// # use rocket::response::content::DefaultContentType;
/// # use rocket::http::ContentType;
/// # use rocket::local::blocking::Client;
/// let rocket = Rocket::build()
///     .attach(DefaultContentType::new(ContentType::JSON));
/// let client = Client::untracked(rocket).unwrap();
/// let ret_ty = client.get("/").dispatch().content_type();
/// assert_eq!(ret_ty, Some(ContentType::JSON), "Wrong Content Type");
/// ```
///
/// This header is typlically already set by the client, but there are some cases where a client
/// may not have specified. This is also useful for malformed requests, since Rocket has to
/// fabricate a request in order to route it to the 400 catcher.
pub struct DefaultContentType(ContentType);

impl DefaultContentType {
    /// Set a default content type for incoming messages
    pub fn new(t: ContentType) -> Self {
        Self(t)
    }
}

#[crate::async_trait]
impl Fairing for DefaultContentType {
    fn info(&self) -> Info {
        Info {
            name: "DefaultContentType",
            kind: Kind::Request,
        }
    }

    async fn on_request(&self, req: &mut Request<'_>, _data: &mut crate::Data<'_>) {
        if req.headers().get_one("Accept").is_none() {
            req.add_header(Header::new("Accept", format!("{}", self.0)));
        }
    }
}

mod tests {
    use crate as rocket;

    #[test]
    fn test_default_content_type() {
    }
}
