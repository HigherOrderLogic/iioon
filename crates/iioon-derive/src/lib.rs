use std::{
    collections::HashMap,
    env::var,
    ffi::OsStr,
    fs::{read_dir, read_to_string},
    path::PathBuf,
};

use anyhow::{Context, Error as AnyError};
use convert_case::{Case, Casing};
use darling::FromDeriveInput;
use proc_macro::TokenStream;
use proc_macro_error2::proc_macro_error;
use proc_macro2::{Span, TokenStream as TokenStream2};
use quote::quote;
use syn::{DeriveInput, Error as SynError, Generics, Ident, parse_macro_input};
use toml::{Value, from_str, map::Map as TomlMap};

#[derive(FromDeriveInput)]
#[darling(attributes(i18n), supports(struct_unit))]
struct DeriveOpts {
    ident: Ident,
    generics: Generics,
    folder: PathBuf,
    fallback: Option<String>,
}

fn check_tables(left: &TomlMap<String, Value>, right: &TomlMap<String, Value>) -> bool {
    for (key, val) in left {
        let Some(right_val) = right.get(key) else {
            return false;
        };
        if !val.same_type(right_val) {
            return false;
        }
        if let Some(table) = val.as_table()
            && let Some(right_table) = right_val.as_table()
        {
            if !check_tables(table, right_table) {
                return false;
            }
        }
    }

    true
}

fn generate_enum_impl(
    langs: &Vec<String>,
    langs_map: &HashMap<String, TomlMap<String, Value>>,
    struct_name: Option<String>,
    is_enum: bool,
    new_struct: bool,
) -> Result<TokenStream2, AnyError> {
    let mut new_struct_name = if let Some(s) = &struct_name {
        Vec::from([String::from(s).to_case(Case::Pascal)])
    } else {
        Vec::new()
    };
    let struct_ident = Ident::new(
        struct_name.unwrap_or(String::from("Language")).as_ref(),
        Span::call_site(),
    );

    let mut current_struct = quote! {};
    let mut current_impl = quote! {};
    let mut extra_impl = quote! {};

    if new_struct {
        current_struct.extend(quote! {
            #[derive(Clone, Copy)]
            pub struct #struct_ident(Language);
        });
    }

    let first_lang = langs_map
        .values()
        .next()
        .context("there should be at least 1 language")?;

    for (key, val) in first_lang {
        let mut current_fn_impl = quote! {};
        let mut fn_return_ty = quote! {};

        match val {
            Value::String(_) => {
                fn_return_ty.extend(quote! {String});
                let mut fn_match_content = quote! {};

                for lang in langs {
                    let lang_ident = Ident::new(lang, Span::call_site());
                    let locale_str = langs_map
                        .get(&lang.to_case(Case::Snake))
                        .context(format!("invalid language {}", lang))?
                        .get(key)
                        .context(format!("invalid key {}", key))?
                        .as_str()
                        .context(format!("invalid field {}", key))?;

                    fn_match_content.extend(quote! {
                        Language::#lang_ident => format!(#locale_str),
                    });
                }

                current_fn_impl.extend(if is_enum {
                    quote! {
                        match self {
                            #fn_match_content
                        }
                    }
                } else {
                    quote! {
                        match self.0 {
                            #fn_match_content
                        }
                    }
                })
            }
            Value::Table(_) => {
                new_struct_name.push(String::from(key).to_case(Case::Pascal));
                let mut new_map = HashMap::new();
                for lang in langs {
                    let locale_table = langs_map
                        .get(&lang.to_case(Case::Snake))
                        .context(format!("invalid language {}", lang))?
                        .get(key)
                        .context(format!("invalid key {}", key))?
                        .as_table()
                        .context(format!("invalid field {}", key))?;
                    new_map.insert(
                        String::from(lang).to_case(Case::Snake),
                        locale_table.clone(),
                    );
                }

                let new_struct_name_str = new_struct_name.join("__");
                let new_struct_impl = generate_enum_impl(
                    langs,
                    &new_map,
                    Some(new_struct_name_str.clone()),
                    false,
                    true,
                )?;
                let new_struct_ident = Ident::new(&new_struct_name_str, Span::call_site());

                fn_return_ty.extend(quote! {#new_struct_ident});
                extra_impl.extend(new_struct_impl);
                current_fn_impl.extend(if is_enum {
                    quote! {
                        #new_struct_ident(self)
                    }
                } else {
                    let mut langs_match_body = quote! {};

                    for lang in langs {
                        let lang_ident = Ident::new(lang, Span::call_site());
                        langs_match_body.extend(quote! {
                            &Language::#lang_ident => Language::#lang_ident,
                        });
                    }

                    quote! {
                        #new_struct_ident(self.0)
                    }
                });

                new_struct_name.pop();
            }
            _ => (),
        }

        let fn_name = Ident::new(key, Span::call_site());

        current_impl.extend(quote! {
            pub fn #fn_name(self) -> #fn_return_ty {
               #current_fn_impl
            }
        })
    }

    Ok(quote! {
        #current_struct

        impl #struct_ident {
            #current_impl
        }

        #extra_impl
    })
}

