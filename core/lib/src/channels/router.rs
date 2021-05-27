//! Internal Routing structs

use std::borrow::Borrow;
use std::collections::HashMap;
use std::{io::Cursor, sync::Arc};

use futures::{Future, FutureExt};
use rocket_codegen::uri;
use rocket_http::uri::Uri;
use rocket_http::{Header, Status, hyper::upgrade::Upgraded, uri::Origin};
use rocket_http::hyper::{self, header::{CONNECTION, UPGRADE}, upgrade::OnUpgrade};
use tokio::sync::oneshot;

use websocket_codec::{ClientRequest, Opcode};

use crate::route::WebsocketEvent;
use crate::{Data, Request, Response, Rocket, Route, phase::Orbit};
use crate::router::{Collide, Collisions};
use yansi::Paint;

use super::broker::Broker;
use super::{WebsocketChannel, channel::InnerChannel};

async fn handle<Fut, T, F>(name: Option<&str>, run: F) -> Option<T>
    where F: FnOnce() -> Fut, Fut: Future<Output = T>,
{
    use std::panic::AssertUnwindSafe;

    macro_rules! panic_info {
        ($name:expr, $e:expr) => {{
            match $name {
                Some(name) => error_!("Handler {} panicked.", Paint::white(name)),
                None => error_!("A handler panicked.")
            };

            info_!("This is an application bug.");
            info_!("A panic in Rust must be treated as an exceptional event.");
            info_!("Panicking is not a suitable error handling mechanism.");
            info_!("Unwinding, the result of a panic, is an expensive operation.");
            info_!("Panics will severely degrade application performance.");
            info_!("Instead of panicking, return `Option` and/or `Result`.");
            info_!("Values of either type can be returned directly from handlers.");
            warn_!("A panic is treated as an internal server error.");
            $e
        }}
    }

    let run = AssertUnwindSafe(run);
    let fut = std::panic::catch_unwind(run)
        .map_err(|e| panic_info!(name, e))
        .ok()?;

    AssertUnwindSafe(fut)
        .catch_unwind()
        .await
        .map_err(|e| panic_info!(name, e))
        .ok()
}

#[derive(Debug, PartialEq, Eq)]
enum Protocol {
    Naked,
    Multiplexed,
}

#[derive(Debug, PartialEq, Eq, Clone, Copy, Hash)]
enum Event {
    Join,
    Message,
    Leave,
}

#[derive(Debug)]
pub struct WebsocketRouter {
    routes: HashMap<Event, Vec<Route>>,
}

impl WebsocketRouter {
    pub fn new() -> Self {
        Self {
            routes: HashMap::new(),
        }
    }

    pub fn routes(&self) -> impl Iterator<Item = &Route> + Clone {
        self.routes.iter().flat_map(|(_, r)| r.iter())
    }

    pub fn add_route(&mut self, route: Route) {
        //if route.websocket_handler.is_some() {
            //self.routes.push(route);
        //}
        match route.websocket_handler {
            WebsocketEvent::None => (),
            WebsocketEvent::Join(_) => self.routes.entry(Event::Join).or_default().push(route),
            WebsocketEvent::Message(_) => self.routes.entry(Event::Message).or_default().push(route),
            WebsocketEvent::Leave(_) => self.routes.entry(Event::Leave).or_default().push(route),
        }
    }

    fn collisions<'a, I, T>(&self, items: I) -> impl Iterator<Item = (T, T)> + 'a
        where I: Iterator<Item = &'a T> + Clone + 'a, T: Collide + Clone + 'a,
    {
        items.clone().enumerate()
            .flat_map(move |(i, a)| {
                items.clone()
                    .skip(i + 1)
                    .filter(move |b| a.collides_with(b))
                    .map(move |b| (a.clone(), b.clone()))
            })
    }

    pub fn finalize(&self) -> Result<(), Collisions> {
        let routes: Vec<_> = self.collisions(self.routes()).collect();

        if !routes.is_empty() {
            return Err(Collisions { routes, catchers: vec![] })
        }

        Ok(())
    }

