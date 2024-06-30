mod parse;

use devise::ext::SpanDiagnosticExt;
use devise::{Diagnostic, Level, Result, Spanned};
use proc_macro2::{TokenStream, Span};

use crate::http_codegen::Optional;
use crate::syn_ext::ReturnTypeExt;
use crate::exports::*;

fn error_arg_ty(arg: &syn::FnArg) -> Result<&syn::Type> {
    match arg {
        syn::FnArg::Receiver(_) => Err(Diagnostic::spanned(
            arg.span(),
            Level::Error,
            "Catcher cannot have self as a parameter",
        )),
        syn::FnArg::Typed(syn::PatType { ty, .. }) => match ty.as_ref() {
            syn::Type::Reference(syn::TypeReference { elem, .. }) => Ok(elem.as_ref()),
            _ => Err(Diagnostic::spanned(
                ty.span(),
                Level::Error,
                "Error type must be a reference",
            )),
        },
    }
}

pub fn _catch(
    args: proc_macro::TokenStream,
    input: proc_macro::TokenStream
) -> Result<TokenStream> {
    // Parse and validate all of the user's input.
    let catch = parse::Attribute::parse(args.into(), input)?;

    // Gather everything we'll need to generate the catcher.
    let user_catcher_fn = &catch.function;
    let user_catcher_fn_name = &catch.function.sig.ident;
    let vis = &catch.function.vis;
    let status_code = Optional(catch.status.map(|s| s.code));
    let deprecated = catch.function.attrs.iter().find(|a| a.path().is_ident("deprecated"));

    // Determine the number of parameters that will be passed in.
    if catch.function.sig.inputs.len() > 3 {
        return Err(catch.function.sig.paren_token.span.join()
            .error("invalid number of arguments: must be zero, one, or two")
            .help("catchers optionally take `&Request` or `Status, &Request`"));
    }

    // This ensures that "Responder not implemented" points to the return type.
    let return_type_span = catch.function.sig.output.ty()
        .map(|ty| ty.span())
        .unwrap_or_else(Span::call_site);

    // TODO: how to handle request?
    //   - Right now: (), (&Req), (Status, &Req) allowed
    // Set the `req` and `status` spans to that of their respective function
    // arguments for a more correct `wrong type` error span. `rev` to be cute.
    let codegen_args = &[__req, __status, __error];
    let inputs = catch.function.sig.inputs.iter().rev()
        .zip(codegen_args.iter())
        .map(|(fn_arg, codegen_arg)| match fn_arg {
            syn::FnArg::Receiver(_) => codegen_arg.respanned(fn_arg.span()),
            syn::FnArg::Typed(a) => codegen_arg.respanned(a.ty.span())
        }).rev();
    let (make_error, error_type) = if catch.function.sig.inputs.len() >= 3 {
        let arg = catch.function.sig.inputs.first().unwrap();
        let ty = error_arg_ty(arg)?;
        (quote_spanned!(arg.span() =>
            let #__error: &#ty = match ::rocket::catcher::downcast(__error_init.as_ref()) {
                Some(v) => v,
                None => return #_Result::Err((#__status, __error_init)),
            };
        ), quote! {Some((#_catcher::TypeId::of::<#ty>(), ::std::any::type_name::<#ty>()))})
    } else {
        (quote! {}, quote! {None})
    };

    // We append `.await` to the function call if this is `async`.
    let dot_await = catch.function.sig.asyncness
        .map(|a| quote_spanned!(a.span() => .await));

    let catcher_response = quote_spanned!(return_type_span => {
        let ___responder = #user_catcher_fn_name(#(#inputs),*) #dot_await;
        #_response::Responder::respond_to(___responder, #__req).map_err(|s| (s, __error_init))?
    });

    // Generate the catcher, keeping the user's input around.
    Ok(quote! {
        #user_catcher_fn

        #[doc(hidden)]
        #[allow(nonstandard_style)]
        /// Rocket code generated proxy structure.
        #deprecated #vis struct #user_catcher_fn_name {  }

        /// Rocket code generated proxy static conversion implementations.
        #[allow(nonstandard_style, deprecated, clippy::style)]
        impl #user_catcher_fn_name {
            fn into_info(self) -> #_catcher::StaticInfo {
                fn monomorphized_function<'__r>(
                    #__status: #Status,
                    #__req: &'__r #Request<'_>,
                    __error_init: #ErasedError<'__r>,
                ) -> #_catcher::BoxFuture<'__r> {
                    #_Box::pin(async move {
                        #make_error
                        let __response = #catcher_response;
                        #_Result::Ok(
                            #Response::build()
                                .status(#__status)
                                .merge(__response)
                                .finalize()
                        )
                    })
                }

                #_catcher::StaticInfo {
                    name: ::core::stringify!(#user_catcher_fn_name),
                    code: #status_code,
                    error_type: #error_type,
                    handler: monomorphized_function,
                    location: (::core::file!(), ::core::line!(), ::core::column!()),
                }
            }

            #[doc(hidden)]
            pub fn into_catcher(self) -> #Catcher {
                self.into_info().into()
            }
        }
    })
}

pub fn catch_attribute(
    args: proc_macro::TokenStream,
    input: proc_macro::TokenStream
) -> TokenStream {
    _catch(args, input).unwrap_or_else(|d| d.emit_as_item_tokens())
}
