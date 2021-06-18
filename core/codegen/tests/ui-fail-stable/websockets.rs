#[macro_use] extern crate rocket;

#[join("/echo")]
fn join() -> &'static str {
    ""
}

#[message("/echo")]
fn message() -> &'static str {
    ""
}

#[leave("/echo")]
fn leave() -> &'static str {
    ""
}

#[join("/echo", format = "text/json")]
fn join_format() {
}

#[message("/echo", format = "text/json")]
fn message_format()  {
}

#[leave("/echo", format = "text/json")]
fn leave_format() {
}

fn main() { }
