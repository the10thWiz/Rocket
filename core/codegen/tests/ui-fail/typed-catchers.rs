#[macro_use] extern crate rocket;
use rocket::request::Request;

fn main() {}
struct Foo;

#[catch(default, error = "<foo>")]
fn isnt_ref(foo: std::io::Error) -> &'static str {
    ""
}

#[catch(default, error = "<foo>")]
fn isnt_ref_or_error(foo: Foo) -> &'static str {
    ""
}

// TODO: Ideally, this error message shouldn't mention `Transient`, but
// I don't think it's avoidable. It does mention `TypedError` first.
#[catch(default, error = "<foo>")]
fn doesnt_implement_error(foo: &Foo) -> &'static str {
    ""
}

#[catch(default)]
fn doesnt_implement_from_error(foo: Foo) -> &'static str {
    ""
}

#[catch(default)]
fn request_by_value(foo: Request<'_>) -> &'static str {
    ""
}
