#[macro_use] extern crate rocket;

use rocket::http::Status;
use rocket::websocket::Channel;
use rocket::response::content::Html;
use rocket::websocket::token::WebSocketToken;
use rocket::{State, Data};
use dashmap::DashSet;
use rocket::serde::{Deserialize, Serialize};
use rocket::serde::json::Json;

#[get("/")]
fn index() -> Html<&'static str> {
    Html(r#"<!DOCTYPE html>
<html lang="en">
   <head>
        <title>WebSocket Channel Server</title>
    </head>
    <body>
        <h1>Channel Server</h1>
        <p>Messages sent from here will be sent to every connected client</p>
        <p id="status">Connecting...</p>
        <input type="text" id="text" />
        <button type="button" id="send">Send</button>
        <div id="lines"></div>
        <script type="text/javascript">
            const lines = document.getElementById('lines');
            const text = document.getElementById('text');
            const status = document.getElementById('status');
            // Example get_connection code. It takes a username
            var get_connection = function(name, conn, error) {
                // This creates a web request, and adds the appropriate event listeners
                let req = new XMLHttpRequest();
                req.addEventListener("load", (e) => {
                    if (e.target.status != 200) {
                        // If it failed
                        error(e);
                    } else {
                        // Create the actual connection, and pass it to conn
                        let connection =
                           new WebSocket('ws://' + document.location.host + e.target.responseText);
                        conn(connection);
                    }
                });
                req.addEventListener('error', error);
                // This is the POST request to the auth route below
                req.open("POST", document.location + "/listen");
                // This adds the body. A simpler API might not require JSON, but we do.
                req.send(JSON.stringify({ 'name': name }));
            };
            // This is primarily to prompt the user for a username
            var get = function() {
                get_connection(prompt('Username'), (ws) => {
                    // this callback is called on success
                    ws.addEventListener('open', (e) => {
                        status.innerText = 'Connected :';
                    });
                    ws.addEventListener('close', (e) => {
                        status.innerText = 'Disconnected :(';
                        lines.innerHTML = '';
                    });
                    ws.addEventListener('message', (msg) => {
                        const line = document.createElement('p');
                        line.innerText = msg.data;
                        lines.prepend(line);
                    });
                    send.addEventListener('click', (e) => {
                        ws.send(text.value);
                        text.value = '';
                    });
                    // this callback is called on failure; we just retry
                }, (e) => get());
            };
            get();
        </script>
    </body>
</html>"#)
}

/// User struct (this version just contains the name, but a more complete app might also add
/// profile picture, user id, etc)
///
/// This derives Eq and Hash so it can be used in the DashSet (a concurrent HashSet)
#[derive(Debug, PartialEq, Eq, Hash, Default, Serialize, Deserialize, Clone)]
#[serde(crate = "rocket::serde")]
struct User {
    name: String,
}

// This is the authenication route. By default, WebSocketToken will use the Origin URI of this
// route as the URI of the resulting websocket connection, regardless of the actualy URI the client
// used.
#[post("/listen", data = "<data>")]
fn auth(data: Json<User>, users: &State<DashSet<User>>) -> Result<WebSocketToken<User>, Status> {
    // users.insert will return false if the Username is already taken
    if users.insert(data.clone()) {
        // Return WebSocketToken with the requested username
        Ok(WebSocketToken::new(data.into_inner()))
    } else {
        Err(Status::Conflict)
    }
}

// There is no permitted Data for join - any data that need to be passed (like the username here)
// must be passed via WebSocketTokens
//
// This event should only be used for sending a join message to other people connected; no actual
// authentication logic should be here.
//
// TODO: This is currently required (due to a todo!() in rocket), some additional work needs to be
// done to the data attributes.
#[join("/listen")]
async fn join(ws: &Channel<'_>, user: &WebSocketToken<User>) {
    ws.broadcast(format!("{} Joined", user.get_data().name)).await;
}

// This is more or less the same as before
#[message("/listen", data = "<data>")]
async fn listen(data: &str, ws: &Channel<'_>, _user: &WebSocketToken<User>) {
    ws.broadcast(data).await;
}

// Same here
#[leave("/listen")]
async fn leave(ws: &Channel<'_>, user: &WebSocketToken<User>, users: &State<DashSet<User>>) {
    ws.broadcast(format!("{} Left", user.get_data().name)).await;
    users.remove(user.get_data());
}

#[launch]
fn rocket() -> _ {
    rocket::build()
       .manage(DashSet::<User>::new())
       .mount("/", routes![auth, join, listen, leave, index])
}
