mod parse;

use std::hash::Hash;
use std::net::IpAddr;

use crate::attribute::param::{Guard, Parameter};
use crate::http_codegen::{Optional, WebSocketEvent};
use crate::proc_macro_ext::StringLit;
use crate::syn_ext::{IdentExt, TypeExt as _};
use devise::ext::TypeExt as _;
use devise::{Diagnostic, FromMeta, Result, SpanWrapped, Spanned};
use proc_macro2::{Literal, Span, TokenStream};

use self::parse::{
    Attribute, MethodAttribute, OriginPattern, OriginPatternPart, OriginPatterns, Route,
};

impl Route {
    pub fn guards(&self) -> impl Iterator<Item = &Guard> {
        self.param_guards()
            .chain(self.query_guards())
            .chain(self.request_guards.iter())
    }

    pub fn param_guards(&self) -> impl Iterator<Item = &Guard> {
        self.path_params.iter().filter_map(|p| p.guard())
    }

    pub fn query_guards(&self) -> impl Iterator<Item = &Guard> {
        self.query_params.iter().filter_map(|p| p.guard())
    }
}

fn query_decls(route: &Route) -> Option<TokenStream> {
    use devise::ext::{Split2, Split6};

    if route.query_params.is_empty() && route.query_guards().next().is_none() {
        return None;
    }

    define_spanned_export!(Span::call_site() =>
        __req, __data, _log, _form, Outcome, _Ok, _Err, _Some, _None
    );

    // Record all of the static parameters for later filtering.
    let (raw_name, raw_value) = route
        .query_params
        .iter()
        .filter_map(|s| s.r#static())
        .map(|name| match name.find('=') {
            Some(i) => (&name[..i], &name[i + 1..]),
            None => (name.as_str(), ""),
        })
        .split2();

    // Now record all of the dynamic parameters.
    let (name, matcher, ident, init_expr, push_expr, finalize_expr) = route
        .query_guards()
        .map(|guard| {
            let (name, ty) = (&guard.name, &guard.ty);
            let ident = guard.fn_ident.rocketized().with_span(ty.span());
            let matcher = match guard.trailing {
                true => quote_spanned!(name.span() => _),
                _ => quote!(#name),
            };

            define_spanned_export!(ty.span() => FromForm, _form);

            let ty = quote_spanned!(ty.span() => <#ty as #FromForm>);
            let init = quote_spanned!(ty.span() => #ty::init(#_form::Options::Lenient));
            let finalize = quote_spanned!(ty.span() => #ty::finalize(#ident));
            let push = match guard.trailing {
                true => quote_spanned!(ty.span() => #ty::push_value(&mut #ident, _f)),
                _ => quote_spanned!(ty.span() => #ty::push_value(&mut #ident, _f.shift())),
            };

            (name, matcher, ident, init, push, finalize)
        })
        .split6();

    #[allow(non_snake_case)]
    Some(quote! {
        let (#(#ident),*) = {
            let mut __e = #_form::Errors::new();
            #(let mut #ident = #init_expr;)*

            for _f in #__req.query_fields() {
                let _raw = (_f.name.source().as_str(), _f.value);
                let _key = _f.name.key_lossy().as_str();
                match (_raw, _key) {
                    // Skip static parameters so <param..> doesn't see them.
                    #(((#raw_name, #raw_value), _) => { /* skip */ },)*
                    #((_, #matcher) => #push_expr,)*
                    _ => { /* in case we have no trailing, ignore all else */ },
                }
            }

            #(
                let #ident = match #finalize_expr {
                    #_Ok(_v) => #_Some(_v),
                    #_Err(_err) => {
                        __e.extend(_err.with_name(#_form::NameView::new(#name)));
                        #_None
                    },
                };
            )*

            if !__e.is_empty() {
                #_log::warn_!("Query string failed to match route declaration.");
                for _err in __e { #_log::warn_!("{}", _err); }
                return #Outcome::Forward(#__data);
            }

            (#(#ident.unwrap()),*)
        };
    })
}

fn request_guard_decl(guard: &Guard, websocket: bool) -> TokenStream {
    let (ident, ty) = (guard.fn_ident.rocketized(), &guard.ty);
    define_spanned_export!(ty.span() =>
        __req, __data, _request, _log, FromRequest, FromWebSocket, Outcome
    );

    let conversion = if websocket {
        quote_spanned!(ty.span() => <#ty as #FromWebSocket>::from_websocket(#__req))
    } else {
        quote_spanned!(ty.span() => <#ty as #FromRequest>::from_request(#__req))
    };

    quote_spanned! { ty.span() =>
        let #ident: #ty = match #conversion.await {
            #Outcome::Success(__v) => __v,
            #Outcome::Forward(_) => {
                #_log::warn_!("Request guard `{}` is forwarding.", stringify!(#ty));
                return #Outcome::Forward(#__data);
            },
            #Outcome::Failure((__c, __e)) => {
                #_log::warn_!("Request guard `{}` failed: {:?}.", stringify!(#ty), __e);
                return #Outcome::Failure(__c);
            }
        };
    }
}

fn param_guard_decl(guard: &Guard, _websocket: bool) -> TokenStream {
    let (i, name, ty) = (guard.index, &guard.name, &guard.ty);
    define_spanned_export!(ty.span() =>
        __req, __data, _log, _None, _Some, _Ok, _Err,
        Outcome, FromSegments, FromParam
    );

    // Returned when a dynamic parameter fails to parse.
    let parse_error = quote!({
        #_log::warn_!("Parameter guard `{}: {}` is forwarding: {:?}.",
            #name, stringify!(#ty), __error);

        #Outcome::Forward(#__data)
    });

    // All dynamic parameters should be found if this function is being called;
    // that's the point of statically checking the URI parameters.
    let expr = match guard.trailing {
        false => quote_spanned! { ty.span() =>
            match #__req.routed_segment(#i) {
                #_Some(__s) => match <#ty as #FromParam>::from_param(__s) {
                    #_Ok(__v) => __v,
                    #_Err(__error) => return #parse_error,
                },
                #_None => {
                    #_log::error_!("Internal invariant broken: dyn param not found.");
                    #_log::error_!("Please report this to the Rocket issue tracker.");
                    #_log::error_!("https://github.com/SergioBenitez/Rocket/issues");
                    return #Outcome::Forward(#__data);
                }
            }
        },
        true => quote_spanned! { ty.span() =>
            match <#ty as #FromSegments>::from_segments(#__req.routed_segments(#i..)) {
                #_Ok(__v) => __v,
                #_Err(__error) => return #parse_error,
            }
        },
    };

    let ident = guard.fn_ident.rocketized();
    quote!(let #ident: #ty = #expr;)
}

fn data_guard_decl(guard: &Guard, websocket: &WebSocketEvent) -> TokenStream {
    let (ident, ty) = (guard.fn_ident.rocketized(), &guard.ty);
    define_spanned_export!(ty.span() =>
        _log, __req, __data, FromData, Outcome, WebSocketData, _route
    );

    let (method, data) = match websocket {
        WebSocketEvent::WebSocketJoin => (
            quote!(from_ws),
            quote! {
                let #__data = match #__data {
                    #WebSocketData::Join => (),
                    data => return #Outcome::Forward(data),
                };
            },
        ),
        WebSocketEvent::WebSocketMessage => (
            quote!(from_ws),
            quote! {
                let mut #__data = match #__data {
                    #WebSocketData::Join => return #Outcome::Success(()),
                    #WebSocketData::Message(data) => data,
                    data => return #Outcome::Forward(data),
                };
            },
        ),
        WebSocketEvent::WebSocketLeave => {
            return quote_spanned! { ty.span() =>
                let #__data = match #__data {
                    #WebSocketData::Leave(data) => data.into(),
                    data => return #Outcome::Forward(data),
                };
            }
        }
        WebSocketEvent::Http(_) => (quote!(from_data), quote! {}),
    };

    quote_spanned! { ty.span() =>
        #data
        let mut #__data = #__data;
        let #ident: #ty = match <#ty as #FromData>::#method(#__req, #__data).await {
            #Outcome::Success(__d) => __d,
            #Outcome::Forward(__d) => {
                #_log::warn_!("Data guard `{}` is forwarding.", stringify!(#ty));
                return #Outcome::Forward(__d.into());
            }
            #Outcome::Failure((__c, __e)) => {
                #_log::warn_!("Data guard `{}` failed: {:?}.", stringify!(#ty), __e);
                return #Outcome::Failure(__c.into());
            }
        };
    }
}

