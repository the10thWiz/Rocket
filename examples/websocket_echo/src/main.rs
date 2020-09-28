#[macro_use]
extern crate rocket;

use rocket::http::ContentType;
use rocket::response::Content;
use rocket::tungstenite::WsUpgrade;

#[get("/echo", data = "<data>")]
async fn echo(data: rocket::Data, upgrade: WsUpgrade<'_>) -> rocket::Response<'_> {
    tokio::spawn(async move {
        let ws_stream = rocket::tungstenite::on_upgrade(data, None)
            .await
            .expect("upgrade failed");

        use crate::rocket::futures::StreamExt;
        let (write, read) = ws_stream.split();
        read.take_while(|m| futures::future::ready(m.as_ref().unwrap().is_text()))
            .forward(write)
            .await
            .expect("failed to forward message");
    });

    upgrade.accept()
}

#[get("/")]
fn index() -> Content<&'static str> {
    Content(
        ContentType::HTML,
        r#"<!DOCTYPE html>
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

            ws.onopen = function() {
                status.innerText = 'Connected :)';
            };

            ws.onclose = function() {
                status.innerText = 'Disconnected :(';
                lines.innerHTML = '';
            };

            ws.onmessage = function(msg) {
                const line = document.createElement('p');
                line.innerText = msg.data;
                lines.prepend(line);
            };

            send.onclick = function() {
                ws.send(text.value);
                text.value = '';
            };
        </script>
    </body>
</html>"#,
    )
}

#[launch]
fn rocket() -> rocket::Rocket {
    rocket::ignite().mount("/", routes![index, echo])
}
