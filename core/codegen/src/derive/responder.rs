// use quote::ToTokens;
// use crate::{exports::{*, Status as _Status}, syn_ext::IdentExt};
// use devise::{*, ext::{TypeExt, SpanDiagnosticExt}};
// use crate::http_codegen::{ContentType, Status};

use syn::{Ident, Lifetime};
use quote::ToTokens;
use devise::{*, ext::{TypeExt, SpanDiagnosticExt}};
use proc_macro2::{Span, TokenStream};

use crate::{exports::{*, Status as _Status}, syn_ext::IdentExt};
use crate::syn_ext::{TypeExt as _, GenericsExt as _};
use crate::http_codegen::{ContentType, Status};


#[derive(Debug, Default, FromMeta)]
struct ItemAttr {
    content_type: Option<SpanWrapped<ContentType>>,
    status: Option<SpanWrapped<Status>>,            
}
#[derive(Default, FromMeta)]
struct FieldAttr {
    ignore: bool,
}

pub fn derive_responder(input: proc_macro::TokenStream) -> TokenStream {
    let impl_tokens = quote!(impl<'r, 'o: 'r> #_response::Responder<'r, 'o>);
    DeriveGenerator::build_for(input, impl_tokens)
        .support(Support::Struct | Support::Enum | Support::Lifetime | Support::Type)
        .replace_generic(1, 0)
        .type_bound_mapper(MapperBuild::new()
            .try_enum_map(|m, e| mapper::enum_null(m, e))
            .try_fields_map(|_, fields| {
                let generic_idents = fields.parent.input().generics().type_idents();
                let lifetime = |ty: &syn::Type| syn::Lifetime::new("'o", ty.span());
                let mut types = fields.iter()
                    .map(|f| (f, &f.field.inner.ty))
                    .map(|(f, ty)| (f, ty.with_replaced_lifetimes(lifetime(ty))));

                let mut bounds = vec![];
                if let Some((_, ty)) = types.next() {
                    if !ty.is_concrete(&generic_idents) {
                        let span = ty.span();
                        bounds.push(quote_spanned!(span => #ty: #_response::Responder<'r, 'o>));
                        bounds.push(quote_spanned!(span =>
                            <
                                <#ty as #_response::Responder<'r, 'o>>::Error
                                as #_catcher::Transient
                            >::Transience: #_catcher::CanTranscendTo<#_catcher::Inv<'r>>));
                    }
                }

                for (f, ty) in types {
                    let attr = FieldAttr::one_from_attrs("response", &f.attrs)?.unwrap_or_default();
                    if ty.is_concrete(&generic_idents) || attr.ignore {
                        continue;
                    }

                    bounds.push(quote_spanned! { ty.span() =>
                        #ty: ::std::convert::Into<#_http::Header<'o>>
                    });
                }

                Ok(quote!(#(#bounds,)*))
            })
        )
        .validator(ValidatorBuild::new()
            .input_validate(|_, i| match i.generics().lifetimes().count() > 1 {
                true => Err(i.generics().span().error("only one lifetime is supported")),
                false => Ok(())
            })
            .fields_validate(|_, fields| match fields.is_empty() {
                true => Err(fields.span().error("need at least one field")),
                false => Ok(())
            })
        )
        .inner_mapper(MapperBuild::new()
            .with_output(|_, output| quote! {
                fn respond_to(self, __req: &'r #Request<'_>)
                    -> #_response::Result<'o, Self::Error>
                {
                    #output
                }
            })
            .try_fields_map(|_, fields| {
                fn set_header_tokens<T: ToTokens + Spanned>(item: T) -> TokenStream {
                    quote_spanned!(item.span() => __res.set_header(#item);)
                }

                let error_outcome = match fields.parent {
                    FieldParent::Variant(p) => {
                        let name = p.parent.ident.append("Error");
                        let var_name = &p.ident;
                        quote! { #name::#var_name(e) }
                        // quote! { #_catcher::AnyError(#_Box::new(e)) }
                    },
                    _ => quote! { e },
                };

                let attr = ItemAttr::one_from_attrs("response", fields.parent.attrs())?
                    .unwrap_or_default();

                let responder = fields.iter().next().map(|f| {
                    let (accessor, ty) = (f.accessor(), f.ty.with_stripped_lifetimes());
                    quote_spanned! { f.span() =>
                        let mut __res = match <#ty as #_response::Responder>::respond_to(
                            #accessor, __req
                        ) {
                            #_Result::Ok(val) => val,
                            #_Result::Err(e) => return #_Result::Err(#error_outcome),
                            // #Outcome::Forward(f) => return #Outcome::Forward(f),
                        };
                    }
                }).expect("have at least one field");

                let mut headers = vec![];
                for field in fields.iter().skip(1) {
                    let attr = FieldAttr::one_from_attrs("response", &field.attrs)?
                        .unwrap_or_default();

                    if !attr.ignore {
                        headers.push(set_header_tokens(field.accessor()));
                    }
                }

                let content_type = attr.content_type.map(set_header_tokens);
                let status = attr.status.map(|status| {
                    quote_spanned!(status.span() => __res.set_status(#status);)
                });

                Ok(quote! {
                    #responder
                    #(#headers)*
                    #content_type
                    #status
                    #_Ok(__res)
                })
            })
        )
        // TODO: What's the proper way to do this?
        .inner_mapper(MapperBuild::new()
            .with_output(|_, output| quote! {
                type Error = #output;
            })
            .try_struct_map(|_, item| {
                let (old, ty) = item.fields.iter().next().map(|f| {
                    let ty = f.ty.with_replaced_lifetimes(Lifetime::new("'o", Span::call_site()));
                    let old = f.ty.with_replaced_lifetimes(Lifetime::new("'a", Span::call_site()));
                    (old, ty)
                }).expect("have at least one field");
                let type_params: Vec<_> = item.generics.type_params().map(|p| &p.ident).collect();
                let output_life = if old == ty && ty.is_concrete(&type_params) {
                    quote! { 'static }
                } else {
                    quote! { 'o }
                };

                Ok(quote! {
                    <#ty as #_response::Responder<'r, #output_life>>::Error
                })
            })
            .enum_map(|_, item| {
                let name = item.ident.append("Error");
                let type_params: Vec<_> = item.generics.type_params().map(|p| &p.ident).collect();
                let types = item.variants()
                    .map(|f| {
                        let ty = f.fields()
                        .iter()
                        .next()
                        .expect("Variants must have at least one field")
                        .ty
                        .with_replaced_lifetimes(Lifetime::new("'o", f.span()));

                        let alt = ty.with_replaced_lifetimes(Lifetime::new("'a", f.span()));
                        // TODO: typed: Check this logic - it seems to work.
                        // This isn't safety critical - if it's wrong, it causes compilation failures.
                        let output = if ty.is_concrete(&type_params) &&  alt == ty {
                            quote! { 'static }
                        } else {
                            quote! { 'o }
                        };
                        quote! { <#ty as #_response::Responder<'r, #output>>::Error, }
                    });
                quote!{ #name<'r, #(#types)*> }
            })
        )
        .outer_mapper(MapperBuild::new()
            .enum_map(|_, item| {
                let name = item.ident.append("Error");
                let type_params: Vec<Ident> =  item.variants()
                    .map(|var| var.ident.clone())
                    .collect();
                let variants_decl = type_params.iter()
                    .map(|i| quote! { #i(#i), });
                let static_params = type_params.iter()
                    .map(|i| quote! { #i::Static });
                let transient_bounds = type_params.iter()
                    .map(|i| quote! {
                        #i: #_catcher::Transient,
                        <#i as #_catcher::Transient>::Transience: #_catcher::CanTranscendTo<#_catcher::Inv<'r>>,
                    });
                let error_bounds = type_params.iter()
                    .map(|i| quote! {
                        #i: #_catcher::TypedError<'r>,
                        #i: #_catcher::Transient,
                        <#i as #_catcher::Transient>::Transience: #_catcher::CanTranscendTo<#_catcher::Inv<'r>>,
                    });
                let debug_bounds = type_params.iter()
                    .map(|i| quote! {
                        #i: ::std::fmt::Debug,
                    });
                quote!{
                    pub enum #name<'r, #(#type_params,)*> {
                        #(#variants_decl)*
                        UnusedVariant(
                            // Make this variant impossible to construct
                            ::std::convert::Infallible,
                            // Needed in case no variants use `'o`
                            ::std::marker::PhantomData<fn(&'r ())>,
                        ),
                    }
                    // SAFETY: Each variant holds a type `T` that must be
                    // - T: Transient
                    // - T::Transience: CanTranscendTo<Inv<'r>>
                    // - T: 'r
                    // The UnusedVariant has transience Co<'r>, which can transcend to Inv<'r>
                    // Because each variant can transcend to Inv<'r>, it is safe to transcend
                    // the enum as a whole to Inv<'r>
                    unsafe impl<'r, #(#type_params: 'r,)*> ::rocket::catcher::Transient
                        for #name<'r, #(#type_params,)*>
                            where #(#transient_bounds)*
                    {
                        type Static = #name<'static, #(#static_params,)*>;
                        type Transience = ::rocket::catcher::Inv<'r>;
                    }

                    impl<'r, #(#type_params,)*> ::std::fmt::Debug
                        for #name<'r, #(#type_params,)*>
                            where #(#debug_bounds)*
                    {
                        fn fmt(&self, f: &mut ::std::fmt::Formatter<'_>) -> ::std::fmt::Result {
                            match self {
                                #(Self::#type_params(v) => v.fmt(f),)*
                                Self::UnusedVariant(f, ..) => match *f { }
                            }
                        }
                    }

                    impl<'r, #(#type_params,)*> #TypedError<'r>
                        for #name<'r, #(#type_params,)*>
                            where #(#error_bounds)*
                    {
                        fn source(&'r self) -> #_Option<&dyn #TypedError<'r>> {
                            match self {
                                #(Self::#type_params(v) => v.source(),)*
                                Self::UnusedVariant(f, ..) => match *f { }
                            }
                        }

                        fn name(&self) -> &'static str {
                            match self {
                                #(Self::#type_params(v) => v.name(),)*
                                Self::UnusedVariant(f, ..) => match *f { }
                            }
                        }

                        fn respond_to(
                            &self,
                            __req: &'r #_request::Request<'_>
                        ) -> #_Result<#Response<'r>, #_Status> {
                            match self {
                                #(Self::#type_params(v) => v.respond_to(__req),)*
                                Self::UnusedVariant(f, ..) => match *f { }
                            }
                        }

                        fn status(&self) -> #_Status {
                            match self {
                                #(Self::#type_params(v) => v.status(),)*
                                Self::UnusedVariant(f, ..) => match *f { }
                            }
                        }
                    }
                }
            })
        )
        .to_tokens()
}
