#[macro_use] extern crate rocket;

use rocket::websocket::Channel;
use rocket::response::content::Html;
#[message("/echo", data = "<data>", rank = 2)]
async fn echo(data: &str, websocket: &Channel<'_>) {
    websocket.send(data).await;
}

/// Authenticated route
mod auth_routes {
    use rocket::websocket::token::WebSocketToken;
    use rocket::websocket::Channel;

    #[message("/echo-auth", data = "<data>")]
    pub async fn echo_auth(data: &str, websocket: &Channel<'_>, _t: &WebSocketToken<()>) {
        websocket.send(data).await;
    }

    #[get("/echo-auth")]
    pub async fn auth() -> WebSocketToken<()> {
        // No authentication is actually done here, but it would be trivial to add
        WebSocketToken::new(())
    }
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
    rocket::build()
        .mount("/", routes![echo, index, auth_routes::auth, auth_routes::echo_auth])
}
