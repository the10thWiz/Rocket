//use figment::{Figment, Profile};
use rocket::Config;
use rocket_http::uri::{Absolute, Asterisk, Authority, Origin, Reference};
use serde::{Serialize, Deserialize};
//use pretty_assertions::assert_eq;

#[derive(PartialEq, Debug, Serialize, Deserialize)]
struct UriContainer<'a> {
    asterisk: Asterisk,
    #[serde(borrow)]
    origin: Origin<'a>,
    #[serde(borrow)]
    authority: Authority<'a>,
    #[serde(borrow)]
    absolute: Absolute<'a>,
    #[serde(borrow)]
    reference: Reference<'a>,
}

#[test]
fn uri_deserialize() {
    figment::Jail::expect_with(|jail| {
        jail.create_file("Rocket.toml", r#"
            [default]
            asterisk = "*"
            origin = "/foo/bar?baz"
            authority = "user:pass@rocket.rs:80"
            absolute = "https://rocket.rs/foo/bar"
            reference = "https://rocket.rs:8000/index.html"
        "#)?;

        let uris: UriContainer<'_> = Config::figment().extract()?;
        assert_eq!(uris, UriContainer {
            asterisk: Asterisk,
            origin: Origin::new("/foo/bar", Some("baz")),
            authority: Authority::const_new(Some("user:pass"), "rocket.rs", Some(80)),
            absolute: Absolute::const_new(
                "https",
                Some(Authority::const_new(None, "rocket.rs", None)),
                "/foo/bar",
                None),
            reference: Reference::const_new(
                Some("https"),
                Some(Authority::const_new(None, "rocket.rs", Some(8000))),
                "/index.html",
                None, None),
        });
        Ok(())
    });
}
