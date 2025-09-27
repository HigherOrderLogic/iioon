use convert_case::{Case, Casing};
use proc_macro2::Span;
use syn::Ident;

#[derive(Clone, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub struct Lang(String);

impl From<String> for Lang {
    fn from(s: String) -> Self {
        Self::new(s)
    }
}

impl From<&Lang> for Lang {
    fn from(l: &Lang) -> Self {
        Self::new(l.inner())
    }
}

impl Lang {
    pub fn new(s: String) -> Self {
        Self(s)
    }

    pub fn inner(&self) -> String {
        String::from(&self.0)
    }

    pub fn fn_name(&self) -> Ident {
        Ident::new(&self.inner().to_case(Case::Snake), Span::call_site())
    }

    pub fn enum_variant(&self) -> Ident {
        Ident::new(&self.inner().to_case(Case::Pascal), Span::call_site())
    }
}