fn internal_uri_macro_decl(route: &Route) -> TokenStream {
    // FIXME: Is this the right order? Does order matter?
    let uri_args = route
        .param_guards()
        .chain(route.query_guards())
        .map(|guard| (&guard.fn_ident, &guard.ty))
        .map(|(ident, ty)| quote!(#ident: #ty));

    // Generate a unique macro name based on the route's metadata.
    let macro_name = route.handler.sig.ident.prepend(crate::URI_MACRO_PREFIX);
    let inner_macro_name = macro_name.uniqueify_with(|mut hasher| {
        route.handler.sig.ident.hash(&mut hasher);
        route.attr.uri.path().hash(&mut hasher);
        route.attr.uri.query().hash(&mut hasher)
    });

    let route_uri = route.attr.uri.to_string();

    quote_spanned! { Span::call_site() =>
        #[doc(hidden)]
        #[macro_export]
        /// Rocket generated URI macro.
        macro_rules! #inner_macro_name {
            ($($token:tt)*) => {{
                rocket::rocket_internal_uri!(#route_uri, (#(#uri_args),*), $($token)*)
            }};
        }

        #[doc(hidden)]
        pub use #inner_macro_name as #macro_name;
    }
}

fn responder_outcome_expr(route: &Route, websocket: bool) -> TokenStream {
    let ret_span = match route.handler.sig.output {
        syn::ReturnType::Default => route.handler.sig.ident.span(),
        syn::ReturnType::Type(_, ref ty) => ty.span(),
    };

    let user_handler_fn_name = &route.handler.sig.ident;
    let parameter_names = route
        .arguments
        .map
        .values()
        .map(|(ident, _)| ident.rocketized());

    let _await = route
        .handler
        .sig
        .asyncness
        .map(|a| quote_spanned!(a.span() => .await));

    define_spanned_export!(ret_span => __req, _route, Outcome);

    if websocket {
        quote_spanned! { ret_span =>
            let ___responder: () = #user_handler_fn_name(#(#parameter_names),*) #_await;
            #Outcome::Success(())
        }
    } else {
        quote_spanned! { ret_span =>
            let ___responder = #user_handler_fn_name(#(#parameter_names),*) #_await;
            #_route::Outcome::from(#__req, ___responder).into()
        }
    }
}

fn sentinels_expr(route: &Route) -> TokenStream {
    let ret_ty = match route.handler.sig.output {
        syn::ReturnType::Default => None,
        syn::ReturnType::Type(_, ref ty) => Some(ty.with_stripped_lifetimes()),
    };

    let generic_idents: Vec<_> = route
        .handler
        .sig
        .generics
        .type_params()
        .map(|p| &p.ident)
        .collect();

    // Note: for a given route, we need to emit a valid graph of eligble
    // sentinels. This means that we don't have broken links, where a child
    // points to a parent that doesn't exist. The concern is that the
    // `is_concrete()` filter will cause a break in the graph.
    //
    // Here's a proof by cases for why this can't happen:
    //    1. if `is_concrete()` returns `false` for a (valid) type, it returns
    //       false for all of its parents. we consider this an axiom; this is
    //       the point of `is_concrete()`. the type is filtered out, so the
    //       theorem vacously holds
    //    2. if `is_concrete()` returns `true`, for a type `T`, it either:
    //      * returns `false` for the parent. by 1) it will return false for
    //        _all_ parents of the type, so no node in the graph can consider,
    //        directly or indirectly, `T` to be a child, and thus there are no
    //        broken links; the thereom holds
    //      * returns `true` for the parent, and so the type has a parent, and
    //      the theorem holds.
    //    3. these are all the cases. QED.

    const TY_MACS: &[&str] = &["ReaderStream", "TextStream", "ByteStream", "EventStream"];

    fn ty_mac_mapper(tokens: &TokenStream) -> Option<syn::Type> {
        use crate::bang::typed_stream::Input;

        match syn::parse2(tokens.clone()).ok()? {
            Input::Type(ty, ..) => Some(ty),
            Input::Tokens(..) => None,
        }
    }

    let eligible_types = route
        .guards()
        .map(|guard| &guard.ty)
        .chain(ret_ty.as_ref().into_iter())
        .flat_map(|ty| ty.unfold_with_ty_macros(TY_MACS, ty_mac_mapper))
        .filter(|ty| ty.is_concrete(&generic_idents))
        .map(|child| (child.parent, child.ty));

    let sentinel = eligible_types.map(|(parent, ty)| {
        define_spanned_export!(ty.span() => _sentinel);

        match parent {
            Some(p) if p.is_concrete(&generic_idents) => {
                quote_spanned!(ty.span() => #_sentinel::resolve!(#ty, #p))
            }
            Some(_) | None => quote_spanned!(ty.span() => #_sentinel::resolve!(#ty)),
        }
    });

    quote!(::std::vec![#(#sentinel),*])
}

fn ip_quote(ip: &IpAddr) -> TokenStream {
    match ip {
        IpAddr::V6(ip) => {
            let segments = ip.segments();
            let segments = segments.iter().map(|&u| Literal::u16_suffixed(u));
            quote! {
                [#(#segments)*]
            }
        }
        IpAddr::V4(ip) => {
            let segments = ip.octets();
            let segments = segments.iter().map(|&u| Literal::u8_suffixed(u));
            quote! {
                [#(#segments)*]
            }
        }
    }
}

fn port_quote(port: &Option<u16>) -> TokenStream {
    match port {
        None => quote! {},
        Some(port) => quote! { && port == Some(#port) },
    }
}

fn domain_match_quote(domain: &Vec<OriginPatternPart>) -> TokenStream {
    // How to match?
    let parts = domain.iter().rev().map(|p| match p {
        OriginPatternPart::Domain(d) => quote! {
            if !parts.next().map_or(false, |d| ::rocket::http::uncased::eq(d, #d)) { false } else
        },
        OriginPatternPart::Wildcard => quote! {
            if !parts.next().is_some() { false } else
        },
    });
    quote! {
        {
            let mut parts = domain.as_str().rsplit('.');
            #(#parts)* {
                true
            }
        }
    }
    //quote! { true }
}

fn allowed_origins_guard(origins: &OriginPatterns, websocket: bool) -> TokenStream {
    // TODO
    define_spanned_export!(origins.span =>
        _log, __req, __data, Outcome, _route
    );

    let mut ips = origins.patterns.iter().filter_map(|pat| match pat {
        OriginPattern::IP { ip, port, span } => {
            let ip = ip_quote(ip);
            let port = port_quote(port);
            Some(quote_spanned! { *span =>
                if ip == ::std::net::IpAddr::from(#ip) #port {
                    // Origin allowed
                } else
            })
        }
        _ => None,
    });
    // Only attempt to parse as an IP addr if at least one ip pattern exists
    let ips = if let Some(cond) = ips.next() {
        quote! {
            else if let Ok(ip) = domain.as_str().parse::<::std::net::IpAddr>() {
                #cond #(#ips)* {
                    return #Outcome::Forward(#__data);
                }
            }
        }
    } else {
        quote! {}
    };

    let domains = origins.patterns.iter().filter_map(|pat| match pat {
        OriginPattern::Domain { domain, port, span } => {
            let domain_match = domain_match_quote(domain);
            let port = port_quote(port);
            Some(quote_spanned! { *span =>
                if #domain_match #port {
                    // Origin allowed
                } else
            })
        }
        _ => None,
    });

    let request = if websocket {
        quote!(#__req.request())
    } else {
        quote!(#__req)
    };

    // How to match on a list of
    quote! {
        if let Some(host) = #request.host() {
            let mut domain = host.domain();
            // Trim final dot
            if domain.as_str().ends_with('.') {
                domain = &domain[..domain.len() - 1];
            }
            let port = host.port();
            if domain == "localhost" || domain == "127.0.0.1" {
                // Allowed
            } #ips else {
                #(#domains)* {
                    return #Outcome::Forward(#__data);
                }
            }
        } else {
            return #Outcome::Forward(#__data);
        }
    }
}

fn monomorphized_function(route: &Route) -> TokenStream {
    use crate::exports::*;

    // Generate the declarations for all of the guards.
    let origin_guard = route
        .attr
        .allowed_origins
        .as_ref()
        .map(|o| allowed_origins_guard(o, route.attr.method.is_websocket()))
        .or_else(|| {
            if route.attr.method.is_websocket() {
                Some(quote! { (); }) //todo!("Cookies must be disabled"); })
            } else {
                None
            }
        });
    let request_guards = route
        .request_guards
        .iter()
        .map(|r| request_guard_decl(r, route.attr.method.is_websocket()));
    let param_guards = route
        .param_guards()
        .map(|r| param_guard_decl(r, route.attr.method.is_websocket()));
    let query_guards = query_decls(&route);
    let data_guard = route
        .data_guard
        .as_ref()
        .map(|r| data_guard_decl(r, &route.attr.method));

    let responder_outcome = responder_outcome_expr(&route, route.attr.method.is_websocket());

    let (request, box_future, datatype) = if route.attr.method.is_websocket() {
        (WebSocket, quote!(BoxWsFuture), WebSocketData)
    } else {
        (Request, quote!(BoxFuture), Data)
    };

    quote! {
        fn monomorphized_function<'__r>(
            #__req: &'__r #request<'_>,
            #__data: #datatype<'__r>,
        ) -> #_route::#box_future<'__r> {
            #_Box::pin(async move {
                #origin_guard
                #(#request_guards)*
                #(#param_guards)*
                #query_guards
                #data_guard

                #responder_outcome
            })
        }
    }
}

fn codegen_route(route: Route) -> Result<TokenStream> {
    use crate::exports::*;

    // Check for various websocket specific errors. Although this could be implemented via a
    // seperate attr type, this method allows the errors to be customized.
    if route.attr.method.is_websocket() {
        if let syn::ReturnType::Type(arrow, _) = &route.handler.sig.output {
            return Err(Diagnostic::spanned(
                arrow.span(),
                devise::Level::Error,
                "WebSocket Event handlers are not permitted to return values",
            ));
        }
        if let Some(media_type) = &route.attr.format {
            return Err(Diagnostic::spanned(
                media_type.span(),
                devise::Level::Error,
                "WebSocket Event handlers do not support formats (yet)",
            ));
        }
        let mut iter = route.path_params.iter();
        match iter.next() {
            Some(Parameter::Static(name)) if name == "websocket" => match iter.next() {
                Some(Parameter::Static(name)) if name == "token" => match iter.next() {
                    Some(Parameter::Dynamic(_)) => match iter.next() {
                        None => {
                            return Err(Diagnostic::spanned(
                                route.attr.uri.path_span().span(),
                                devise::Level::Error,
                                "`/websocket/token/<_>` conflicts with websocket tokens",
                            ))
                        }
                        _ => (),
                    },
                    _ => (),
                },
                _ => (),
            },
            _ => (),
        }
    }

    // Extract the sentinels from the route.
    let sentinels = sentinels_expr(&route);

    // Gather info about the function.
    let (vis, handler_fn) = (&route.handler.vis, &route.handler);
    let handler_fn_name = &handler_fn.sig.ident;
    let internal_uri_macro = internal_uri_macro_decl(&route);

    let method = route.attr.method.http_method();
    let method = quote_spanned!(route.attr.method.span() => #method);
    let uri = route.attr.uri.to_string();
    let rank = Optional(route.attr.rank);
    let format = Optional(route.attr.format.as_ref());

    let function = monomorphized_function(&route);

    let (handler, websocket_handler) = if route.attr.method.is_websocket() {
        let ws = route.attr.method;
        (
            quote!(upgrade_required),
            quote!(#ws(monomorphized_function)),
        )
    } else {
        (
            quote!(monomorphized_function),
            quote!(#WebSocketEvent::None),
        )
    };

    Ok(quote! {
        #handler_fn

        #[doc(hidden)]
        #[allow(non_camel_case_types)]
        /// Rocket code generated proxy structure.
        #vis struct #handler_fn_name {  }

        /// Rocket code generated proxy static conversion implementations.
        impl #handler_fn_name {
            #[allow(non_snake_case, unreachable_patterns, unreachable_code)]
            fn into_info(self) -> #_route::StaticInfo {
                #function

                #[allow(dead_code)]
                fn upgrade_required<'__r>(
                    #__req: &'__r #Request<'_>,
                    #__data: #Data<'__r>,
                ) -> #_route::BoxFuture<'__r> {
                    #_Box::pin(async move {
                        #_route::Outcome::from(#__req, #Status::UpgradeRequired)
                    })
                }

                #_route::StaticInfo {
                    name: stringify!(#handler_fn_name),
                    method: #method,
                    uri: #uri,
                    handler: #handler,
                    websocket_handler: #websocket_handler,
                    format: #format,
                    rank: #rank,
                    sentinels: #sentinels,
                }
            }

            #[doc(hidden)]
            pub fn into_route(self) -> #Route {
                self.into_info().into()
            }
        }

        /// Rocket code generated wrapping URI macro.
        #internal_uri_macro
    })
}

