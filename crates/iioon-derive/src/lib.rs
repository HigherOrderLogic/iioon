mod i18n;

use proc_macro::TokenStream;
use proc_macro_error2::proc_macro_error;
use syn::{DeriveInput, Error as SynError, parse_macro_input};

use crate::i18n::try_derive_i18n;

#[proc_macro_error]
#[proc_macro_derive(I18N, attributes(i18n))]
pub fn derive_i18n(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    try_derive_i18n(&input)
        .unwrap_or_else(|e| match e.downcast::<SynError>() {
            Ok(e) => e.into_compile_error(),
            Err(e) => SynError::new_spanned(
                input,
                format!("error occured while trying to expand I18N macro: {}", e),
            )
            .into_compile_error(),
        })
        .into()
}
