# WebSockets

Unlike traditional web requests, WebSockets allow bidirectional communication.
This means that servers can send events to clients, which is often useful,
especially for things like chat rooms.

## Event handlers

There are three types of events triggered during a WebSocket's lifetime. The only
required event is `message`, which is triggered for every incoming message. This
is a simple echo example

```rust
#[message("/echo", data = "<data>")]
async fn echo(data: Data, ws: Channel<'_>) {
  ws.send(data).await;
}
```

There are few things going on here. First, WebSockets have a path and query, just
like any other web request. The data attribute will be the contents of the
message sent. There are a few differences though, specifically in relation to
request guards. Every request guard will work in a WebSocket channel, however the
reverse is not always true. In fact, `Channel` can only be used a guard in
WebSocket handlers.

### Other events

The other two events are `join` and `leave` events, triggered when a client connects
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

## Authentication

Any and all authentication should be done with Request Guards, just like
everywhere else in Rocket. It is highly inadvisable to rely on `join` handlers,
since it is possible for a client to connection without triggering a `join` handler.

To avoid running costly authentication procedures for every message, the request
guard can cache tokens in the requests local cache.

TODO: Example

### Mounting event handlers

WebSocket Event Handlers are attached to Rocket in the same way as any other route.
In fact, WebSocket Event Handlers actually also generate an HTTP route, to provide
error responses. These default routes are given the lowest priority possible, to
prevent collisions with existing routes.

### Channel

The `Channel` type requires a lifetime, and it can't be `'static`. This has to do
with implementation details, and the elided lifetime `'_` should almost alays be
what you want.

The `send(message)` method on `Channel` takes any parameter that implements
`IntoMessage`. `String`, `&str`, and many other types implement `IntoMessage`,
although the details of implementing `IntoMessage` are beyond the scope of this
guide.

## Broadcasts & Topics

To send a message to more than one client, use the `broadcast(message)` method.
This will broadcast the message to every client connected to the same topic.
Rocket uses the Uri used to connect as the topic. E.g., in the following
example, the topic is `'/listen'`.

```rust
#[message("/listen", data = "<data>")]
async fn echo(data: Data, ws: Channel<'_>) {
  ws.broadcast(data).await;
}
```

This handler will broadcast every message recieved to every client connected to
`'/listen'`.

Similarly, the `broadcast_to(topic, message)` method broadcasts to a message, but
it broadcasts to the topic specified, rather than the one it's responding to. When
possible, use the `uri!` macro to construct topics.

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
  - [ ] Echo guide
- [x] Broadcast example
  - [ ] Broadcast guide
- [ ] Multiplex example
  - [ ] Multiplex guide
- [ ] Broker guide
- [x] Authentication
- [ ] Compiler Errors & Warnings
- [ ] Runtime Errors & Warnings