fn complete_route(args: TokenStream, input: TokenStream) -> Result<TokenStream> {
    let function: syn::ItemFn = syn::parse2(input)
        .map_err(Diagnostic::from)
        .map_err(|diag| diag.help("`#[route]` can only be used on functions"))?;

    let attr_tokens = quote!(route(#args));
    let attribute = Attribute::from_meta(&syn::parse2(attr_tokens)?)?;
    codegen_route(Route::from(attribute, function)?)
}

fn incomplete_route(
    method: WebSocketEvent,
    args: TokenStream,
    input: TokenStream,
) -> Result<TokenStream> {
    let method_str = method.to_string().to_lowercase();
    // FIXME(proc_macro): there should be a way to get this `Span`.
    let method_span = StringLit::new(format!("#[{}]", method), Span::call_site())
        .subspan(2..2 + method_str.len());

    let method_ident = syn::Ident::new(&method_str, method_span);

    let function: syn::ItemFn = syn::parse2(input)
        .map_err(Diagnostic::from)
        .map_err(|d| d.help(format!("#[{}] can only be used on functions", method_str)))?;

    let full_attr = quote!(#method_ident(#args));
    let method_attribute = MethodAttribute::from_meta(&syn::parse2(full_attr)?)?;

    let attribute = Attribute {
        method: SpanWrapped {
            full_span: method_span,
            key_span: None,
            span: method_span,
            value: method,
        },
        uri: method_attribute.uri,
        data: method_attribute.data,
        format: method_attribute.format,
        rank: method_attribute.rank,
        allowed_origins: method_attribute.allowed_origins,
    };

    codegen_route(Route::from(attribute, function)?)
}

pub fn route_attribute<M: Into<Option<N>>, N: Into<WebSocketEvent>>(
    method: M,
    args: proc_macro::TokenStream,
    input: proc_macro::TokenStream,
) -> TokenStream {
    let result = match method.into() {
        Some(method) => incomplete_route(method.into(), args.into(), input.into()),
        None => complete_route(args.into(), input.into()),
    };

    result.unwrap_or_else(|diag| diag.emit_as_item_tokens())
}
