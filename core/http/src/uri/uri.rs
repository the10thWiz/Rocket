use std::fmt::{self, Display};
use std::convert::From;
use std::borrow::Cow;
use std::str::Utf8Error;
use std::convert::TryFrom;

use crate::RawStr;
use crate::ext::IntoOwned;
use crate::parse::Extent;
use crate::uri::{Origin, Authority, Absolute, Error};
use crate::uri::encoding::{percent_encode, DEFAULT_ENCODE_SET};

/// An `enum` encapsulating any of the possible URI variants.
///
/// # Usage
///
/// In Rocket, this type will rarely be used directly. Instead, you will
/// typically encounter URIs via the [`Origin`] type. This is because all
/// incoming requests contain origin-type URIs.
///
/// Nevertheless, the `Uri` type is typically enountered as a conversion target.
/// In particular, you will likely see generic bounds of the form: `T:
/// TryInto<Uri>` (for instance, in [`Redirect`](rocket::response::Redirect)
/// methods). This means that you can provide any type `T` that implements
/// `TryInto<Uri>`, or, equivalently, any type `U` for which `Uri` implements
/// `TryFrom<U>` or `From<U>`. These include `&str` and `String`, [`Origin`],
/// [`Authority`], and [`Absolute`].
///
/// ## Parsing
///
/// The `Uri` type implements a full, zero-allocation, zero-copy [RFC 7230]
/// compliant "request target" parser with limited liberties for real-world
/// deviations. In particular, the parser deviates as follows:
///
///   * It accepts `%` characters without two trailing hex-digits unless it is
///     the only character in the URI.
///
///   * It accepts the following additional unencoded characters in query parts,
///     to match real-world browser behavior:
///
///     `{`, `}`, `[`,  `]`, `\`,  `^`,  <code>&#96;</code>, `|`
///
/// To parse an `&str` into a `Uri`, use [`Uri::parse()`]. Alternatively, you
/// may also use the `TryFrom<&str>` and `TryFrom<String>` trait implementation.
/// To inspect the parsed type, match on the resulting `enum` and use the
/// methods of the internal structure.
///
/// [RFC 7230]: https://tools.ietf.org/html/rfc7230
///
/// ## Percent Encoding/Decoding
///
/// This type also provides the following percent encoding/decoding helper
/// methods: [`Uri::percent_encode()`], [`Uri::percent_decode()`], and
/// [`Uri::percent_decode_lossy()`].
///
/// [`Origin`]: crate::uri::Origin
/// [`Authority`]: crate::uri::Authority
/// [`Absolute`]: crate::uri::Absolute
/// [`Uri::parse()`]: crate::uri::Uri::parse()
/// [`Uri::percent_encode()`]: crate::uri::Uri::percent_encode()
/// [`Uri::percent_decode()`]: crate::uri::Uri::percent_decode()
/// [`Uri::percent_decode_lossy()`]: crate::uri::Uri::percent_decode_lossy()
///
/// ## Serde
///
/// When the `serde` feature is enabled, the Uri type implements both serialize and
/// deserialize, through the appropriate Serde traits. This allows the use of Uri
/// in configuration documents that are parsed via serde, with little to no issue.
/// 
/// The implementation currently relies on the Uri's display and parse functions,
/// and may not be compatible across mutiple Rocket versions
#[derive(Debug, PartialEq, Clone)]
pub enum Uri<'a> {
    /// An origin URI.
    Origin(Origin<'a>),
    /// An authority URI.
    Authority(Authority<'a>),
    /// An absolute URI.
    Absolute(Absolute<'a>),
    /// An asterisk: exactly `*`.
    Asterisk,
}

impl<'a> Uri<'a> {
    #[inline]
    pub(crate) unsafe fn raw_absolute(
        source: Cow<'a, [u8]>,
        scheme: Extent<&'a [u8]>,
        path: Extent<&'a [u8]>,
        query: Option<Extent<&'a [u8]>>,
    ) -> Uri<'a> {
        let origin = Origin::raw(source.clone(), path, query);
        Uri::Absolute(Absolute::raw(source.clone(), scheme, None, Some(origin)))
    }

    /// Parses the string `string` into a `Uri`. Parsing will never allocate.
    /// Returns an `Error` if `string` is not a valid URI.
    ///
    /// # Example
    ///
    /// ```rust
    /// # extern crate rocket;
    /// use rocket::http::uri::Uri;
    ///
    /// // Parse a valid origin URI (note: in practice, use `Origin::parse()`).
    /// let uri = Uri::parse("/a/b/c?query").expect("valid URI");
    /// let origin = uri.origin().expect("origin URI");
    /// assert_eq!(origin.path(), "/a/b/c");
    /// assert_eq!(origin.query().unwrap(), "query");
    ///
    /// // Invalid URIs fail to parse.
    /// Uri::parse("foo bar").expect_err("invalid URI");
    /// ```
    pub fn parse(string: &'a str) -> Result<Uri<'a>, Error<'_>> {
        crate::parse::uri::from_str(string)
    }

    /// Returns the internal instance of `Origin` if `self` is a `Uri::Origin`.
    /// Otherwise, returns `None`.
    ///
    /// # Example
    ///
    /// ```rust
    /// # extern crate rocket;
    /// use rocket::http::uri::Uri;
    ///
    /// let uri = Uri::parse("/a/b/c?query").expect("valid URI");
    /// assert!(uri.origin().is_some());
    ///
    /// let uri = Uri::parse("http://google.com").expect("valid URI");
    /// assert!(uri.origin().is_none());
    /// ```
    pub fn origin(&self) -> Option<&Origin<'a>> {
        match self {
            Uri::Origin(ref inner) => Some(inner),
            _ => None
        }
    }

    /// Returns the internal instance of `Authority` if `self` is a
    /// `Uri::Authority`. Otherwise, returns `None`.
    ///
    /// # Example
    ///
    /// ```rust
    /// # extern crate rocket;
    /// use rocket::http::uri::Uri;
    ///
    /// let uri = Uri::parse("user:pass@domain.com").expect("valid URI");
    /// assert!(uri.authority().is_some());
    ///
    /// let uri = Uri::parse("http://google.com").expect("valid URI");
    /// assert!(uri.authority().is_none());
    /// ```
    pub fn authority(&self) -> Option<&Authority<'a>> {
        match self {
            Uri::Authority(ref inner) => Some(inner),
            _ => None
        }
    }

    /// Returns the internal instance of `Absolute` if `self` is a
    /// `Uri::Absolute`. Otherwise, returns `None`.
    ///
    /// # Example
    ///
    /// ```rust
    /// # extern crate rocket;
    /// use rocket::http::uri::Uri;
    ///
    /// let uri = Uri::parse("http://google.com").expect("valid URI");
    /// assert!(uri.absolute().is_some());
    ///
    /// let uri = Uri::parse("/path").expect("valid URI");
    /// assert!(uri.absolute().is_none());
    /// ```
    pub fn absolute(&self) -> Option<&Absolute<'a>> {
        match self {
            Uri::Absolute(ref inner) => Some(inner),
            _ => None
        }
    }

    /// Returns a URL-encoded version of the string. Any reserved characters are
    /// percent-encoded.
    ///
    /// # Examples
    ///
    /// ```rust
    /// # extern crate rocket;
    /// use rocket::http::uri::Uri;
    ///
    /// let encoded = Uri::percent_encode("hello?a=<b>hi</b>");
    /// assert_eq!(encoded, "hello%3Fa%3D%3Cb%3Ehi%3C%2Fb%3E");
    /// ```
    pub fn percent_encode<S>(string: &S) -> Cow<'_, str>
        where S: AsRef<str> + ?Sized
    {
        percent_encode::<DEFAULT_ENCODE_SET>(RawStr::new(string))
    }

    /// Returns a URL-decoded version of the string. If the percent encoded
    /// values are not valid UTF-8, an `Err` is returned.
    ///
    /// # Examples
    ///
    /// ```rust
    /// # extern crate rocket;
    /// use rocket::http::uri::Uri;
    ///
    /// let decoded = Uri::percent_decode("/Hello%2C%20world%21".as_bytes());
    /// assert_eq!(decoded.unwrap(), "/Hello, world!");
    /// ```
    pub fn percent_decode<S>(bytes: &S) -> Result<Cow<'_, str>, Utf8Error>
        where S: AsRef<[u8]> + ?Sized
    {
        let decoder = percent_encoding::percent_decode(bytes.as_ref());
        decoder.decode_utf8()
    }

    /// Returns a URL-decoded version of the path. Any invalid UTF-8
    /// percent-encoded byte sequences will be replaced � U+FFFD, the
    /// replacement character.
    ///
    /// # Examples
    ///
    /// ```rust
    /// # extern crate rocket;
    /// use rocket::http::uri::Uri;
    ///
    /// let decoded = Uri::percent_decode_lossy("/Hello%2C%20world%21".as_bytes());
    /// assert_eq!(decoded, "/Hello, world!");
    /// ```
    pub fn percent_decode_lossy<S>(bytes: &S) -> Cow<'_, str>
        where S: AsRef<[u8]> + ?Sized
    {
        let decoder = percent_encoding::percent_decode(bytes.as_ref());
        decoder.decode_utf8_lossy()
    }
}

