#![forbid(unsafe_code)]

use proc_macro_error::{abort_if_dirty, emit_error, proc_macro_error};
use quote::quote;
use syn::punctuated::Punctuated;
use syn::{Attribute, Data, Meta, MetaList, Token};
use synstructure::{AddBounds, Structure, decl_derive};

const IGNORE: &str = "ignore";
const UNSAFE_NO_DROP: &str = "unsafe_no_drop";
const ALLOWED_ATTR_META_ITEMS: [&str; 2] = [IGNORE, UNSAFE_NO_DROP];

decl_derive!([Trace, attributes(rust_cc)] => #[proc_macro_error] derive_trace_trait);

fn derive_trace_trait(mut s: Structure<'_>) -> proc_macro2::TokenStream {
    // Check if the struct is annotated with #[rust_cc(unsafe_no_drop)]
    let no_drop = s.ast().attrs.iter().any(|attr| attr_contains(attr, UNSAFE_NO_DROP));

    // Ignore every field and variant annotated with #[rust_cc(ignore)]
    // Filter fields before variants to be able to emit all the errors in case of wrong attributes in ignored variants
    s.filter(|bi| !bi.ast().attrs.iter().any(|attr| attr_contains(attr, IGNORE)));

    // Filter variants only in case of enums
    if let Data::Enum(_) = s.ast().data {
        s.filter_variants(|vi| !vi.ast().attrs.iter().any(|attr| attr_contains(attr, IGNORE)));
    }

    // Abort if errors has been emitted
    abort_if_dirty();

    // Identifier for the ctx parameter of Trace::trace(...)
    // Shouldn't clash with any other identifier
    let ctx = quote::format_ident!("__rust_cc__Trace__ctx__");

    // There's no .len() method, so this is the only way to know if there are any traced fields
    let mut has_no_variants = true; // See inline_attr below

    let body = s.each(|bi| {
        has_no_variants = false;

        let ty = &bi.ast().ty;
        quote! {
            <#ty as rust_cc::Trace>::trace(#bi, #ctx);
        }
    });

    // Generate an #[inline(always)] if no field is being traced
    let inline_attr = if has_no_variants {
        quote! { #[inline(always)] }
    } else {
        quote! { #[inline] }
    };

    s.underscore_const(true);

    s.add_bounds(AddBounds::Fields);
    let trace_impl = s.gen_impl(quote! {
        extern crate rust_cc;

        gen unsafe impl rust_cc::Trace for @Self {
            #inline_attr
            #[allow(non_snake_case)]
            fn trace(&self, #ctx: &mut rust_cc::Context<'_>) {
                match *self { #body }
            }
        }
    });

    if no_drop {
        return trace_impl;
    }

    s.add_bounds(AddBounds::None); // Don't generate bounds for Drop
    let drop_impl = s.gen_impl(quote! {
        extern crate core;

        gen impl core::ops::Drop for @Self {
            #[inline(always)]
            fn drop(&mut self) {
            }
        }
    });

    quote! {
        #trace_impl
        #drop_impl
    }
}

fn get_meta_items(attr: &Attribute) -> Option<&MetaList> {
    if attr.path().is_ident("rust_cc") {
        match &attr.meta {
            Meta::List(meta) => Some(meta),
            err => {
                emit_error!(err, "Invalid attribute");
                None
            },
        }
    } else {
        None
    }
}

fn attr_contains(attr: &Attribute, ident: &str) -> bool {
    let Some(meta_list) = get_meta_items(attr) else {
        return false;
    };

    let nested = match meta_list.parse_args_with(Punctuated::<Meta, Token![,]>::parse_terminated) {
        Ok(nested) => nested,
        Err(err) => {
            emit_error!(meta_list, "Invalid attribute: {}", err);
            return false;
        },
    };

    for meta in nested {
        match meta {
            Meta::Path(path) if path.is_ident(ident) => {
                return true;
            },
            Meta::Path(path) if ALLOWED_ATTR_META_ITEMS.iter().any(|id| path.is_ident(id)) => {
                emit_error!(path, "Invalid attribute position");
            },
            Meta::Path(path) => {
                emit_error!(path, "Unrecognized attribute");
            },
            err => {
                emit_error!(err, "Invalid attribute");
            },
        }
    }

    false
}

decl_derive!([Finalize] => derive_finalize_trait);

fn derive_finalize_trait(mut s: Structure<'_>) -> proc_macro2::TokenStream {
    s.underscore_const(true);
    s.add_bounds(AddBounds::None); // Don't generate bounds for Finalize
    s.gen_impl(quote! {
        extern crate rust_cc;

        gen impl rust_cc::Finalize for @Self {
        }
    })
}
