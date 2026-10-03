//! Display language is independent of musical state and stored Unicode text.
use serde::{Deserialize, Serialize};
use std::{cell::Cell, collections::BTreeMap, sync::LazyLock};
macro_rules! tr { ($key:expr) => { crate::localization::text($key) }; }
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum Locale { #[default] English, Spanish, German }
impl Locale {
    pub const ALL: [Self; 3] = [Self::English, Self::Spanish, Self::German];
    /// Name the language in its own script. Takes this locale; returns its stable chooser label.
    pub fn name(self) -> &'static str { match self { Self::English => "English", Self::Spanish => "Español", Self::German => "Deutsch" } }
}
thread_local! { static CURRENT: Cell<Locale> = const { Cell::new(Locale::English) }; }
pub(crate) struct Scope(Locale);
impl Drop for Scope { fn drop(&mut self) { CURRENT.set(self.0); } }
/// Select a display language for this UI frame and restore the previous one on drop.
/// Takes a saved locale; returns a thread-local guard, leaving workers and other apps independent.
pub(crate) fn scope(locale: Locale) -> Scope { Scope(CURRENT.replace(locale)) }
/// Read the current frame language. Takes no input; returns its display locale.
pub(crate) fn current() -> Locale { CURRENT.get() }
static ENGLISH: LazyLock<BTreeMap<String, String>> = LazyLock::new(|| serde_json::from_str(include_str!("../locales/en.json")).expect("checked English catalogue"));
static SPANISH: LazyLock<BTreeMap<String, String>> = LazyLock::new(|| serde_json::from_str(include_str!("../locales/es.json")).expect("checked Spanish catalogue"));
static GERMAN: LazyLock<BTreeMap<String, String>> = LazyLock::new(|| serde_json::from_str(include_str!("../locales/de.json")).expect("checked German catalogue"));
fn translated(key: &str) -> Option<&'static str> {
    let catalogue = match CURRENT.get() { Locale::English => &*ENGLISH, Locale::Spanish => &*SPANISH, Locale::German => &*GERMAN };
    catalogue.get(key).or_else(|| ENGLISH.get(key)).map(String::as_str)
}
/// Look up a stable gettext-style English key without changing dynamic user text.
/// Takes a static message key; returns the active translation or its English fallback.
pub(crate) fn text(key: &'static str) -> &'static str { translated(key).unwrap_or(key) }
/// Translate a known UI label held by a helper, retaining unknown dynamic content.
/// Takes a borrowed label; returns a borrowed catalogue value or the original text.
pub(crate) fn text_dynamic(key: &str) -> &str { translated(key).unwrap_or(key) }
fn render(template: &str, values: &[String]) -> Option<String> {
    let mut out = String::new(); let mut chars = template.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '{' if chars.peek() == Some(&'{') => { chars.next(); out.push('{'); }
            '}' if chars.peek() == Some(&'}') => { chars.next(); out.push('}'); }
            '{' => {
                let mut field = String::new();
                loop { let c = chars.next()?; if c == '}' { break; } field.push(c); }
                out.push_str(values.get(field.parse::<usize>().ok()?)?);
            }
            '}' => return None,
            _ => out.push(c),
        }
    }
    Some(out)
}
/// Insert already formatted values into an external message template.
/// Takes a stable key and values evaluated once; returns localized prose with unchanged values.
pub(crate) fn format(key: &'static str, values: &[String]) -> String {
    let template = translated(key).unwrap_or(key);
    render(template, values).or_else(|| ENGLISH.get(key).and_then(|t| render(t,values))).unwrap_or_else(|| key.into())
}
/// Display a decimal number with language-specific separators.
/// Takes the value and fixed precision; returns display text without changing the value.
pub(crate) fn number(value: f64, precision: usize) -> String {
    let raw = format!("{value:.precision$}");
    if CURRENT.get() == Locale::English || !value.is_finite() { return raw; }
    raw.replace('.', ",")
}
/// Show an editable number without rounding its original precision.
/// Takes a musical control value; returns its shortest roundtrip decimal in the frame language.
pub(crate) fn number_input(value: f32) -> String {
    let raw = value.to_string();
    if CURRENT.get() == Locale::English { raw } else { raw.replace('.', ",") }
}
/// Parse a display decimal using comma or point, preserving finite musical values.
/// Takes entered text; returns a finite value or refusal without applying it.
pub(crate) fn parse_number(value: &str) -> Option<f64> {
    let parsed: f64 = if CURRENT.get() == Locale::English { value.trim().parse().ok()? }
        else { value.trim().replace(',', ".").parse().ok()? };
    parsed.is_finite().then_some(parsed)
}
/// Display an absolute UTC date in the selected language's date order.
/// Takes Unix nanoseconds; returns a localized calendar label or a checked fallback.
pub(crate) fn timestamp(ns: u64) -> String {
    let Ok(seconds) = libc::time_t::try_from(ns / 1_000_000_000) else { return text("time unavailable").into(); };
    let mut calendar = std::mem::MaybeUninit::<libc::tm>::uninit();
    if unsafe { libc::gmtime_r(&seconds, calendar.as_mut_ptr()) }.is_null() { return text("time unavailable").into(); }
    let c = unsafe { calendar.assume_init() };
    let date = calendar_date(i64::from(c.tm_year)+1900,c.tm_mon+1,c.tm_mday);
    format!("{date} {:02}:{:02}:{:02} UTC",c.tm_hour,c.tm_min,c.tm_sec)
}
/// Display a UTC calendar date without changing its absolute year or day.
/// Takes year/month/day fields; returns the selected date order, including extended years.
pub(crate) fn calendar_date(year: i64, month: i32, day: i32) -> String {
    let year = if (0..=9999).contains(&year) { format!("{year:04}") } else { format!("{year:+07}") };
    match CURRENT.get() {
        Locale::English => format!("{year}-{month:02}-{day:02}"),
        Locale::Spanish => format!("{day:02}/{month:02}/{year}"),
        Locale::German => format!("{day:02}.{month:02}.{year}"),
    }
}
/// Build a locale-independent Unicode search key without rewriting stored content.
/// Takes original text; returns NFC after full Unicode case folding for canonical matches.
pub(crate) fn search_key(value: &str) -> String {
    let folded = icu_casemap::CaseMapper::new().fold_string(value);
    icu_normalizer::ComposingNormalizer::new_nfc().normalize(&folded).into_owned()
}
#[cfg(test)] mod tests;
