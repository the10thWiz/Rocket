+++
summary = "a migration guide from Rocket v0.5 to v0.6"
+++

# Upgrading

This a placeholder for an eventual migration guide from v0.5 to v0.6.

## Typed Errors and Catchers

Whenever a guard fails with a type, such as a `FromParam`, `FromRequest`, etc
implementation, the error type is boxed up, and can be retrieved by catchers.
This well-requested feature makes providing useful error types much easier.

The simplest way to start is by returning a `Result` from a route. At long as
it implements `TypedError`, the `Err` variant can be caught by a typed catcher.

The following example mounts a single route, and a catcher that will catch any
invalid integer parameters. The catcher then formats the error into a JSON
object and returns it.

TODO: add tests, and make this run?
```rust,no_run
# #[macro_use] extern crate rocket;
use std::num::ParseIntError;

use rocket::Responder;
use rocket::serde::{Serialize, json::Json};
use rocket::request::FromParamError;

#[derive(Responder)]
#[response(status = 422)]
struct ParameterError<T>(T);

#[derive(Serialize)]
#[serde(crate = "rocket::serde")]
struct ErrorInfo<'a> {
    invalid_value: &'a str,
    description: String,
}

#[catch(422, error = "<int_error>")]
fn catch_invalid_int<'a>(
    // `&ParseIntError` would also work here, but `&FromParamError<ParseIntError>`
    // also gives us access to `raw`, the specific segment that failed to parse.
    int_error: &FromParamError<'a, ParseIntError>
) -> ParameterError<Json<ErrorInfo<'a>>> {
    ParameterError(Json(ErrorInfo {
        invalid_value: int_error.raw,
        description: format!("{}", int_error.error),
    }))
}

/// A simple route with a single parameter. If you pass this a valid `u8`,
/// it will return a string. However, if you pass it something invalid as a `u8`,
/// such as `-1`, `1234`, or `apple`, the catcher above will be respond with
/// details on the error.
#[get("/<number>")]
fn number(number: u8) -> String {
    format!("You selected {}", number)
}
```

To see a full demonstration of this feature, check out the error-handling
(TODO: link) example.

## Getting Help

If you run into any issues upgrading, we encourage you to ask questions via
[GitHub discussions] or via chat at [`#rocket:mozilla.org`] on Matrix. The
[FAQ](../faq/) also provides answers to commonly asked questions.

[GitHub discussions]: @github/discussions
[`#rocket:mozilla.org`]: @chat
