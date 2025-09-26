use iioon::I18N;

#[derive(I18N)]
#[i18n(folder = "test-locales/", fallback = "en")]
pub struct Locale;

#[test]
fn top_level() {
    assert!(!Locale.en().hello().is_empty())
}

#[test]
fn nested() {
    assert!(!Locale.en().nested().hello_nested().is_empty())
}

#[test]
fn deeper_nested() {
    assert!(
        !Locale
            .en()
            .nested()
            .deeper_nested()
            .hello_deeper_nested()
            .is_empty()
    )
}

#[test]
fn other_language() {
    assert!(!Locale.de().nested().hello_nested().is_empty())
}

#[test]
fn fallback() {
    assert!(!Locale.fallback().hello().is_empty())
}

#[test]
fn get_lang() {
    assert!(Locale.get_lang("en").is_some());
    assert!(Locale.get_lang("eN").is_some());
    assert!(Locale.get_lang("no").is_none());
    assert!(!Locale.get_lang("No").unwrap_or_default().hello().is_empty())
}

#[test]
fn args() {
    assert_eq!(Locale.en().args().hello_args("John Doe"), "Hello John Doe!")
}
