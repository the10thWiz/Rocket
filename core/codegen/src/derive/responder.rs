use quote::ToTokens;
use devise::{*, ext::{TypeExt, SpanDiagnosticExt}};
use proc_macro2::{Span, TokenStream};
use syn::Lifetime;

use crate::exports::*;
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
                bounds_from_fields(fields)
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
                    -> #_response::Outcome<'o, Self::Error>
                {
                    #output
                }
            })
            .try_fields_map(|_, fields| {
                fn set_header_tokens<T: ToTokens + Spanned>(item: T) -> TokenStream {
                    quote_spanned!(item.span() => __res.set_header(#item);)
                }

                let error_outcome = match fields.parent {
                    FieldParent::Variant(_p) => {
                        // let name = p.parent.ident.append("Error");
                        // let var_name = &p.ident;
                        // quote! { #name::#var_name(e) }
                        quote! { #_catcher::AnyError(#_Box::new(e)) }
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
                            #Outcome::Success(val) => val,
                            #Outcome::Error(e) => return #Outcome::Error(#error_outcome),
                            #Outcome::Forward(f) => return #Outcome::Forward(f),
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
                    #Outcome::Success(__res)
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
            .enum_map(|_, _item| {
                // let name = item.ident.append("Error");
                // let response_types: Vec<_> = item.variants()
                //     .flat_map(|f| responder_types(f.fields()).into_iter()).collect();
                // // TODO: add where clauses, and filter for the type params I need
                // let type_params: Vec<_> = item.generics
                //     .type_params()
                //     .map(|p| &p.ident)
                //     .filter(|p| generic_used(p, &response_types))
                //     .collect();
                // quote!{ #name<'r, 'o, #(#type_params,)*> }
                quote!{ #_catcher::AnyError<'r> }
            })
        )
        // TODO: typed: We should generate this type, to avoid double-boxing the error
        // .outer_mapper(MapperBuild::new()
        //     .enum_map(|_, item| {
        //         let name = item.ident.append("Error");
        //         let variants = item.variants().map(|d| {
        //             let var_name = &d.ident;
        //             let (old, ty) = d.fields().iter().next().map(|f| {
        //                 let ty = f.ty.with_replaced_lifetimes(
        //                        Lifetime::new("'o", Span::call_site()));
        //                 (f.ty.clone(), ty)
        //             }).expect("have at least one field");
        //             let output_life = if old == ty {
        //                 quote! { 'static }
        //             } else {
        //                 quote! { 'o }
        //             };
        //             quote!{
        //                 #var_name(<#ty as #_response::Responder<'r, #output_life>>::Error),
        //             }
        //         });
        //         let source = item.variants().map(|d| {
        //             let var_name = &d.ident;
        //             quote!{
        //                 Self::#var_name(v) => #_Some(v),
        //             }
        //         });
        //         let response_types: Vec<_> = item.variants()
        //             .flat_map(|f| responder_types(f.fields()).into_iter()).collect();
        //         // TODO: add where clauses, and filter for the type params I need
        //         let type_params: Vec<_> = item.generics
        //             .type_params()
        //             .map(|p| &p.ident)
        //             .filter(|p| generic_used(p, &response_types))
        //             .collect();
        //         let bounds: Vec<_> = item.variants()
        //             .map(|f| bounds_from_fields(f.fields()).expect("Bounds must be valid"))
        //             .collect();
        //         let bounds: Vec<_> = item.variants()
        //             .flat_map(|f| responder_types(f.fields()).into_iter())
        //             .map(|t| quote!{#t: #_response::Responder<'r, 'o>,})
        //             .collect();
        //         quote!{
        //             pub enum #name<'r, 'o, #(#type_params: 'r,)*>
        //                 where #(#bounds)*
        //             {
        //                 #(#variants)*
        //                 UnusedVariant(
        //                     // Make this variant impossible to construct
        //                     ::std::convert::Infallible,
        //                     ::std::marker::PhantomData<&'o ()>,
        //                 ),
        //             }
        //             // TODO: validate this impl - roughly each variant must be (at least) inv
        //             // wrt a lifetime, since they impl CanTransendTo<Inv<'r>>
        //             // TODO: also need to add requirements on the type parameters
        //             unsafe impl<'r, 'o: 'r, #(#type_params: 'r,)*> ::rocket::catcher::Transient
        //                 for #name<'r, 'o, #(#type_params,)*>
        //                     where #(#bounds)*
        //             {
        //                 type Static = #name<'static, 'static>;
        //                 type Transience = ::rocket::catcher::Inv<'r>;
        //             }
        //             impl<'r, 'o: 'r, #(#type_params,)*> #TypedError<'r>
        //                 for #name<'r, 'o, #(#type_params,)*>
        //                     where #(#bounds)*
        //             {
        //                 fn source(&self) -> #_Option<&dyn #TypedError<'r>> {
        //                     match self {
        //                         #(#source)*
        //                         Self::UnusedVariant(f, ..) => match *f { }
        //                     }
        //                 }
        //             }
        //         }
        //     })
        // )
        .to_tokens()
}

// fn generic_used(ident: &Ident, res_types: &[Type]) -> bool {
//     res_types.iter().any(|t| !t.is_concrete(&[ident]))
// }

// fn responder_types(fields: Fields<'_>) -> Vec<Type> {
//     let generic_idents = fields.parent.input().generics().type_idents();
//     let lifetime = |ty: &syn::Type| syn::Lifetime::new("'o", ty.span());
//     let mut types = fields.iter()
//         .map(|f| (f, &f.field.inner.ty))
//         .map(|(f, ty)| (f, ty.with_replaced_lifetimes(lifetime(ty))));

//     let mut bounds = vec![];
//     if let Some((_, ty)) = types.next() {
//         if !ty.is_concrete(&generic_idents) {
//             bounds.push(ty);
//         }
//     }
//     bounds
// }

fn bounds_from_fields(fields: Fields<'_>) -> Result<TokenStream> {
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
}
