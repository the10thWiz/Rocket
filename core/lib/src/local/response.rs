macro_rules! getter_method {
    ($doc_prelude:literal, $desc:literal, $f:ident -> $r:ty) => (
        getter_method!(@$doc_prelude, $f, $desc, $r,
            concat!("let ", stringify!($f), " = response.", stringify!($f), "();"));
    );
    (@$doc_prelude:literal, $f:ident, $desc:expr, $r:ty, $use_it:expr) => (
        /// Returns the
        #[doc = $desc]
        /// of `self`.
        ///
        /// # Example
        ///
        /// ```rust
        #[doc = $doc_prelude]
        ///
        /// # Client::_test(|_, _, response| {
        /// let response: LocalResponse = response;
        #[doc = $use_it]
        /// # });
        /// ```
        #[inline(always)]
        pub fn $f(&self) -> $r {
            self._response().$f()
        }
    )
}

macro_rules! pub_response_impl {
    ($doc_prelude:literal $($prefix:tt $suffix:tt)?) =>
{
    getter_method!($doc_prelude, "HTTP status",
        status -> crate::http::Status);

    getter_method!($doc_prelude, "Content-Type, if a valid one is set,",
        content_type -> Option<crate::http::ContentType>);

    getter_method!($doc_prelude, "HTTP headers",
        headers -> &crate::http::HeaderMap<'_>);

    /// Return a cookie jar containing the HTTP cookies in the response.
    ///
    /// # Example
    ///
    /// ```rust
    #[doc = $doc_prelude]
    ///
    /// # Client::_test(|_, _, response| {
    /// let response: LocalResponse = response;
    /// let string = response.cookies();
    /// # });
    /// ```
    #[inline(always)]
    pub fn cookies(&self) -> &crate::http::CookieJar<'_> {
        self._cookies()
    }

    getter_method!($doc_prelude, "response body, if there is one,",
        body -> &crate::response::Body<'_>);

    /// Consumes `self` and reads the entirety of its body into a string.
    ///
    /// If reading fails, the body contains invalid UTF-8 characters, or the
    /// body is unset in the response, returns `None`. Otherwise, returns
    /// `Some`. The string may be empty if the body is empty.
    ///
    /// # Example
    ///
    /// ```rust
    #[doc = $doc_prelude]
    ///
    /// # Client::_test(|_, _, response| {
    /// let response: LocalResponse = response;
    /// let string = response.into_string();
    /// # });
    /// ```
    #[inline(always)]
    pub $($prefix)? fn into_string(self) -> Option<String> {
        if self._response().body().is_none() {
            return None;
        }

        self._into_string() $(.$suffix)? .ok()
    }

    /// Consumes `self` and reads the entirety of its body into a `Vec` of
    /// bytes.
    ///
    /// If reading fails or the body is unset in the response, returns `None`.
    /// Otherwise, returns `Some`. The returned vector may be empty if the body
    /// is empty.
    ///
    /// # Example
    ///
    /// ```rust
    #[doc = $doc_prelude]
    ///
    /// # Client::_test(|_, _, response| {
    /// let response: LocalResponse = response;
    /// let bytes = response.into_bytes();
    /// # });
    /// ```
    #[inline(always)]
    pub $($prefix)? fn into_bytes(self) -> Option<Vec<u8>> {
        if self._response().body().is_none() {
            return None;
        }

        self._into_bytes() $(.$suffix)? .ok()
    }

    /// Consumes `self` and deserializes its body as JSON without buffering in
    /// memory.
    ///
    /// If deserialization fails or the body is unset in the response, returns
    /// `None`. Otherwise, returns `Some`.
    ///
    /// # Example
    ///
    /// ```rust
    #[doc = $doc_prelude]
    /// use rocket::serde::Deserialize;
    ///
    /// #[derive(Deserialize)]
    /// struct Task {
    ///     id: usize,
    ///     complete: bool,
    ///     text: String,
    /// }
    ///
    /// # Client::_test(|_, _, response| {
    /// let response: LocalResponse = response;
    /// let task = response.into_json::<Task>();
    /// # });
    /// ```
    #[cfg(feature = "json")]
    #[cfg_attr(nightly, doc(cfg(feature = "json")))]
    pub $($prefix)? fn into_json<T>(self) -> Option<T>
        where T: Send + serde::de::DeserializeOwned + 'static
    {
        if self._response().body().is_none() {
            return None;
        }

        self._into_json() $(.$suffix)?
    }

    /// Consumes `self` and deserializes its body as MessagePack without
    /// buffering in memory.
    ///
    /// If deserialization fails or the body is unset in the response, returns
    /// `None`. Otherwise, returns `Some`.
    ///
    /// # Example
    ///
    /// ```rust
    #[doc = $doc_prelude]
    /// use rocket::serde::Deserialize;
    ///
    /// #[derive(Deserialize)]
    /// struct Task {
    ///     id: usize,
    ///     complete: bool,
    ///     text: String,
    /// }
    ///
    /// # Client::_test(|_, _, response| {
    /// let response: LocalResponse = response;
    /// let task = response.into_msgpack::<Task>();
    /// # });
    /// ```
    #[cfg(feature = "msgpack")]
    #[cfg_attr(nightly, doc(cfg(feature = "msgpack")))]
    pub $($prefix)? fn into_msgpack<T>(self) -> Option<T>
        where T: Send + serde::de::DeserializeOwned + 'static
    {
        if self._response().body().is_none() {
            return None;
        }

        self._into_msgpack() $(.$suffix)?
    }

    /// Checks if a route was routed by a specific route type. This only returns true if the route
    /// actually generated a response, and a catcher was not run.
    ///
    /// # Example
    ///
    /// ```rust
    /// # use rocket::get;
    /// #[get("/")]
    /// fn index() -> &'static str { "Hello World" }
    #[doc = $doc_prelude]
    /// # Client::_test(|_, _, response| {
    /// let response: LocalResponse = response;
    /// assert!(response.routed_by::<index>())
    /// # });
    /// ```
    ///
    /// # Other Route types
    ///
    /// [`FileServer`](crate::fs::FileServer) implementes `RouteType`, so a route that should
    /// return a static file can be checked against it. Libraries which provide a Route type should
    /// implement `RouteType`, see [`RouteType`](crate::route::RouteType) for more information.
    pub fn routed_by<T: crate::route::RouteType>(&self) -> bool {
        if let Some(route_type) = self._request().route().map(|r| r.route_type).flatten() {
            route_type == std::any::TypeId::of::<T>()
        } else {
            false
        }
    }

    /// Checks if a route was caught by a specific route type
    ///
    /// # Example
    ///
    /// ```rust
    /// # use rocket::get;
    /// #[catch(404)]
    /// fn default_404() -> &'static str { "Hello World" }
    #[doc = $doc_prelude]
    /// # Client::_test(|_, _, response| {
    /// let response: LocalResponse = response;
    /// assert!(response.caught_by::<default_404>())
    /// # });
    /// ```
    ///
    /// # Rocket's default catcher
    ///
    /// The default catcher has a `CatcherType` of [`DefaultCatcher`](crate::catcher::DefaultCatcher)
    pub fn caught_by<T: crate::catcher::CatcherType>(&self) -> bool {
        if let Some(catcher_type) = self._request().catcher().map(|r| r.catcher_type).flatten() {
            catcher_type == std::any::TypeId::of::<T>()
        } else {
            false
        }
    }

    /// Checks if a route was caught by a catcher
    ///
    /// # Example
    ///
    /// ```rust
    /// # use rocket::get;
    #[doc = $doc_prelude]
    /// # Client::_test(|_, _, response| {
    /// let response: LocalResponse = response;
    /// assert!(response.was_caught())
    /// # });
    /// ```
    pub fn was_caught(&self) -> bool {
        self._request().catcher().is_some()
    }

    #[cfg(test)]
    #[allow(dead_code)]
    fn _ensure_impls_exist() {
        fn is_debug<T: std::fmt::Debug>() {}
        is_debug::<Self>();
    }
}}
