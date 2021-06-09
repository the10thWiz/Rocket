# WebSockets

Unlike traditional web requests, WebSockets allow bidirectional communication.
This means that servers can send events to clients, which is often useful,
especially for things like chat rooms.

For this guide, at least a passing familiarity with WebSockets and the JS
WebSocket API is assumed.

## Event handlers

There are three types of events triggered during a WebSocket's lifetime. The only
required event is `message`, which is triggered for every incoming message. This
is a simple echo example

```rust
#[macro_use] extern crate rocket;

#[message("/echo", data = "<data>")]
async fn echo(data: Data, ws: Channel<'_>) {
    ws.send(data).await;
}

#[launch]
fn rocket() -> _ {
    rocket::build().mount("/", routes![echo])
}
```

There are few things going on here. First, WebSockets have a path and query, just
like any other web request. The data attribute will be the contents of the
message sent. There are a few differences though, specifically in relation to
request guards. Every request guard will work in a WebSocket channel, however the
reverse is not always true. In fact, `Channel` can only be used a guard in
WebSocket handlers.

The above example is technically a complete example, but it's not particularly interesting.
To add functionality, an index route that contains the HTML and JS needed to connect
to the server can be added, such as the following

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
                console.log(e);
            };
            ws.onclose = function(e) {
                status.innerText = 'Disconnected :(';
                lines.innerHTML = '';
                console.log(e);
            };
            ws.onmessage = function(msg, e) {
                const line = document.createElement('p');
                line.innerText = msg.data;
                lines.prepend(line);
                console.log(e);
            };
            send.onclick = function(e) {
                ws.send(text.value);
                text.value = '';
                console.log(e);
            };
        </script>
    </body>
</html>"#)
}
```

### Authentication

Any and all authentication should be done with Request Guards, just like
everywhere else in Rocket. It is highly inadvisable to rely on `join` handlers,
since it is possible for a client to connection without triggering a `join` handler.

To avoid running costly authentication procedures for every message, the request
guard can cache tokens in the requests local cache.

See [`Request::local_cache`](@api/rocket/request/struct.Request.html#method.local_cache)
for more information.

### Other events

The other two events `join` and `leave`, are triggered when a client connects
and disconnects, respectively. They behave in much the same way as `message`,
the main difference lies in how they handle the data attribute. `join` handlers
cannot have a data attribute, and `leave` events require the data attribute to
be of the type `WebsocketStatus`.

! note: Join and Leave events are optional

  As mentioned above, `join` and `leave` events are not required. If a `join` route
  matches the request, then it must succeed for the client to connect. Otherwise,
  the request guards for the message handler will be checked.

The current implementation allows the `message` handler to be optional, as long
as there is a matching `join` handler to accept their connection.

### Mounting event handlers

WebSocket Event Handlers are attached to Rocket in the same way as any other route.
In fact, WebSocket Event Handlers actually also generate an HTTP route, to provide
error responses. These default routes are given the lowest priority possible, so
every other route will be tried first. The defualt handler just returns an
UpgradeRequired status code, to indicate that the route is actually a WebSocket Route.

### Channel

The `Channel` type requires a lifetime, and it can't be `'static`. This has to do
with implementation details, and the elided lifetime `'_` should almost alays be
what you want.

The `send(message)` method on `Channel` takes any parameter that implements
`IntoMessage`. `String`, `&str`, and many other types implement `IntoMessage`,
although the details of implementing `IntoMessage` are beyond the scope of this
guide.

## Broadcasts & Inter-client communication

The first example presented is kind of boring. All it does is send a message back
to the client that sent it. To make it more interesting, we could instead broadcast
the message to every client.

Rocket provides the capability to broadcast to multiple clients, so all we need
to do is change the `send` to `broadcast`.

```rust
#[message("/listen", data = "<data>")]
async fn echo(data: Data, ws: Channel<'_>) {
  ws.broadcast(data).await;
}
```

This turns our simple echo server into a rudimentary chat room. However, this
leads to a natural question: what if you only want to send a message to some
clients? Rocket provides a way to do this as well, via Topics. Topics are just
the Url used to connect (`'/listen'` in this case). `broadcast` doesn't actually
forward the message to every client, it only sends it clients connected to the
same topic. In fact, to create a second room, we could add a second handler with
a different Url, or we could mount the broadcast route in multiple places. The best
option (and the one we will go with) is to simply make the echo route dynamic.

```rust
#[message("/<_>", data = "<data>")]
async fn echo(data: Data, ws: Channel<'_>) {
  ws.broadcast(data).await;
}
```

The `<_>` route parameter will match anything, without requiring us to add a
parameter to the handler. Now, if two client connects to `'/listen'`, they will
still be able to see eachother's messages, but a client connected to `'/global'`
can't see them.

### Broker

It is not possible to get a `Channel` outside of an Event Handler. This is where
the `Broker` comes in. It only has the `broadcast_to` method, but it can be extracted
from the Rocket instance anywhere in the application. It is also a Request Guard,
for convience when sending messages based on requests.

## Rocket-Multiplex

There is one large drawback to the Topics outlined above: the client needs to
open a seperate WebSocket connection for every topic they wish to connect to.
There are two ways to reduce the number of open TCP connections: WebSockets
over HTTP/2, or topic multiplexing. Rocket supports HTTP/2, but the underlying
libraries don't support the required features for WebSocket connections, and
only the most recent browsers have support for WebSocket over HTTP/2.

Rocket-Multiplex is the topic multiplexing protocol Rocket supports. It's a
light-weight, simple protocol, and largely transparent. Rocket has support for
rocket-multiplex out of the box, and a JS library for web browsers is provided.

Rocket-multiplex does have a few limitations; the request (and associated headers,
etc) are copied to each topic subscribed to, although this is typically the
case anyway.

For those who want or need to implement rocket-multiplex for another language, the
full specification can be found in
[`rocket::channels::rocket_multiplex`](@api/rocket/channels/rocket_multiplex/)

## Documentation and Guide todo

- [x] Echo example
  - [x] Echo guide
- [x] Broadcast example
  - [x] Broadcast guide
- [ ] Multiplex example
  - [ ] Multiplex guide
- [ ] Broker guide
- [x] Authentication
- [ ] Compiler Errors & Warnings
- [ ] Runtime Errors
