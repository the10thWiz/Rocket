use rocket_http::{Status, ext::IntoOwned, uri::Origin};

use crate::{request::FromRequest, response::Responder};

pub struct WebSocketToken<T: Send + Sync> {
    data: T,
    uri: Option<Origin<'static>>,
}

impl<T: Send + Sync> WebSocketToken<T> {
    pub fn new(data: T) -> Self {
        Self { data, uri: None, }
    }

    pub fn get_data(&self) -> &T {
        &self.data
    }
}

impl<'r, 'o: 'r, T: Send + Sync> Responder<'r, 'o> for WebSocketToken<T> {
    fn respond_to(mut self, request: &'r crate::Request<'_>) -> crate::response::Result<'o> {
        // Set uri
        if self.uri.is_none() {
            self.uri = Some(request.uri().clone().into_owned());
        }

        Err(Status::ImATeapot)
    }
}

struct InnerTokenData<T: Send + Sync>(T);

#[crate::async_trait]
impl<'r, T: Send + Sync + 'static> FromRequest<'r> for WebSocketToken<&'r T> {
    type Error = std::convert::Infallible;

    async fn from_request(request: &'r crate::Request<'_>) -> crate::request::Outcome<Self, Self::Error> {
        if let Some(inner) = request.state.cache.try_get::<InnerTokenData<T>>() {
            crate::request::Outcome::Success(Self {
                data: &inner.0,
                uri: None,
            })
        } else {
            crate::request::Outcome::Forward(())
        }
    }
}
