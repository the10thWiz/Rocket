#[macro_use] extern crate rocket;

use rocket::channels::{Websocket, Channel};
use rocket::response::content::Html;
use rocket::State;

#[websocket("/listen", "<data>")]
async fn listen(data: String, websocket: Websocket, channel: &State<Channel>) {
    channel.broadcast(data);
}

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
            const ws = new WebSocket('ws://' + location.host + '/listen');
            ws.onopen = function(e) {
                status.innerText = 'Connected :)';
                ws.send("listen:global");
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

#[launch]
fn rocket() -> _ {
    rocket::build().mount("/", routes![listen, index])
}
