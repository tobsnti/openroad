use darling::FromAttributes;
use proc_macro2::{Ident, TokenStream};
use proc_macro_error::abort;
use quote::{format_ident, quote, quote_spanned};
use syn::spanned::Spanned;
use syn::{Data, Expr, Field, Fields, Type};

use crate::{
    get_type_of, get_variant_value, is_other, PacketArgs, PacketFieldArgs, UsedType,
    DEFAULT_LIST_TYPE,
};

pub(crate) fn deserialize(ident: &Ident, data: &Data, args: PacketArgs) -> TokenStream {
    match *data {
        Data::Struct(ref struct_data) => match &struct_data.fields {
            Fields::Named(named) => {
                let idents = named
                    .named
                    .iter()
                    .map(|field| field.ident.as_ref().expect("i failed"))
                    .collect::<Vec<&Ident>>();
                let content = named.named.iter().map(|field| {
                    generate_reader_for(field, field.ident.as_ref().expect("i failed"))
                });
                quote_spanned! {ident.span()=>
                    #(#content)*
                    Ok(#ident { #(#idents),* })
                }
            }
            Fields::Unnamed(unnamed) => {
                let idents = (0..unnamed.unnamed.len())
                    .map(|i| format_ident!("t{}", i))
                    .collect::<Vec<Ident>>();
                let content = unnamed
                    .unnamed
                    .iter()
                    .zip(&idents)
                    .map(|(field, ident)| generate_reader_for(field, ident));
                quote_spanned! { ident.span() =>
                    #(#content)*
                    Ok(#ident(#(#idents),*))
                }
            }
            Fields::Unit => {
                quote_spanned! { ident.span() =>
                    Ok(#ident)
                }
            }
        },
        Data::Enum(ref enum_data) => {
            let enum_size = args.size.unwrap_or(1);
            let mut others = enum_data
                .variants
                .iter()
                .filter(|variant| is_other(variant));
            let other = others.next();
            if let Some(second) = others.next() {
                abort!(
                    second,
                    "only one variant can be `#[sro_packet(other)]`: it is read for every value \
                     without a variant of its own"
                );
            }
            // The `other` arm matches every value, so it comes last whatever
            // the declaration order.
            let variants = enum_data
                .variants
                .iter()
                .filter(|variant| !is_other(variant))
                .chain(other);
            let arms = variants.map(|variant| {
                let field_args =
                    PacketFieldArgs::from_attributes(&variant.attrs).expect("i failed");
                // The `other` variant keeps the value it matched in its first
                // field, which is bound to it instead of read.
                let (pattern, skip) = if field_args.other {
                    if variant.fields.is_empty() {
                        abort!(
                            variant,
                            "an `#[sro_packet(other)]` variant needs a first field for the value"
                        );
                    }
                    (quote!(unknown), 1)
                } else {
                    let value = get_variant_value(
                        &variant.ident,
                        field_args.value.expect("Missing value for variant."),
                        enum_size,
                    );
                    (quote!(#value), 0)
                };
                let variant_ident = &variant.ident;

                match &variant.fields {
                    Fields::Named(named) => {
                        let idents = named
                            .named
                            .iter()
                            .map(|field| field.ident.as_ref().expect("i failed"))
                            .collect::<Vec<&Ident>>();
                        let bound = &idents[..skip];
                        let content = named.named.iter().skip(skip).map(|field| {
                            generate_reader_for(field, field.ident.as_ref().expect("i failed"))
                        });
                        quote_spanned! { variant_ident.span() =>
                            #pattern => {
                                #(let #bound = unknown;)*
                                #(#content)*
                                Ok(#ident::#variant_ident { #(#idents),* })
                            }
                        }
                    }
                    Fields::Unnamed(unnamed) => {
                        let idents = (0..unnamed.unnamed.len())
                            .map(|i| format_ident!("t{}", i))
                            .collect::<Vec<Ident>>();
                        let bound = &idents[..skip];
                        let content = unnamed
                            .unnamed
                            .iter()
                            .zip(&idents)
                            .skip(skip)
                            .map(|(field, ident)| generate_reader_for(field, ident));
                        quote_spanned! { variant_ident.span() =>
                            #pattern => {
                                #(let #bound = unknown;)*
                                #(#content)*
                                Ok(#ident::#variant_ident(#(#idents),*))
                            }
                        }
                    }
                    Fields::Unit => {
                        quote_spanned! { variant_ident.span() =>
                            #pattern => Ok(#ident::#variant_ident)
                        }
                    }
                }
            });

            let variant_string = format!("{}", ident);
            let unknown_arm = other.is_none().then(|| {
                quote_spanned! { ident.span() =>
                    unknown => Err(sro_macro::SerializationError::UnknownVariation(unknown as usize, #variant_string)),
                }
            });
            let size = args.size.unwrap_or(1);
            let reader = match size {
                1 => quote_spanned!(ident.span() => u8::read_from(reader)?),
                2 => quote_spanned!(ident.span() => u16::read_from(reader)?),
                4 => quote_spanned!(ident.span() => u32::read_from(reader)?),
                8 => quote_spanned!(ident.span() => u64::read_from(reader)?),
                _ => abort!(ident, "Invalid size"),
            };
            quote_spanned! { ident.span() =>
                match #reader {
                    #(#arms,)*
                    #unknown_arm
                }
            }
        }
        _ => abort!(ident, "Unions are not supported."),
    }
}

fn generate_reader_for(field: &Field, ident: &Ident) -> TokenStream {
    let typ = get_type_of(&field.ty);
    let type_name = &field.ty;
    let args = PacketFieldArgs::from_attributes(&field.attrs).expect("i failed");
    match typ {
        UsedType::Primitive => {
            quote_spanned! { field.span() =>
                let #ident = #type_name::read_from(reader)?;
            }
        }
        UsedType::String => {
            let content = match args.size.unwrap_or(1) {
                1 => quote! {
                    for _ in 0..len {
                        bytes.push(u8::read_from(reader)?);
                    }
                    let #ident = String::from_utf8(bytes)?;
                },
                2 => quote! {
                    for _ in 0..len {
                        bytes.push(u16::read_from(reader)?);
                    }
                    let #ident = String::from_utf16(&bytes)?;
                },
                _ => abort!(field, "Unknown String size"),
            };

            quote_spanned! { field.span() =>
                let len = u16::read_from(reader)?;
                let mut bytes = Vec::with_capacity(len as usize);
                #content
            }
        }
        UsedType::Array(len) => {
            quote_spanned! { field.span() =>
                let mut bytes = [0u8; #len];
                reader.read_exact(&mut bytes)?;
                let #ident = bytes;
            }
        }
        UsedType::Collection(inner) => {
            let length_type = args.list_type.as_deref().unwrap_or(DEFAULT_LIST_TYPE);
            let inner_ty = get_type_of(inner);
            let inner = generate_reader_for_inner(ident, inner, &inner_ty);

            if length_type == "break" {
                quote_spanned! {field.span() =>
                    let mut items = Vec::new();
                    loop {
                        let has_entry = u8::read_from(reader)?;
                        if has_entry == 2 {
                            break;
                        }
                        if has_entry == 1 {
                            #inner
                            items.push(#ident);
                        }
                    }
                    let #ident = items;
                }
            } else if length_type == "has-more" {
                quote_spanned! {field.span() =>
                    let mut items = Vec::new();
                    loop {
                        let has_entry = u8::read_from(reader)?;
                        if has_entry == 0 {
                            break;
                        }
                        if has_entry == 1 {
                            #inner
                            items.push(#ident);
                        }
                    }
                    let #ident = items;
                }
            } else if length_type == "by-size-field" {
                let size_field = args.size_field.as_deref();
                match size_field {
                    Some(size_field_name) => {
                        if let Ok(size_field_name) = syn::parse_str::<Expr>(&size_field_name) {
                            quote_spanned! { field.span() =>
                                let size = #size_field_name;
                                let mut items = Vec::with_capacity(size as usize);
                                for _ in 0..size {
                                    #inner
                                    items.push(#ident)
                                }
                                let #ident = items;
                            }
                        } else {
                            abort!(field, "size field could not be parsed");
                        }
                    }
                    None => {
                        let size_field_name = "length";
                        quote_spanned! { field.span() =>
                            let size = #size_field_name;
                            let mut items = Vec::with_capacity(size as usize);
                            for _ in 0..size {
                                #inner
                                items.push(#ident)
                            }
                            let #ident = items;
                        }
                    }
                }
            } else {
                quote_spanned! { field.span() =>
                    let size = u8::read_from(reader)?;
                    let mut items = Vec::with_capacity(size as usize);
                    for _ in 0..size {
                        #inner
                        items.push(#ident);
                    }
                    let #ident = items;
                }
            }
            // not yet required and not yet adapted
            // } else if length_type == "length" {
            //     let size_type = match size {
            //         1 => quote!(u8),
            //         2 => quote!(u16),
            //         3 => quote!(u32),
            //         4 => quote!(u64),
            //         _ => abort!(ident, "Could not determine size for list."),
            //     };
            //     quote_spanned! {field.span() =>
            //         (#ident.len() as #size_type).serialize_to(buf);
            //         for inner in #ident.iter() {
            //             #inner_ty;
            //         }
            //     }
            // } else {
            //     quote_spanned! {field.span() =>
            //         for inner in #ident.iter() {
            //             #inner_ty;
            //         }
            //     }
            // }
        }
        UsedType::Option(inner) => {
            let inner_ty = get_type_of(inner);
            let inner_ts = generate_reader_for_inner(ident, inner, &inner_ty);
            match args.when {
                Some(condition) => {
                    if let Ok(condition) = syn::parse_str::<Expr>(&condition) {
                        quote_spanned! { field.span() =>
                            let #ident = if #condition {
                                #inner_ts
                                Some(#ident)
                            } else {
                                None
                            };
                        }
                    } else {
                        abort!(field, "Condition could not be parsed");
                    }
                }
                None => {
                    quote_spanned! { field.span() =>
                        let some = u8::read_from(reader)?;
                        let #ident = if some == 1 {
                            #inner_ts
                            Some(#ident)
                        } else {
                            None
                        };
                    }
                }
            }
        }
        _ => abort!(ident, "Tuples are not supported."), // UsedType::Tuple(inner) => {
                                                         //     let idents = (0..inner.len())
                                                         //         .map(|i| format_ident!("t{}", i))
                                                         //         .collect::<Vec<Ident>>();
                                                         //     let content = inner.iter().zip(&idents).map(|(ty, ident)| {
                                                         //         let inner_ty = get_type_of(ty);
                                                         //         generate_reader_for_inner(ident, ty, &inner_ty)
                                                         //     });
                                                         //     quote_spanned! { field.span() =>
                                                         //         #(#content)*
                                                         //         let #ident = (#(#idents),*);
                                                         //     }
                                                         // },
    }
}

fn generate_reader_for_inner(ident: &Ident, type_name: &Type, ty: &UsedType) -> TokenStream {
    match ty {
        UsedType::Primitive => {
            quote_spanned! { ident.span() =>
                let #ident = #type_name::read_from(reader)?;
            }
        }
        UsedType::String => {
            quote_spanned! { ident.span() =>
                let len = u16::read_from(reader)?;
                let mut bytes = Vec::with_capacity(len as usize);
                for _ in 0..len {
                    bytes.push(u8::read_from(reader)?);
                }
                let #ident = String::from_utf8(bytes)?;
            }
        }
        UsedType::Array(len) => {
            quote_spanned! { ident.span() =>
                let mut bytes = [0u8; #len];
                reader.read_exact(bytes)?;
                let #ident = bytes;
            }
        }
        UsedType::Collection(inner) => {
            quote_spanned! { ident.span() =>
                let size = u8::read_from(reader)?;
                let mut items = Vec::with_capacity(size as usize);
                for _ in 0..size {
                    items.push(#inner::read_from(reader)?);
                }
                let #ident = items;
            }
        }
        UsedType::Option(inner) => {
            quote_spanned! { ident.span() =>
                let some = u8::read_from(reader)?;
                let #ident = if some == 1 {
                    Some(#inner::read_from(reader)?)
                } else {
                    None
                };
            }
        }
        UsedType::Tuple(inner) => {
            let content = inner.iter().map(|ty| quote!(#ty::read_from(reader)?));
            quote_spanned! { ident.span() =>
                let #ident = (#(#content),*);
            }
        }
    }
}
