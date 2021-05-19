//! Internal Routing structs

use std::{io::Cursor, sync::Arc};

use futures::{Future, FutureExt};
use rocket_http::{Header, Status, hyper::upgrade::{Parts, Upgraded}, uri::Origin};
use rocket_http::hyper::{self, header::{CONNECTION, UPGRADE}, upgrade::OnUpgrade};
use tokio::{net::TcpStream, sync::oneshot};
use tokio_util::codec::Decoder;
use websocket_codec::{ClientRequest, Message, MessageCodec, Opcode};

use crate::{Data, Request, Response, Rocket, Route, phase::Orbit};
use crate::router::{Collide, Collisions};
use yansi::Paint;

use super::{WebsocketChannel, websockets::Channel};

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
    let fut = std::panic::catch_unwind(move || run())
        .map_err(|e| panic_info!(name, e))
        .ok()?;

    AssertUnwindSafe(fut)
        .catch_unwind()
        .await
        .map_err(|e| panic_info!(name, e))
        .ok()
}

#[derive(Debug)]
pub struct WebsocketRouter {
    routes: Vec<Route>,
}

impl WebsocketRouter {
    pub fn new() -> Self {
        Self {
            routes: vec![],
        }
    }

    pub fn routes(&self) -> impl Iterator<Item = &Route> + Clone {
        self.routes.iter()
    }

    pub fn add_route(&mut self, route: Route) {
        if route.websocket_handler.is_some() {
            self.routes.push(route);
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
        req: &'r Request<'r>,
    ) -> impl Iterator<Item = &'a Route> + 'r {
        // Note that routes are presorted by ascending rank on each `add`.
        self.routes.iter().filter(move |r| r.matches(req))
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
                let dummy = Request::new(&rocket, rocket_http::Method::Get, Origin::dummy());
                let r = rocket.handle_error(Status::BadRequest, &dummy).await;
                return rocket.send_response(r, tx).await;
            }
        };
        let mut data = Data::from(h_body);

        // Dispatch the request to get a response, then write that response out.
        let _token = rocket.preprocess_request(&mut req, &mut data).await;

        let mut response = None;
        let websocket_channel = WebsocketChannel::new();
        req.local_cache(|| Some(Channel::from_websocket(&websocket_channel)));

        for route in rocket.websocket_router.route(&req) {
            req.set_route(route);

            let name = route.name.as_deref();
            let handler = route.websocket_handler.as_ref().unwrap();
            let res = handle(name, || handler.handle(&req, None)).await;
            // Successfully ran
            if let Some(Ok(_)) = res {
                response = Some((Self::create_reponse(&req), route));
                break;
            }
        }
        if let Some((response, route)) = response {
            rocket.send_response(response, tx).await;
            Self::websocket_task(rocket.clone(), &req, upgrade, route, websocket_channel).await;
        }else {
            let response = Self::handle_error(Status::NotFound);
            rocket.send_response(response, tx).await;
        }
    }

    fn create_reponse<'r>(req: &'r Request<'r>) -> Response<'r> {
        // Use websocket-codec to parse the client request
        let cl_req = match ClientRequest::parse(|n| req.headers().get_one(n)) {
            Ok(v) => v,
            Err(_e) => return Self::handle_error(Status::UpgradeRequired),
        };

        let mut response = Response::build();
        response.status(Status::SwitchingProtocols);
        response.header(Header::new(CONNECTION.as_str(), "upgrade"));
        response.header(Header::new(UPGRADE.as_str(), "websocket"));
        response.header(Header::new("Sec-WebSocket-Accept", cl_req.ws_accept()));
        response.sized_body(None, Cursor::new("Switching to websocket"));
        response.finalize()
    }

    /// Construct a rocket response from the given hyper request
    fn handle_error<'_b>(status: Status) -> Response<'_b> {
        let mut response = Response::build();
        response.status(status);
        response.finalize()
    }

    async fn websocket_task(
        _rocket: Arc<Rocket<Orbit>>,
        request: &Request<'_>,
        on_upgrade: OnUpgrade,
        route: &Route,
        mut ws: WebsocketChannel,
    ) {
        if let Ok(upgrade) = on_upgrade.await {
            //let ws = request.local_cache(|| WebsocketChannel::empty());
            //ws.add_inner(upgrade).await;
            ws.upgraded(upgrade);

            let name = route.name.as_deref();
            let handler = route.websocket_handler.as_ref().unwrap();
            while let Some(message) = ws.next().await {
                match message.opcode() {
                    Opcode::Text | Opcode::Binary => {
                        let _res = handle(name, || handler.handle(&request,
                                                          Some(Data::from(message.into_data()))
                                                    )).await;
                    }
                    Opcode::Close => break,
                    _ => (),
                }
            }
        }
    }
}