pub(crate) unsafe fn as_utf8_unchecked(input: Cow<'_, [u8]>) -> Cow<'_, str> {
    match input {
        Cow::Borrowed(bytes) => Cow::Borrowed(std::str::from_utf8_unchecked(bytes)),
        Cow::Owned(bytes) => Cow::Owned(String::from_utf8_unchecked(bytes))
    }
}

impl<'a> TryFrom<&'a str> for Uri<'a> {
    type Error = Error<'a>;

    #[inline]
    fn try_from(string: &'a str) -> Result<Uri<'a>, Self::Error> {
        Uri::parse(string)
    }
}

impl TryFrom<String> for Uri<'static> {
    type Error = Error<'static>;

    #[inline]
    fn try_from(string: String) -> Result<Uri<'static>, Self::Error> {
        // TODO: Potentially optimize this like `Origin::parse_owned`.
        Uri::parse(&string)
            .map(|u| u.into_owned())
            .map_err(|e| e.into_owned())
    }
}

impl IntoOwned for Uri<'_> {
    type Owned = Uri<'static>;

    fn into_owned(self) -> Uri<'static> {
        match self {
            Uri::Origin(origin) => Uri::Origin(origin.into_owned()),
            Uri::Authority(authority) => Uri::Authority(authority.into_owned()),
            Uri::Absolute(absolute) => Uri::Absolute(absolute.into_owned()),
            Uri::Asterisk => Uri::Asterisk
        }
    }
}

