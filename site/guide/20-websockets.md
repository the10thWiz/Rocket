# Websockets

Unlike traditional web requests, websockets allow bidirectional communication.
This means that servers can send events to clients, which is often useful,
especially for things like chat rooms.

## Event handlers

There are three types of events triggered during a websocket's lifetime. The only
required event is `message`, which is triggered for every incoming message. This
is a simple echo example

```rust
#[message("/echo", data = "<data>")]
async fn echo(data: Data, ws: Channel<'_>) {
  ws.send(data).await;
}
```

There are few things going on here. First, websockets have a path and query, just
like any other web request. The data attribute will be the contents of the
message sent. There are a few differences though, specifically in relation to
request guards. Every request guard will work in a websocket channel, however the
reverse is not always true. In fact, `Channel` can only be used a guard in
websocket handlers.

### Other events

The other two events are `join` and `leave` events, triggered when a client connects
and disconnects, respectively. They behave in much the same way as `message`, although
they don't have a data parameter.

! note: Join and Leave events are optional

  As mentioned above, `join` and `leave` events are not required. In order for a
  client to sucsefully connect, a `join` event handler must sucseed, or no join
  handlers match. Be careful to make sure that the `join` handlers are at least as
  broad as the `message` handlers.

In may be the case the `message` is optional, and a `join` handler may be enough.

### Mounting event handlers

Websocket Event Handlers are attached to Rocket in the same way as any other route.
In fact, Websocket Event Handlers actually also generate an HTTP route, to provide
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
open a seperate websocket connection for every topic they wish to connect to.
There are two ways to reduce the number of open TCP connections: Websockets
over HTTP/2, or topic multiplexing. Rocket supports HTTP/2, but the underlying
libraries don't support the required features for websocket connections, and
only the most recent browsers have support for websocket over HTTP/2.

Rocket-Multiplex is the topic multiplexing protocol Rocket supports. It's a
light-weight, simple protocol, and largely transparent. Rocket has support for
rocket-multiplex out of the box, and a JS library for web browsers is provided.

Rocket-multiplex does have a few limitations; the request (and associated headers,
etc) are copied to each topic subscribed to, although this is typically the
case anyway.

For those who want or need to implement rocket-multiplex for another language, the
full specification can be found in
[`rocket::channels::rocket_multiplex`](@api/rocket/channels/rocket_multiplex/)
