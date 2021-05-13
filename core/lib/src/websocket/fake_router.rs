use rocket_http::hyper;
use tokio::sync::mpsc;

use crate::{Route, router::Collisions};

enum WebsocketMessage {
}


#[derive(Clone, Debug)]
pub(crate) struct WebsocketRouter {}


impl WebsocketRouter {
    pub(crate) fn new() -> Self {
        Self{}
    }
    pub(crate) fn finalize(&mut self) -> Result<(), Collisions> {
        Ok(())
    } 
    pub fn is_upgrade(&self, hyper_request: hyper::Request<hyper::Body>) -> bool {
        false
    }
}