impl Display for Uri<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            Uri::Origin(ref origin) => write!(f, "{}", origin),
            Uri::Authority(ref authority) => write!(f, "{}", authority),
            Uri::Absolute(ref absolute) => write!(f, "{}", absolute),
            Uri::Asterisk => write!(f, "*")
        }
    }
}

/// The error type returned when a URI conversion fails.
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub struct TryFromUriError(());

impl fmt::Display for TryFromUriError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        "invalid conversion from general to specific URI variant".fmt(f)
    }
}

macro_rules! impl_uri_from {
    ($type:ident) => (
        impl<'a> From<$type<'a>> for Uri<'a> {
            fn from(other: $type<'a>) -> Uri<'a> {
                Uri::$type(other)
            }
        }

        impl<'a> TryFrom<Uri<'a>> for $type<'a> {
            type Error = TryFromUriError;

            fn try_from(uri: Uri<'a>) -> Result<Self, Self::Error> {
                match uri {
                    Uri::$type(inner) => Ok(inner),
                    _ => Err(TryFromUriError(()))
                }
            }
        }
    )
}

impl_uri_from!(Origin);
impl_uri_from!(Authority);
impl_uri_from!(Absolute);

mod uri_serde {
    #![cfg(feature = "serde")]
    
    pub use super::Uri;
    use std::fmt;
    use std::convert::TryFrom;
    // TODO pick import names
    use _serde::{Deserialize, Deserializer, Serialize, Serializer, de::{self, Visitor}};

    // The serialize implementation depends on the fmt::Display implementation. However, this
    // should be okay, and is likely the best option, since allocating a string is unavoidable, and
    // this method should allocate exactly one string
    impl<'a> Serialize for Uri<'a> {
        fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
        where
                S: Serializer {
            serializer.serialize_str(&format!("{}", self))
        }
    }

    struct UriVisitor;

    impl<'de> Visitor<'de> for UriVisitor {
        type Value = Uri<'de>;
        fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            write!(f, "a URI")
        }

