#[macro_use] extern crate rocket;

use rocket::websocket::Channel;
use rocket::response::content::Html;
use rocket::Data;
use rocket::websocket::token::WebSocketToken;

#[message("/echo", data = "<data>", rank = 2)]
async fn echo(data: Data<'_>, websocket: Channel<'_>) {
    websocket.send(data).await;
}

#[join("/echo", rank = 2)]
async fn echo_j() {
}

#[message("/echo-auth", data = "<data>")]
async fn echo_auth(data: Data<'_>, websocket: Channel<'_>, _t: &WebSocketToken<()>) {
    websocket.send(data).await;
}

#[join("/echo-auth")]
async fn echo_auth_j(_t: &WebSocketToken<()>) {
}

#[get("/echo-auth")]
async fn auth() -> WebSocketToken<()> {
    WebSocketToken::new(())
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

#[launch]
fn rocket() -> _ {
    rocket::build().mount("/", routes![echo, index, auth, echo_auth, echo_auth_j, echo_j])
}
