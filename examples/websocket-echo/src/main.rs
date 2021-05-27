#[macro_use] extern crate rocket;

use rocket::channels::Channel;
use rocket::response::content::Html;
use rocket::Data;

#[message("/echo", "<data>")]
async fn echo(data: Data, websocket: Channel<'_>) {
    websocket.send(data).await;
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
            var arr = [];
            for(let i = 0; i < 100; i++) {
                arr.push(i);
            }
        </script>
    </body>
</html>"#)
}

#[launch]
fn rocket() -> _ {
    rocket::build().mount("/", routes![echo, index])
}