        fn visit_borrowed_str<E>(self, v: &'de str) -> Result<Self::Value, E>
        where
                E: de::Error, {
            Uri::try_from(v).map_err(de::Error::custom)
        }
        fn visit_str<E>(self, v: &str) -> Result<Self::Value, E>
        where
                E: de::Error, {
            Uri::try_from(v.to_string()).map_err(de::Error::custom)
        }
        fn visit_string<E>(self, v: String) -> Result<Self::Value, E>
        where
                E: de::Error, {
            Uri::try_from(v).map_err(de::Error::custom)
        }
    }

    impl<'de> Deserialize<'de> for Uri<'de> {
        fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
        where
                D: Deserializer<'de> {
            deserializer.deserialize_str(UriVisitor)
        }
    }
    
    #[cfg(test)]
    mod tests {
        use super::*;
        
        use std::convert::TryFrom;
        use crate::{ext::IntoOwned, uri::{self, Origin, Authority, Absolute, Host}};
        #[derive(Debug)]
        enum TestError {
            UriError(Box<uri::Error<'static>>),
            JsonError(serde_json::Error),
        }
        impl<'a> From<uri::Error<'a>> for TestError {
            fn from(e: uri::Error<'a>) -> Self {
                Self::UriError(Box::new(e.into_owned()))
            }
        }
        impl From<serde_json::Error> for TestError {
            fn from(e: serde_json::Error) -> Self {
                Self::JsonError(e)
            }
        }

        // Test Helper function
        // 'a sould almost always be 'static, but it may be useful to not require that
        fn test_helper<'a>(raw: &str, json: &'a str, uri_obj: Uri<'a>) -> Result<(), TestError> {
            let uri = Uri::try_from(raw).expect(raw);
            assert_eq!(uri, uri_obj, "Uri::try_from did not create the expected Uri: {} != {}", raw, uri_obj);

            assert_eq!(serde_json::to_string(&uri)?, json, "Serialization did not produce the expected Json string");
            assert_eq!(serde_json::from_str::<Uri<'a>>(json)?, uri, "Deserialization from a `&str` did not produce the expected Uri");
            Ok(())
        }
        
        /// /path?query
        fn origin<'a>(path: &'a str, query: Option<&'a str>) -> Uri<'a> {
            Uri::Origin(Origin::new(path, query))
        }

        #[test]
        fn serde_origin_form() -> Result<(), TestError> {
            test_helper("/", "\"/\"", origin("/", None))?;
            test_helper("/foo", "\"/foo\"", origin("/foo", None))?;
            test_helper("/bar/foo", "\"/bar/foo\"", origin("/bar/foo", None))?;
            test_helper("/index.html", "\"/index.html\"", origin("/index.html", None))?;
            test_helper("/a/nice/url", "\"/a/nice/url\"", origin("/a/nice/url", None))?;

            test_helper("/a?param", "\"/a?param\"", origin("/a", Some("param")))?;
            test_helper("/a?param=value", "\"/a?param=value\"", origin("/a", Some("param=value")))
        }

        /// username:password@some.host:8088
        fn authority<'a>(user: Option<&'a str>, host: &'a str, port: Option<u16>) -> Uri<'a> {
            Uri::Authority(Authority::new(user, Host::Raw(host), port))
        }

        #[test]
        fn serde_authority_form() -> Result<(), TestError> {
            test_helper("example.com", "\"example.com\"", authority(None, "example.com", None))?;
            test_helper("test.example.com", "\"test.example.com\"", authority(None, "test.example.com", None))?;
            test_helper("www.example.com", "\"www.example.com\"", authority(None, "www.example.com", None))?;
            test_helper("com", "\"com\"", authority(None, "com", None))?;

            test_helper("user:pass@example.com", "\"user:pass@example.com\"", authority(Some("user:pass"), "example.com", None))?;
            test_helper("user:pass@test.example.com", "\"user:pass@test.example.com\"", authority(Some("user:pass"), "test.example.com", None))?;
            test_helper("user:pass@www.example.com", "\"user:pass@www.example.com\"", authority(Some("user:pass"), "www.example.com", None))?;
            test_helper("user:pass@com", "\"user:pass@com\"", authority(Some("user:pass"), "com", None))?;
            test_helper("username@example.com", "\"username@example.com\"", authority(Some("username"), "example.com", None))?;
            test_helper("username:long_pass)with0odd$chars@example.com", "\"username:long_pass)with0odd$chars@example.com\"", authority(Some("username:long_pass)with0odd$chars"), "example.com", None))?;
            test_helper("username0with$user:passchars@example.com", "\"username0with$user:passchars@example.com\"", authority(Some("username0with$user:passchars"), "example.com", None))?;
            test_helper("u@example.com", "\"u@example.com\"", authority(Some("u"), "example.com", None))?;

            test_helper("example.com:1", "\"example.com:1\"", authority(None, "example.com", Some(1)))?;
            test_helper("example.com:8000", "\"example.com:8000\"", authority(None, "example.com", Some(8000)))?;
            test_helper("example.com:8080", "\"example.com:8080\"", authority(None, "example.com", Some(8080)))?;
            test_helper("example.com:65333", "\"example.com:65333\"", authority(None, "example.com", Some(65333)))?;

            test_helper("user:pass@test.example.com:123", "\"user:pass@test.example.com:123\"", authority(Some("user:pass"), "test.example.com", Some(123)))?;
            test_helper("user:pass@www.example.com:123", "\"user:pass@www.example.com:123\"", authority(Some("user:pass"), "www.example.com", Some(123)))?;
            test_helper("user:pass@com:123", "\"user:pass@com:123\"", authority(Some("user:pass"), "com", Some(123)))?;
            test_helper("username@example.com:123", "\"username@example.com:123\"", authority(Some("username"), "example.com", Some(123)))?;
            test_helper("username:long_pass)with0odd$chars%@example.com:123", "\"username:long_pass)with0odd$chars%@example.com:123\"", authority(Some("username:long_pass)with0odd$chars%"), "example.com", Some(123)))?;
            test_helper("username0with$user:passchars@example.com:123", "\"username0with$user:passchars@example.com:123\"", authority(Some("username0with$user:passchars"), "example.com", Some(123)))?;

            test_helper("u@example.com:1", "\"u@example.com:1\"", authority(Some("u"), "example.com", Some(1)))?;
            test_helper("u@example.com:8000", "\"u@example.com:8000\"", authority(Some("u"), "example.com", Some(8000)))?;
            test_helper("u@example.com:8080", "\"u@example.com:8080\"", authority(Some("u"), "example.com", Some(8080)))?;
            test_helper("u@example.com:65333", "\"u@example.com:65333\"", authority(Some("u"), "example.com", Some(65333)))
        }

        /// http://user:pass@domain.com:4444/path?query
        fn absolute<'a>(scheme: &'a str, user: Option<&'a str>, host: Option<&'a str>, port: Option<u16>, path: Option<&'a str>, query: Option<&'a str>) -> Uri<'a> {
            Uri::Absolute(Absolute::new(scheme, host.map(|host| Authority::new(user, Host::Raw(host), port)), path.map(|path| Origin::new(path, query))))
        }

        #[test]
        fn serde_absolute_form() -> Result<(), TestError> {
            // `Some("")` and `Some("/")` are needed because of how the Uri is parsed. This could
            // potentially be fixed in the Uri's implementation of Eq, where an origin or authority
            // of None could be interperted as "/" or "" as appropriate
            test_helper("http:///", "\"http:///\"", absolute("http", None, Some(""), None, Some("/"), None))?;
            test_helper("http://", "\"http://\"", absolute("http", None, Some(""), None, None, None))?;
            test_helper("https:///", "\"https:///\"", absolute("https", None, Some(""), None, Some("/"), None))?;
            test_helper("snmp:///", "\"snmp:///\"", absolute("snmp", None, Some(""), None, Some("/"), None))?;

            test_helper("http://example.com/", "\"http://example.com/\"", absolute("http", None, Some("example.com"), None, Some("/"), None))?;
            test_helper("http://topleveldomain/", "\"http://topleveldomain/\"", absolute("http", None, Some("topleveldomain"), None, Some("/"), None))?;
            test_helper("http://sub.example.com/", "\"http://sub.example.com/\"", absolute("http", None, Some("sub.example.com"), None, Some("/"), None))?;
            test_helper("http://user@example.com/", "\"http://user@example.com/\"", absolute("http", Some("user"), Some("example.com"), None, Some("/"), None))?;
            test_helper("http://user:pass@example.com/", "\"http://user:pass@example.com/\"", absolute("http", Some("user:pass"), Some("example.com"), None, Some("/"), None))?;
            test_helper("http://example.com:80/", "\"http://example.com:80/\"", absolute("http", None, Some("example.com"), Some(80), Some("/"), None))?;
            test_helper("http://example.com:123/", "\"http://example.com:123/\"", absolute("http", None, Some("example.com"), Some(123), Some("/"), None))?;
            test_helper("http://example.com:65333/", "\"http://example.com:65333/\"", absolute("http", None, Some("example.com"), Some(65333), Some("/"), None))?;
            test_helper("http://user@example.com:80/", "\"http://user@example.com:80/\"", absolute("http", Some("user"), Some("example.com"), Some(80), Some("/"), None))?;
            test_helper("http://user:pass@example.com:80/", "\"http://user:pass@example.com:80/\"", absolute("http", Some("user:pass"), Some("example.com"), Some(80), Some("/"), None))?;

            test_helper("http://example.com", "\"http://example.com\"", absolute("http", None, Some("example.com"), None, None, None))?;
            test_helper("http://topleveldomain", "\"http://topleveldomain\"", absolute("http", None, Some("topleveldomain"), None, None, None))?;
            test_helper("http://sub.example.com", "\"http://sub.example.com\"", absolute("http", None, Some("sub.example.com"), None, None, None))?;
            test_helper("http://user@example.com", "\"http://user@example.com\"", absolute("http", Some("user"), Some("example.com"), None, None, None))?;
            test_helper("http://user:pass@example.com", "\"http://user:pass@example.com\"", absolute("http", Some("user:pass"), Some("example.com"), None, None, None))?;
            test_helper("http://example.com:80", "\"http://example.com:80\"", absolute("http", None, Some("example.com"), Some(80), None, None))?;
            test_helper("http://example.com:123", "\"http://example.com:123\"", absolute("http", None, Some("example.com"), Some(123), None, None))?;
            test_helper("http://example.com:65333", "\"http://example.com:65333\"", absolute("http", None, Some("example.com"), Some(65333), None, None))?;
            test_helper("http://user@example.com:80", "\"http://user@example.com:80\"", absolute("http", Some("user"), Some("example.com"), Some(80), None, None))?;
            test_helper("http://user:pass@example.com:80", "\"http://user:pass@example.com:80\"", absolute("http", Some("user:pass"), Some("example.com"), Some(80), None, None))?;

            test_helper("http:///path", "\"http:///path\"", absolute("http", None, Some(""), None, Some("/path"), None))?;
            test_helper("http:///path/", "\"http:///path/\"", absolute("http", None, Some(""), None, Some("/path/"), None))?;
            test_helper("http:///path/with/segments", "\"http:///path/with/segments\"", absolute("http", None, Some(""), None, Some("/path/with/segments"), None))?;
            test_helper("http:///path/with/segments/", "\"http:///path/with/segments/\"", absolute("http", None, Some(""), None, Some("/path/with/segments/"), None))?;
            test_helper("http:///index.html", "\"http:///index.html\"", absolute("http", None, Some(""), None, Some("/index.html"), None))?;
            test_helper("http:///path?query", "\"http:///path?query\"", absolute("http", None, Some(""), None, Some("/path"), Some("query")))?;
            test_helper("http:///path?query=value", "\"http:///path?query=value\"", absolute("http", None, Some(""), None, Some("/path"), Some("query=value")))?;
            test_helper("http:///index.html?query", "\"http:///index.html?query\"", absolute("http", None, Some(""), None, Some("/index.html"), Some("query")))?;
            test_helper("http:///index.html?query=value", "\"http:///index.html?query=value\"", absolute("http", None, Some(""), None, Some("/index.html"), Some("query=value")))?;
            test_helper("http:///index.html?query", "\"http:///index.html?query\"", absolute("http", None, Some(""), None, Some("/index.html"), Some("query")))?;
            test_helper("http:///path/with/segments/?query=value", "\"http:///path/with/segments/?query=value\"", absolute("http", None, Some(""), None, Some("/path/with/segments/"), Some("query=value")))
        }
        
        #[test]
        fn serde_asterisk_form() -> Result<(), TestError> {
            test_helper("*", "\"*\"", Uri::Asterisk)
        }
        

        // Commented out tests currently fail
        #[test]
        fn serde_invalid_uri() {
            serde_json::from_str::<Uri<'static>>("\"@\"").expect_err("Invalid uri");
            serde_json::from_str::<Uri<'static>>("\"#\"").expect_err("Invalid uri");
            serde_json::from_str::<Uri<'static>>("\"%\"").expect_err("Invalid uri");
            serde_json::from_str::<Uri<'static>>("\"[\"").expect_err("Invalid uri");
            serde_json::from_str::<Uri<'static>>("\"]\"").expect_err("Invalid uri");
            serde_json::from_str::<Uri<'static>>("\"{\"").expect_err("Invalid uri");
            serde_json::from_str::<Uri<'static>>("\"}\"").expect_err("Invalid uri");
            serde_json::from_str::<Uri<'static>>("\"<\"").expect_err("Invalid uri");
            serde_json::from_str::<Uri<'static>>("\">\"").expect_err("Invalid uri");
            serde_json::from_str::<Uri<'static>>("\" \"").expect_err("Invalid uri");
            serde_json::from_str::<Uri<'static>>("\"^\"").expect_err("Invalid uri");
            serde_json::from_str::<Uri<'static>>("\"|\"").expect_err("Invalid uri");
            serde_json::from_str::<Uri<'static>>("\"\\\"").expect_err("Invalid uri");
            serde_json::from_str::<Uri<'static>>("\"\t\"").expect_err("Invalid uri");
            serde_json::from_str::<Uri<'static>>("\"?\"").expect_err("Invalid uri");
            serde_json::from_str::<Uri<'static>>("\":\"").expect_err("Invalid uri");
            serde_json::from_str::<Uri<'static>>("\"\"").expect_err("Invalid uri");
        }

        #[test]
        fn serde_empty_parts() {
            // Empty Query
            serde_json::from_str::<Uri<'static>>("\"example.com/test?\"").expect_err("Invalid uri");
            //serde_json::from_str::<Uri<'static>>("\"/test?\"").expect_err("Invalid uri");
            // Empty port - `:` is interperted as part of the domain name, but shouldn't be
            //serde_json::from_str::<Uri<'static>>("\"example.com:/test\"").expect_err("Invalid uri");
            // Empty userinfo
            serde_json::from_str::<Uri<'static>>("\"@example.com/test\"").expect_err("Invalid uri");
            // Empty scheme - is interpered with scheme: ``
            // Probably valid!
            //serde_json::from_str::<Uri<'static>>("\"://example.com/test\"").expect_err("Invalid uri");
            
            // The empty scheme is likely valid, but the userinfo and port should make them errors
            //serde_json::from_str::<Uri<'static>>("\"://@example.com/test\"").expect_err("Invalid uri");
            //serde_json::from_str::<Uri<'static>>("\"://example.com:/test\"").expect_err("Invalid uri");
            //serde_json::from_str::<Uri<'static>>("\"://@example.com:/test\"").expect_err("Invalid uri");
            //serde_json::from_str::<Uri<'static>>("\"://example.com/test?\"").expect_err("Invalid uri");
            //serde_json::from_str::<Uri<'static>>("\"://example.com:/test?\"").expect_err("Invalid uri");
            //serde_json::from_str::<Uri<'static>>("\"://@example.com/test?\"").expect_err("Invalid uri");
            //serde_json::from_str::<Uri<'static>>("\"://@example.com:/test?\"").expect_err("Invalid uri");
            serde_json::from_str::<Uri<'static>>("\"@example.com:/test\"").expect_err("Invalid uri");
            serde_json::from_str::<Uri<'static>>("\"@example.com/test?\"").expect_err("Invalid uri");
            serde_json::from_str::<Uri<'static>>("\"http?\"").expect_err("Invalid uri");
            //serde_json::from_str::<Uri<'static>>("\"/http?\"").expect_err("Invalid uri");
            //serde_json::from_str::<Uri<'static>>("\"http:?\"").expect_err("Invalid uri");
        }

        #[test]
        fn serde_quirks() -> Result<(), TestError> {
            // These are valid, albiet unusual Uris
            // `http:/` => scheme: `http`, origin: `/`, authority: None
            test_helper("http:/", "\"http:/\"", absolute("http", None, None, None, Some("/"), None))?;
            // `http:` => scheme: `http`, origin: ``, authority: None
            test_helper("http:", "\"http:\"", absolute("http", None, None, None, Some(""), None))?;
            Ok(())
        }
    }
}

