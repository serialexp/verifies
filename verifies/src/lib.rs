//! `#[verifies(...)]`: declare the requirement keys a Rust test verifies.
//!
//! ```no_run
//! use verifies::verifies;
//!
//! #[verifies("REQ-142.1", "REQ-142.2")]
//! #[test]
//! fn metadata_is_fetched_once_per_ttl() {}
//! ```
//!
//! The attribute changes nothing about how the test runs. It leaves a record
//! of the test's path and keys in the test binary (see `FORMAT.md`), and
//! `cargo verifies junit` copies the keys into nextest's JUnit report as each
//! test case's `requirements` property. Because the record is in the binary,
//! a skipped or filtered-out test still reports its keys.
//!
//! It composes with other test attributes in either order (`#[tokio::test]`,
//! `rstest`, `test-case`). For parameterized tests the record names the
//! function, and `cargo verifies` applies it to every case generated from it.
//!
//! Keys are checked at compile time. Each must be `REQ-<n>` or `REQ-<n>.<m>`,
//! with numbers from 1 and no leading zeros:
//!
//! ```compile_fail
//! #[verifies::verifies("REQ-01")]
//! #[test]
//! fn leading_zero() {}
//! ```
//!
//! ```compile_fail
//! #[verifies::verifies()]
//! #[test]
//! fn no_keys() {}
//! ```
//!
//! ```compile_fail
//! #[verifies::verifies("REQ-1")]
//! struct NotATest;
//! ```

use proc_macro::TokenStream;
use proc_macro2::Span;
use quote::quote;
use syn::ext::IdentExt;
use syn::parse::Parser;
use syn::punctuated::Punctuated;
use syn::{ItemFn, LitStr, Token};

/// The record format version this macro writes (`FORMAT.md`).
const RECORD_VERSION: u8 = 1;

/// Declare the requirement keys a test verifies. See the crate docs.
#[proc_macro_attribute]
pub fn verifies(attr: TokenStream, item: TokenStream) -> TokenStream {
    match expand(attr.into(), item.into()) {
        Ok(tokens) => tokens.into(),
        Err(error) => error.to_compile_error().into(),
    }
}

