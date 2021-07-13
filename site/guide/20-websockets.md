# WebSockets

Traditional HTTP requests must always be initiated by the client, but there are
several reasons why a server might need to send events to the client. There are
several solutions, such as polling the server at some interval, Server Sent
Events (SSE), or WebSockets.

Polling is generally frowned upon, since it uses quite a bit of bandwidth and
makes the server process extra requests even when nothing is ready to be sent.

Rocket provides an API for supporting both SSE and WebSockets. See [SSE] for
more information on how to use SSE with Rocket.

! NOTE: This page assumes a general familiarity with Rocket & WebSockets.

## WebSocket Events

The WebSocket protocol is fairly invloved, but Rocket handles much of the complexity
internally. The following is a complete echo server:

```rust
#[macro_use] extern crate rocket;

use rocket::websocket::Channel;
use rocket::response::content::Html;
use rocket::Data;

#[message("/echo", data = "<data>")]
async fn echo(data: &str, ch: &Channel<'_>) {
    ch.send(data).await;
}

#[launch]
fn rocket() -> _ {
    rocket::build().mount("/", routes![echo])
}
```

There are only a couple of differences to point out here. The use of a `message`
attribute, which isn't an HTTP Method like GET, etc. `message` is a WebSocket event;
there are two more that will be discussed later. The `message` event is run on each
incomming message. Unlike HTTP Methods, `message` cannot return a response.

Conceptually, `Channel` is just a Request Guard that provides a handle to send messages
to the client. There is a slight difference that will be discussed later.

`Data` is handled exactly the same in WebSocket and HTTP routes.

### Join Events

Join events are run immediatly after client connects. There is no data associated
with Join events, because the client has not sent any messages when this is called.
Join handlers are not able to perform authentication, see [Authentication].

### Leave Events

A Leave event is run after the client disconnects. They do support a data parameter,
but the data provided is a actually a `WebSocketStatusResult`. This represents the
returned status code from the client, although this is not the same as HTTP Status.

### A Client implementation

There are several options to create a client to connect to and test the echo server.
There are several WebSocket libraries in a variety of languages, such as the
[tungstenite](https://crates.io/crates/tungstenite) crate for Rust. Below is a simple
example using the JavaScript WebSocket API, which is provided by almost every browser.
This creates an index page which uses JavaScript to open a websocket connection,
and send messages from the browser.

```rust
#[get("/")]
fn index() -> Html<&'static str> {
    Html(r#"<!DOCTYPE html>
<html lang="en">
    <head>
        <title>WebSocket Echo Server</title>
    </head>
    <body>
        <h1>Echo Server</h1>
        <p id="status">Connecting...</p>
        <input type="text" id="text" />
        <button type="button" id="send">Send</button>
        <div id="lines"></div>
        <script type="text/javascript">
            const lines = document.getElementById('lines');
            const text = document.getElementById('text');
            const status = document.getElementById('status');
            const ws = new WebSocket('ws://' + location.host + '/echo');
            ws.onopen = function(e) {
                status.innerText = 'Connected :)';
            };
            ws.onclose = function(e) {
                status.innerText = 'Disconnected :(';
                lines.innerHTML = '';
            };
            ws.onmessage = function(msg, e) {
                const line = document.createElement('p');
                line.innerText = msg.data;
                lines.prepend(line);
            };
            send.onclick = function(e) {
                ws.send(text.value);
                text.value = '';
            };
        </script>
    </body>
</html>"#)
}
```

Don't forget to mount this route as well!

## Authentication

According to the WebSocket spec, authentication can be done using any authentication
method available to HTTP Routes. However, most client libraries don't actually provide
any authentication methods, or the flexibility to implement them yourself. The most
common solution to this issue is token based authentication: first the client authenticates
(typically on a non-websocket route), and gets a token. That token is then used to
connect to the websocket route, and carries the authentication information from the
initial authentication.

Outside of Rocket, this token is somethimes placed in a query parameter of the URI,
but that conflicts with Rocket's use of the URI as a topic identifier. To solve this,
Rocket makes use of temporary URIs. On the server side, this is almost entirely handled
by Rocket, through the `WebSocketToken` type. Here is what the authentication route
should look like.

```rust,no_run
#[post("/updates/listen", data = "<auth>")]
fn updates_auth(auth: Json<UserAuth>) -> WebSocketToken<UserAuth> {
    WebSocketToken::new(auth.into_inner())
}
```

The generic type of `WebSocketToken` is an associated data type, which carries some
object from the authentication route to the WebSocket event routes. This can be used
to carry identifiying information, such as the client's username, user id, etc.

Here is a typical event route that requires the use of a websocket token:

```rust,no_run
#[message("/updates/listen", data = "<data>")]
fn updates_message(data: &str, auth: &WebSocketToken<UserAuth>, ws: &Channel<'_>)
{ }
```

! NOTE: The `&` is required

  Since the `WebSocketToken` is passed to every websocket event handler, it must
  be passed as a reference.

## Channel

The `Channel` is actually a Channel Guard, which means it can only be used on
WebSocket Event handlers. All Request Guards are automatically Channel Guards,
but the reverse is not always true.

`Channel` has a lifetime arguement, which should almost always just be elided (`'_`).

## Broadcasts

Rocket provides a `Broker` implementation, which handles broadcasting messages to
multiple clients. `Channel` holds a reference to Rocket's broker, and has a broadcast
method which broadcasts a message to every client connected to the same topic (the
URI of the initial request). For example, if a client connects to
`ws:://rocket.rs/updates/listen`, the topic would be `/updates/listen`.
