mod lang;

use std::{
    collections::BTreeMap,
    env::var,
    ffi::OsStr,
    fs::{read_dir, read_to_string},
    path::PathBuf,
    sync::LazyLock,
};

use anyhow::{Context, Error as AnyError};
use convert_case::{Case, Casing};
use darling::FromDeriveInput;
use proc_macro2::{Span, TokenStream};
use quote::quote;
use regex::{Regex, RegexBuilder};
use syn::{DeriveInput, Error as SynError, Generics, Ident};
use toml::{Value, from_str, map::Map as TomlMap};

use self::lang::Lang;

static ARGUMENT_RE: LazyLock<Regex> = LazyLock::new(|| {
    RegexBuilder::new(r#"\{(?<arg>[a-zA-z\d_]+)\}"#)
        .build()
        .unwrap()
});

#[derive(FromDeriveInput)]
#[darling(attributes(i18n), supports(struct_unit))]
struct DeriveOpts {
    ident: Ident,
    generics: Generics,
    folder: PathBuf,
    fallback: Option<String>,
}

fn generate_enum_impl(
    langs: &Vec<Lang>,
    langs_map: &BTreeMap<Lang, TomlMap<String, Value>>,
    struct_name: Option<String>,
    is_enum: bool,
    new_struct: bool,
) -> Result<TokenStream, AnyError> {
    let mut new_struct_name = if let Some(s) = &struct_name {
        Vec::from([String::from(s)])
    } else {
        Vec::new()
    };
    let struct_ident = Ident::new(
        struct_name.unwrap_or("Language".to_string()).as_ref(),
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
        let mut fn_args = quote! {};

        match val {
            Value::String(s) => {
                fn_return_ty.extend(quote! {Cow<'static, str>});
                let mut fn_match_content = quote! {};
                let mut has_args = false;

                for arg in ARGUMENT_RE.captures_iter(s) {
                    let Some(arg_name) = arg.name("arg") else {
                        continue;
                    };
                    let arg_ident = Ident::new(arg_name.as_str(), Span::call_site());

                    fn_args.extend(quote! {
                        #arg_ident: impl Display,
                    });
                    has_args = true;
                }

                for lang in langs {
                    let lang_ident = lang.enum_variant();
                    let locale_str = langs_map
                        .get(lang)
                        .context(format!("invalid language {}", lang.inner()))?
                        .get(key)
                        .context(format!("invalid string key {}", key))?
                        .as_str()
                        .context(format!("invalid string field {}", key))?;
                    let return_val = if has_args {
                        quote! {Cow::Owned(format!(#locale_str))}
                    } else {
                        quote! {Cow::Borrowed(#locale_str)}
                    };

                    fn_match_content.extend(quote! {
                        Language::#lang_ident => #return_val,
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
                new_struct_name.push(key.to_case(Case::Pascal));
                let mut new_map = BTreeMap::new();
                for lang in langs {
                    let locale_table = langs_map
                        .get(lang)
                        .context(format!("invalid language {}", lang.inner()))?
                        .get(key)
                        .context(format!("invalid table key {}", key))?
                        .as_table()
                        .context(format!("invalid table field {}", key))?;
                    new_map.insert(Lang::from(lang), locale_table.clone());
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
                        let lang_ident = lang.enum_variant();
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

        let fn_name = Ident::new(&key.to_case(Case::Snake), Span::call_site());

        current_impl.extend(quote! {
            pub fn #fn_name(self, #fn_args) -> #fn_return_ty {
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
    langs: &Vec<(PathBuf, Lang)>,
    fallback: &Option<Lang>,
) -> Result<TokenStream, AnyError> {
    let mut langs_enum_members = quote! {};
    let mut from_str_impl = quote! {};
    let mut files_content = BTreeMap::new();
    let mut first_lang = None;

    for (file, lang) in langs {
        if first_lang.is_none() {
            first_lang = Some(Lang::from(lang));
        }

        let mem_name = lang.enum_variant();
        let mem_inner = lang.inner();
        langs_enum_members.extend(quote! {
            #mem_name,
        });
        from_str_impl.extend(quote! {
            if s.eq_ignore_ascii_case(#mem_inner) {
                return Ok(Language::#mem_name);
            }
        });

        let file_content = read_to_string(file).context(format!(
            "failed to read translation file {}",
            file.to_str().unwrap_or_default()
        ))?;
        let file_content: TomlMap<String, Value> = from_str(&file_content).context(format!(
            "failed to deserialize translation file {}",
            file.to_str().unwrap_or_default()
        ))?;
        files_content.insert(Lang::from(lang), file_content);
    }

    let langs: Vec<Lang> = files_content.keys().map(Lang::from).collect();
    let langs_enum_impl = generate_enum_impl(&langs, &files_content, None, true, false)?;

    let mut fallback_impl = quote! {};
    if let Some(fb) = fallback {
        let default_mem = fb.enum_variant();
        fallback_impl.extend(quote! {
            impl Default for Language {
                fn default() -> Self {
                    Self::#default_mem
                }
            }
        });
    }

    Ok(quote! {
        use std::{borrow::Cow, fmt::Display, str::FromStr};

        #[derive(Clone, Copy)]
        pub enum Language {
            #langs_enum_members
        }

        impl FromStr for Language {
            type Err = ();

            fn from_str(s: &str) -> Result<Self, Self::Err> {
                #from_str_impl

                Err(())
            }
        }

        #langs_enum_impl

        #fallback_impl
    })
}

pub fn try_derive_i18n(input: &DeriveInput) -> Result<TokenStream, AnyError> {
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

    for entry in read_dir(&translation_folder)
        .context("failed to read translation folder")?
        .flatten()
    {
        let path = entry.path().canonicalize().context("invalid path")?;
        if path.is_file() && path.extension().is_some_and(|s| s == OsStr::new("toml")) {
            let filename = path
                .file_stem()
                .context("path should be a file")?
                .to_str()
                .and_then(|s| {
                    let s = s.to_string();
                    if s.is_empty() {
                        None
                    } else {
                        Some(Lang::new(s))
                    }
                })
                .context("path should be valid UTF-8 and not empty")?;
            translation_files.push((path, filename));
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
            .any(|(_, f)| f.inner().eq_ignore_ascii_case(l.as_ref()))
        {
            return Err(SynError::new_spanned(
                input,
                format!("fallback language {} does not exist", l),
            )
            .into());
        }

        fallback_fn.extend(quote! {
            pub fn fallback(&self) -> __generated_i18n_mod::Language {
                Default::default()
            }
        })
    }

    let mut struct_impl = quote! {};
    for (_, lang) in &translation_files {
        let fn_name = lang.fn_name();
        let enum_variant = lang.enum_variant();
        struct_impl.extend(quote! {
            pub fn #fn_name(&self) -> __generated_i18n_mod::Language {
                __generated_i18n_mod::Language::#enum_variant
            }
        })
    }

    let generated_mod = generate_mod(&translation_files, &fallback.map(Lang::from))?;
    let (impl_generics, ty_generics, where_clause) = generics.split_for_impl();

    Ok({
        quote! {
            mod __generated_i18n_mod {
                #generated_mod
            }

            impl #impl_generics #ident #ty_generics #where_clause {
                #struct_impl

                #fallback_fn

                pub fn get_lang(&self, s: impl AsRef<str>) -> Option<__generated_i18n_mod::Language> {
                    let s = s.as_ref();
                    s.parse().ok()
                }
            }
        }
    })
}
