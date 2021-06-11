#[macro_use] extern crate rocket;

use rocket::channels::Channel;
use rocket::response::content::Html;
use rocket::{State, Data};
use rocket::fs::FileServer;

#[message("/listen/<_>", data = "<data>")]
async fn listen(data: Data, ws: Channel<'_>) {
    ws.broadcast(data).await;
}

#[get("/listen/<_>")]
fn other() -> Html<&'static str> {
    Html(r#"<!DOCTYPE html>
<html lang="en">
    <head>
        <title>WebSocket Multiplexed Channel Server</title>
    </head>
    <body>
        <h1>Multiplexed Channel Server</h1>
        <label>Room controls</label><input type="text" id="room" />
        <button type="button" id="add">Add</button>
        <button type="button" id="rm">Remove</button>
        <p id="status">Connecting...</p>
        <input type="text" id="text" />
        <button type="button" id="send">Send</button>
        <div id="lines"></div>
        <script type="text/javascript" src="/scripts/rocket-multiplex.js"></script>
        <script type="text/javascript">
            //document.addEventListener('load', () => {
                const lines = document.getElementById('lines');
                const text = document.getElementById('text');
                const status = document.getElementById('status');
                // location.pathname extracts the path paramteters
                const ws = new RocketWebSocket('ws://' + location.host + location.pathname);
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
            //});
        </script>
    </body>
</html>"#)
}

#[get("/")]
fn index() -> Html<&'static str> {
    Html(r#"<!DOCTYPE html>
<html lang="en">
    <head>
        <title>WebSocket Multiplexed Channel Server</title>
    </head>
    <body>
        <h1>Multiplexed Channel Server</h1>
        <label>Room controls</label><input type="text" id="room" />
        <button type="button" id="add">Add</button>
        <button type="button" id="rm">Remove</button>
        <p id="status">Connecting...</p>
        <input type="text" id="text" />
        <button type="button" id="send">Send</button>
        <div id="lines"></div>
        <script type="text/javascript" src="/scripts/rocket-multiplex.js"></script>
        <script type="text/javascript">
            //document.addEventListener('load', () => {
                const lines = document.getElementById('lines');
                const text = document.getElementById('text');
                const status = document.getElementById('status');
                const ws = new RocketWebSocket('ws://' + location.host + '/listen/global');
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
            //});
        </script>
    </body>
</html>"#)
}

#[launch]
fn rocket() -> _ {
    rocket::build()
        .mount("/", routes![listen, index, other])
        .mount("/scripts", FileServer::from("scripts/"))
}