fn generate_mod(
    input: &DeriveInput,
    langs: &Vec<(PathBuf, String)>,
    fallback: &Option<String>,
) -> Result<TokenStream2, AnyError> {
    let mut langs_enum_content = quote! {};
    let mut files_content = HashMap::new();
    let mut first_lang = String::new();

    for (file, lang) in langs {
        if first_lang.is_empty() {
            first_lang = String::from(lang);
        }

        let mem_name = Ident::new(&lang.to_case(Case::Constant), Span::call_site());
        langs_enum_content.extend(quote! {
            #mem_name,
        });

        let file_content = read_to_string(file).context(format!(
            "failed to read translation file {}",
            file.to_str().unwrap_or_default()
        ))?;
        let file_content: TomlMap<String, Value> = from_str(&file_content).context(format!(
            "failed to deserialize translation file {}",
            file.to_str().unwrap_or_default()
        ))?;
        files_content.insert(String::from(lang), file_content);
    }

    let check_lang = if let Some(fb) = fallback {
        files_content
            .get(&fb.to_string())
            .context("invalid fallback lang")?
    } else {
        files_content
            .get(first_lang.as_str())
            .context("invalid language")?
    };

    for (lang, table) in &files_content {
        if !check_tables(check_lang, table) {
            return Err(SynError::new_spanned(
                input,
                format!(
                    "language {}'s translation file is not in the right format",
                    lang
                ),
            )
            .into());
        }
    }

    let langs: Vec<String> = files_content
        .keys()
        .map(|s| s.to_case(Case::Constant))
        .collect();

    let langs_enum_impl = generate_enum_impl(&langs, &files_content, None, true, false)?;

    let mut fallback_impl = quote! {};
    if let Some(fb) = fallback {
        let default_mem = Ident::new(&fb.to_string().to_case(Case::Constant), Span::call_site());
        fallback_impl.extend(quote! {
            impl Default for Language {
                fn default() -> Self {
                    Self::#default_mem
                }
            }
        });
    }

    Ok(quote! {
        #[derive(Clone, Copy)]
        pub enum Language {
            #langs_enum_content
        }

        #langs_enum_impl

        #fallback_impl
    })
}

fn try_derive_i18n(input: &DeriveInput) -> Result<TokenStream2, AnyError> {
    let DeriveOpts {
        folder,
        fallback,
        generics,
        ident,
    } = match DeriveOpts::from_derive_input(input) {
        Ok(o) => o,
        Err(e) => return Ok(e.write_errors()),
    };

    let translation_folder = if folder.is_relative() {
        PathBuf::from(var("CARGO_MANIFEST_DIR").context("CARGO_MANIFEST_DIR should exist")?)
            .join(&folder)
    } else {
        folder
    };

    if !translation_folder.exists() {
        return Err(SynError::new_spanned(
            input,
            format!(
                "folder {} does not exist",
                translation_folder.to_str().unwrap_or_default()
            ),
        )
        .into());
    }

    let mut translation_files = Vec::new();

    for entry in read_dir(&translation_folder).unwrap() {
        if let Ok(e) = entry {
            let path = e.path().canonicalize().context("invalid path")?;
            if path.is_file() && path.extension().is_some_and(|s| s == OsStr::new("toml")) {
                let filename = path
                    .file_stem()
                    .context("path should be a file")?
                    .to_str()
                    .map(String::from)
                    .and_then(|s| {
                        if s.is_empty() {
                            None
                        } else {
                            Some(s.to_case(Case::Snake))
                        }
                    })
                    .context("path should be valid UTF-8 and not empty")?;
                translation_files.push((path, filename));
            }
        }
    }

    if translation_files.is_empty() {
        return Err(SynError::new_spanned(
            input,
            format!(
                "no translation file found at {}",
                translation_folder.to_str().unwrap_or_default()
            ),
        )
        .into());
    }

    let mut fallback_fn = quote! {};

    if let Some(l) = &fallback {
        if !translation_files
            .iter()
            .any(|(_, f)| f.eq_ignore_ascii_case(l.as_ref()))
        {
            return Err(SynError::new_spanned(
                input,
                format!("fallback language {} does not exist", l),
            )
            .into());
        }

        fallback_fn.extend(quote! {
            pub fn fallback(self) -> __generated_i18n_mod::Language {
                Default::default()
            }
        })
    }

    let mut struct_impl = quote! {};
    for (_, lang) in &translation_files {
        let fn_name = Ident::new(&lang, Span::call_site());
        let enum_variant = Ident::new(
            &String::from(lang).to_case(Case::Constant),
            Span::call_site(),
        );
        struct_impl.extend(quote! {
            pub fn #fn_name(&self) -> __generated_i18n_mod::Language {
                __generated_i18n_mod::Language::#enum_variant
            }
        })
    }

    let generated_mod = generate_mod(&input, &translation_files, &fallback)?;
    let (impl_generics, ty_generics, where_clause) = generics.split_for_impl();

    Ok({
        quote! {
            mod __generated_i18n_mod {
                #generated_mod
            }

            impl #impl_generics #ident #ty_generics #where_clause {
                #struct_impl

                #fallback_fn
            }
        }
    })
}

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