    fn route<'r, 'a: 'r>(
        &'a self,
        event: Event,
        req: &'r Request<'r>,
        topic: &'r Option<Origin<'_>>,
    ) -> impl Iterator<Item = &'a Route> + 'r {
        // Note that routes are presorted by ascending rank on each `add`.
        self.routes.get(&event)
            .into_iter()
            .flat_map(move |routes| routes.iter().filter(move |r| r.matches_topic(req, topic)))
    }

    async fn handle_message<'r, 'a: 'r>(
        &'a self,
        event: Event,
        req: &'r Request<'r>,
        topic: &'r Option<Origin<'_>>,
        mut message: Data,
    ) {
        for route in self.route(event, &req, topic) {
            req.set_route(route);

            let name = route.name.as_deref();
            let handler = route.websocket_handler.unwrap_ref();
            let res = handle(name, || handler.handle(&req, message)).await;
            // Successfully ran
            match res {
                Some(Err(d)) => message = d,
                _ => break,
            }
        }
    }

    pub fn is_upgrade(&self, hyper_request: &hyper::Request<hyper::Body>) -> bool {
        hyper_request.method() == hyper::Method::GET &&
            ClientRequest::parse(|n| hyper_request.headers()
                                 .get(n).map(|s| s.to_str().unwrap_or(""))
                                ).is_ok()
    }

    pub async fn handle(
        rocket: Arc<Rocket<Orbit>>,
        mut request: hyper::Request<hyper::Body>,
        h_addr: std::net::SocketAddr,
        tx: oneshot::Sender<hyper::Response<hyper::Body>>
    ) {
        let upgrade = hyper::upgrade::on(&mut request);
        let (h_parts, h_body) = request.into_parts();

        // Convert the Hyper request into a Rocket request.
        let req_res = Request::from_hyp(
            &rocket, h_parts.method, h_parts.headers, &h_parts.uri, h_addr
        );

        let mut req = match req_res {
            Ok(req) => req,
            Err(e) => {
                error!("Bad incoming request: {}", e);
                // TODO: We don't have a request to pass in, so we just
                // fabricate one. This is weird. We should let the user know
                // that we failed to parse a request (by invoking some special
                // handler) instead of doing this.
                let dummy = Request::new(&rocket, rocket_http::Method::Get, Origin::ROOT);
                let r = rocket.handle_error(Status::BadRequest, &dummy).await;
                rocket.send_response(r, tx).await;
                return;
            }
        };
        let mut data = Data::from(h_body);

        // Dispatch the request to get a response, then write that response out.
        let _token = rocket.preprocess_request(&mut req, &mut data).await;

        //let mut response = None;
        let (websocket_channel, upgrade_tx) = WebsocketChannel::new();
        req.local_cache(|| Some(InnerChannel::from_websocket(&websocket_channel, rocket.state().unwrap())));

        let req_copy = req.clone();
        if rocket.websocket_router.route(Event::Message, &req_copy, &None).nth(0).is_some() {
            let (response, protocol) = Self::create_reponse(&req_copy);
            rocket.send_response(response, tx).await;
            match protocol {
                Protocol::Naked => Self::websocket_task_naked(
                        &mut req,
                        upgrade,
                        websocket_channel,
                        upgrade_tx
                    ).await,
                Protocol::Multiplexed => unimplemented!(),
            }
        }else {
            let response = Self::handle_error(Status::NotFound);
            rocket.send_response(response, tx).await;
        }
    }

    fn create_reponse<'r>(req: &'r Request<'r>) -> (Response<'r>, Protocol) {
        // Use websocket-codec to parse the client request
        let cl_req = match ClientRequest::parse(|n| req.headers().get_one(n)) {
            Ok(v) => v,
            Err(_e) => return (Self::handle_error(Status::UpgradeRequired), Protocol::Naked),
        };

        let protocol = if req.headers()
            .get("Sec-WebSocket-Protocol")
            .flat_map(|s| s.split(",").map(|s| s.trim()))
            .any(|s| s.eq_ignore_ascii_case("rocket-multiplex"))
        {
            Protocol::Multiplexed
        } else {
            Protocol::Naked
        };

        let mut response = Response::build();
        response.status(Status::SwitchingProtocols);
        response.header(Header::new(CONNECTION.as_str(), "upgrade"));
        response.header(Header::new(UPGRADE.as_str(), "websocket"));
        response.header(Header::new("Sec-WebSocket-Accept", cl_req.ws_accept()));
        if protocol == Protocol::Multiplexed {
            response.header(Header::new("Sec-WebSocket-Protocol", "rocket-multiplex"));
        }
        response.sized_body(None, Cursor::new("Switching to websocket"));
        (response.finalize(), protocol)
    }

    /// Construct a rocket response from the given hyper request
    fn handle_error<'_b>(status: Status) -> Response<'_b> {
        let mut response = Response::build();
        response.status(status);
        response.finalize()
    }

    async fn websocket_task_naked<'r, 'a: 'r>(
        request: &'a mut Request<'r>,
        on_upgrade: OnUpgrade,
        mut ws: WebsocketChannel,
        upgrade_tx: oneshot::Sender<Upgraded>,
    ) {
        let broker = request.rocket().state::<Broker>().unwrap().clone();
        if let Ok(upgrade) = on_upgrade.await {
            let _e = upgrade_tx.send(upgrade);
            
            request.state.rocket.websocket_router.handle_message(Event::Join, request, &None, Data::local(vec![])).await;
            broker.subscribe(request.uri(), &ws);
            while let Some(message) = ws.next().await {
                //println!("Message: {:?}", message);
                match message.opcode() {
                    Opcode::Text => 
                        request.state.rocket.websocket_router.handle_message(
                            Event::Message,
                            request,
                            &None,
                            Data::from_ws(message, Some(false))
                        ).await,
                    Opcode::Binary => 
                        request.state.rocket.websocket_router.handle_message(
                            Event::Message,
                            request,
                            &None,
                            Data::from_ws(message, Some(true))
                        ).await,
                    Opcode::Ping => (),
                    Opcode::Pong => (),
                    Opcode::Close => break,
                }
            }
            broker.unsubscribe_all(&ws);
            request.state.rocket.websocket_router.handle_message(Event::Leave, request, &None, Data::local(vec![])).await;
            // TODO implement Ping/Pong (not exposed to the user)
            // TODO handle Close correctly (we should reply with Close,
            // unless we initiated it)
        }
    }

    async fn websocket_task_multiplexed<'r>(
        request: &'r mut Request<'r>,
        on_upgrade: OnUpgrade,
        mut ws: WebsocketChannel,
        upgrade_tx: oneshot::Sender<Upgraded>,
    ) {
        let broker = request.rocket().state::<Broker>().unwrap().clone();
        if let Ok(upgrade) = on_upgrade.await {
            let _e = upgrade_tx.send(upgrade);
            
            request.state.rocket.websocket_router.handle_message(Event::Join, request, &None, Data::local(vec![])).await;
            broker.subscribe(request.uri(), &ws);
            while let Some(message) = ws.next().await {
                match message.opcode() {
                    Opcode::Text => 
                        request.state.rocket.websocket_router.handle_message(
                            Event::Message,
                            request,
                            &None,
                            Data::from_ws(message, Some(false))
                        ).await,
                    Opcode::Binary => 
                    {
                        //request.set_uri(Origin::parse("/echo").unwrap());
                        request.state.rocket.websocket_router.handle_message(
                            Event::Message,
                            request,
                            &None,
                            Data::from_ws(message, Some(true))
                        ).await;
                    }
                    Opcode::Ping => (),
                    Opcode::Pong => (),
                    Opcode::Close => break,
                }
            }
            broker.unsubscribe_all(&ws);
            request.state.rocket.websocket_router.handle_message(Event::Leave, request, &None, Data::local(vec![])).await;
            // TODO implement Ping/Pong (not exposed to the user)
            // TODO handle Close correctly (we should reply with Close,
            // unless we initiated it)
        }
    }
}