fn expand(
    attr: proc_macro2::TokenStream,
    item: proc_macro2::TokenStream,
) -> syn::Result<proc_macro2::TokenStream> {
    let keys = Punctuated::<LitStr, Token![,]>::parse_terminated.parse2(attr)?;
    if keys.is_empty() {
        return Err(syn::Error::new(
            Span::call_site(),
            "#[verifies] needs at least one requirement key, e.g. #[verifies(\"REQ-1.1\")]",
        ));
    }
    let mut seen: Vec<String> = Vec::with_capacity(keys.len());
    for key in &keys {
        let value = key.value();
        if !is_key(&value) {
            return Err(syn::Error::new(
                key.span(),
                format!(
                    "`{value}` is not a requirement key; expected REQ-<n> or REQ-<n>.<m> \
                     with numbers from 1 and no leading zeros"
                ),
            ));
        }
        if seen.contains(&value) {
            return Err(syn::Error::new(
                key.span(),
                format!("`{value}` is listed twice"),
            ));
        }
        seen.push(value);
    }

    let mut function: ItemFn = syn::parse2(item)
        .map_err(|error| syn::Error::new(error.span(), "#[verifies] applies to test functions"))?;
    let name = function.sig.ident.unraw().to_string();
    let keys = seen.join(",");

    // The record is built in a const context from `module_path!()`, which
    // only the compiler knows, so its length is computed there too. It lives
    // in its own block so a test can carry several `#[verifies]` attributes.
    let record = quote! {
        {
            const __VERIFIES_PAYLOAD: &str = ::core::concat!(
                ::core::module_path!(), "::", #name, "\0", #keys
            );
            #[used]
            #[cfg_attr(target_vendor = "apple", unsafe(link_section = "__DATA,__verifies"))]
            #[cfg_attr(not(target_vendor = "apple"), unsafe(link_section = "verifies"))]
            static __VERIFIES_RECORD: [u8; 9 + __VERIFIES_PAYLOAD.len()] = {
                let payload = __VERIFIES_PAYLOAD.as_bytes();
                let mut record = [0u8; 9 + __VERIFIES_PAYLOAD.len()];
                record[0] = b'V';
                record[1] = b'R';
                record[2] = b'F';
                record[3] = b'Y';
                record[4] = #RECORD_VERSION;
                let length = (payload.len() as u32).to_le_bytes();
                record[5] = length[0];
                record[6] = length[1];
                record[7] = length[2];
                record[8] = length[3];
                let mut i = 0;
                while i < payload.len() {
                    record[9 + i] = payload[i];
                    i += 1;
                }
                record
            };
            // Keeps the record exactly as long as the test: PE linkers drop an
            // unreferenced `#[used]` section (FORMAT.md).
            ::core::hint::black_box(&__VERIFIES_RECORD);
        }
    };
    let body = &function.block;
    function.block = syn::parse_quote!({
        #record
        #body
    });
    Ok(quote!(#function))
}

/// `REQ-<n>` or `REQ-<n>.<m>`, each number a positive `i32` written without
/// sign or leading zeros: exactly one spelling per key.
fn is_key(key: &str) -> bool {
    let Some(rest) = key.strip_prefix("REQ-") else {
        return false;
    };
    let mut numbers = rest.splitn(2, '.');
    numbers.all(is_number)
}

fn is_number(digits: &str) -> bool {
    !digits.is_empty()
        && !digits.starts_with('0')
        && digits.bytes().all(|b| b.is_ascii_digit())
        && digits.parse::<i32>().is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_have_exactly_one_spelling() {
        for good in ["REQ-1", "REQ-142.2", "REQ-2147483647.2147483647"] {
            assert!(is_key(good), "{good}");
        }
        for bad in [
            "",
            "REQ-",
            "REQ-0",
            "REQ-01",
            "REQ--1",
            "REQ-+1",
            "REQ-1.",
            "REQ-1.0",
            "REQ-1.01",
            "REQ-1.2.3",
            "REQ-1 .2",
            " REQ-1",
            "REQ-a",
            "req-1",
            "REQ-2147483648",
            "REQ-1,REQ-2",
        ] {
            assert!(!is_key(bad), "{bad:?}");
        }
    }

    fn expand_str(attr: &str, item: &str) -> Result<String, String> {
        expand(attr.parse().unwrap(), item.parse().unwrap())
            .map(|tokens| tokens.to_string())
            .map_err(|error| error.to_string())
    }

    #[test]
    fn the_record_goes_first_in_the_body() {
        let out = expand_str(r#""REQ-1.1", "REQ-2""#, "#[test] fn r#match() { work(); }").unwrap();
        assert!(out.contains(r#""match" , "\0" , "REQ-1.1,REQ-2""#), "{out}");
        let record = out.find("__VERIFIES_RECORD").unwrap();
        assert!(record < out.find("work").unwrap(), "{out}");
        assert!(out.starts_with("# [test] fn r#match"), "{out}");
    }

    #[test]
    fn bad_input_is_a_compile_error() {
        assert!(
            expand_str("", "fn t() {}")
                .unwrap_err()
                .contains("at least one")
        );
        assert!(
            expand_str(r#""REQ-1.0""#, "fn t() {}")
                .unwrap_err()
                .contains("not a requirement key")
        );
        assert!(
            expand_str(r#""REQ-1", "REQ-1""#, "fn t() {}")
                .unwrap_err()
                .contains("twice")
        );
        assert!(expand_str("REQ", "fn t() {}").is_err());
        assert!(
            expand_str(r#""REQ-1""#, "struct S;")
                .unwrap_err()
                .contains("test functions")
        );
    }
}
