//! XFA locales: the document's `<localeSet>` packet plus PDFium's built-in
//! locale data (`xfa/fxfa/parser/cxfa_localemgr.cpp`, BSD-3-Clause; the
//! XML in `locales/` is that data decompressed, with the typeface lists
//! and descriptions dropped).
//!
//! Locale lookup follows `CXFA_LocaleMgr::GetLocaleByName`: an exact
//! `localeSet` entry, then a built-in locale chosen by language (and
//! region for zh/en/es), else en_US. The ambient locale is the config
//! packet's `acrobat|present > common > locale`, else en_US.

use std::sync::OnceLock;

use super::xml::{parse_xml, XmlNode};
use super::XfaLimits;

/// Symbols and patterns of one locale.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Locale {
    pub name: String,
    pub months: [String; 12],
    pub months_abbr: [String; 12],
    pub days: [String; 7],
    pub days_abbr: [String; 7],
    pub meridiem: [String; 2],
    pub eras: [String; 2],
    /// full, long, med, short.
    pub date_patterns: [String; 4],
    pub time_patterns: [String; 4],
    /// numeric, currency, percent.
    pub num_patterns: [String; 3],
    pub decimal: String,
    pub grouping: String,
    pub percent: String,
    pub minus: String,
    pub zero: String,
    pub currency_symbol: String,
    pub currency_iso: String,
    pub currency_decimal: String,
}

/// Index into [`Locale::date_patterns`] / [`Locale::time_patterns`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum PatternLength {
    Full = 0,
    Long = 1,
    Med = 2,
    Short = 3,
}

impl PatternLength {
    pub(crate) fn parse(name: &str) -> Option<Self> {
        Some(match name {
            "full" => Self::Full,
            "long" => Self::Long,
            "med" | "medium" => Self::Med,
            "short" => Self::Short,
            _ => return None,
        })
    }
}

const BUILT_IN: &[(&str, &str)] = &[
    ("en_US", include_str!("locales/en_US.xml")),
    ("en_GB", include_str!("locales/en_GB.xml")),
    ("zh_CN", include_str!("locales/zh_CN.xml")),
    ("zh_TW", include_str!("locales/zh_TW.xml")),
    ("zh_HK", include_str!("locales/zh_HK.xml")),
    ("ja_JP", include_str!("locales/ja_JP.xml")),
    ("ko_KR", include_str!("locales/ko_KR.xml")),
    ("es_ES", include_str!("locales/es_ES.xml")),
    ("es_MX", include_str!("locales/es_MX.xml")),
    ("de_DE", include_str!("locales/de_DE.xml")),
    ("fr_FR", include_str!("locales/fr_FR.xml")),
    ("it_IT", include_str!("locales/it_IT.xml")),
    ("pt_BR", include_str!("locales/pt_BR.xml")),
    ("nl_NL", include_str!("locales/nl_NL.xml")),
    ("ru_RU", include_str!("locales/ru_RU.xml")),
];

fn built_ins() -> &'static [Locale] {
    static LOCALES: OnceLock<Vec<Locale>> = OnceLock::new();
    LOCALES.get_or_init(|| {
        BUILT_IN
            .iter()
            .filter_map(|(_, xml)| {
                let parsed = parse_xml(xml.as_bytes(), &XfaLimits::default()).ok()?;
                Locale::from_xml(&parsed.root)
            })
            .collect()
    })
}

/// en_US, always available.
pub(crate) fn en_us() -> &'static Locale {
    &built_ins()[0]
}

/// PDFium `GetLanguageID`: the built-in locale used for a locale name.
fn built_in_for(name: &str) -> &'static Locale {
    let locales = built_ins();
    if let Some(found) = locales.iter().find(|locale| locale.name == name) {
        return found;
    }
    let lower = name.to_ascii_lowercase();
    let language = lower.get(..2).unwrap_or("");
    let region = lower.get(3..5).unwrap_or("");
    let wanted = match (language, region) {
        ("zh", "cn") => "zh_CN",
        ("zh", "tw") => "zh_TW",
        ("zh", "hk") => "zh_HK",
        ("ja", _) => "ja_JP",
        ("ko", _) => "ko_KR",
        ("en", "gb") => "en_GB",
        ("de", _) => "de_DE",
        ("fr", _) => "fr_FR",
        ("es", "es") => "es_ES",
        ("es", _) => "es_MX",
        ("it", _) => "it_IT",
        ("pt", _) => "pt_BR",
        ("nl", _) => "nl_NL",
        ("ru", _) => "ru_RU",
        _ => "en_US",
    };
    locales
        .iter()
        .find(|locale| locale.name == wanted)
        .unwrap_or(&locales[0])
}

/// Text of a locale symbol element; whitespace-only symbols (the French
/// no-break-space grouping separator) live in the mixed-content `head`.
fn symbol_text(node: &XmlNode) -> String {
    if !node.text.is_empty() {
        node.text.trim_matches(['\r', '\n', '\t']).to_string()
    } else {
        node.head.clone()
    }
}

fn names<const N: usize>(node: Option<&XmlNode>, fallback: &[String; N]) -> [String; N] {
    let mut out = fallback.clone();
    if let Some(node) = node {
        for (slot, child) in out.iter_mut().zip(node.children.iter()) {
            *slot = symbol_text(child);
        }
    }
    out
}

impl Locale {
    /// Parses a `<locale>` element (localeSet packet or built-in data);
    /// missing parts fall back to en_US.
    pub(crate) fn from_xml(node: &XmlNode) -> Option<Self> {
        if node.local_name != "locale" {
            return None;
        }
        let name = node.attr("name")?.to_string();
        let base = built_ins_partial();
        let calendar = node.child("calendarSymbols");
        let pick = |parent: &str, abbr: bool| {
            calendar.and_then(|calendar| {
                calendar.children.iter().find(|child| {
                    child.local_name == parent && (child.attr("abbr") == Some("1")) == abbr
                })
            })
        };
        let mut locale = Locale {
            name,
            months: names(pick("monthNames", false), &base.months),
            months_abbr: names(pick("monthNames", true), &base.months_abbr),
            days: names(pick("dayNames", false), &base.days),
            days_abbr: names(pick("dayNames", true), &base.days_abbr),
            meridiem: names(pick("meridiemNames", false), &base.meridiem),
            eras: names(pick("eraNames", false), &base.eras),
            date_patterns: base.date_patterns.clone(),
            time_patterns: base.time_patterns.clone(),
            num_patterns: base.num_patterns.clone(),
            decimal: base.decimal.clone(),
            grouping: base.grouping.clone(),
            percent: base.percent.clone(),
            minus: base.minus.clone(),
            zero: base.zero.clone(),
            currency_symbol: base.currency_symbol.clone(),
            currency_iso: base.currency_iso.clone(),
            currency_decimal: base.currency_decimal.clone(),
        };
        let set_patterns = |slots: &mut [String; 4], parent: Option<&XmlNode>| {
            for child in parent.into_iter().flat_map(|p| p.children.iter()) {
                if let Some(index) = child.attr("name").and_then(PatternLength::parse) {
                    slots[index as usize] = symbol_text(child);
                }
            }
        };
        set_patterns(&mut locale.date_patterns, node.child("datePatterns"));
        set_patterns(&mut locale.time_patterns, node.child("timePatterns"));
        for child in node
            .child("numberPatterns")
            .into_iter()
            .flat_map(|p| p.children.iter())
        {
            let index = match child.attr("name") {
                Some("numeric") => 0,
                Some("currency") => 1,
                Some("percent") => 2,
                _ => continue,
            };
            locale.num_patterns[index] = symbol_text(child);
        }
        for child in node
            .child("numberSymbols")
            .into_iter()
            .flat_map(|p| p.children.iter())
        {
            let value = symbol_text(child);
            match child.attr("name") {
                Some("decimal") => locale.decimal = value,
                Some("grouping") => locale.grouping = value,
                Some("percent") => locale.percent = value,
                Some("minus") => locale.minus = value,
                Some("zero") => locale.zero = value,
                _ => {}
            }
        }
        for child in node
            .child("currencySymbols")
            .into_iter()
            .flat_map(|p| p.children.iter())
        {
            let value = symbol_text(child);
            match child.attr("name") {
                Some("symbol") => locale.currency_symbol = value,
                Some("isoname") => locale.currency_iso = value,
                Some("decimal") => locale.currency_decimal = value,
                _ => {}
            }
        }
        Some(locale)
    }

    pub(crate) fn date_pattern(&self, length: PatternLength) -> &str {
        &self.date_patterns[length as usize]
    }

    pub(crate) fn time_pattern(&self, length: PatternLength) -> &str {
        &self.time_patterns[length as usize]
    }
}

/// en_US symbols used to complete partial locale definitions (and to
/// parse the built-in data itself).
fn built_ins_partial() -> &'static Locale {
    static BASE: OnceLock<Locale> = OnceLock::new();
    BASE.get_or_init(|| {
        let s = |items: &[&str]| items.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        let arr12 = |v: Vec<String>| -> [String; 12] { v.try_into().unwrap_or_default() };
        let arr7 = |v: Vec<String>| -> [String; 7] { v.try_into().unwrap_or_default() };
        Locale {
            name: "en_US".into(),
            months: arr12(s(&[
                "January",
                "February",
                "March",
                "April",
                "May",
                "June",
                "July",
                "August",
                "September",
                "October",
                "November",
                "December",
            ])),
            months_abbr: arr12(s(&[
                "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
            ])),
            days: arr7(s(&[
                "Sunday",
                "Monday",
                "Tuesday",
                "Wednesday",
                "Thursday",
                "Friday",
                "Saturday",
            ])),
            days_abbr: arr7(s(&["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"])),
            meridiem: ["AM".into(), "PM".into()],
            eras: ["BC".into(), "AD".into()],
            date_patterns: [
                "EEEE, MMMM D, YYYY".into(),
                "MMMM D, YYYY".into(),
                "MMM D, YYYY".into(),
                "M/D/YY".into(),
            ],
            time_patterns: [
                "h:MM:SS A Z".into(),
                "h:MM:SS A Z".into(),
                "h:MM:SS A".into(),
                "h:MM A".into(),
            ],
            num_patterns: [
                "z,zz9.zzz".into(),
                "$z,zz9.99|($z,zz9.99)".into(),
                "z,zz9%".into(),
            ],
            decimal: ".".into(),
            grouping: ",".into(),
            percent: "%".into(),
            minus: "-".into(),
            zero: "0".into(),
            currency_symbol: "$".into(),
            currency_iso: "USD".into(),
            currency_decimal: ".".into(),
        }
    })
}

/// The locales a document can use: its `<localeSet>` entries and the
/// built-ins, plus the ambient (default) locale name.
#[derive(Clone, Debug)]
pub(crate) struct LocaleSet {
    custom: Vec<Locale>,
    ambient: String,
}

impl Default for LocaleSet {
    fn default() -> Self {
        Self {
            custom: Vec::new(),
            ambient: "en_US".to_string(),
        }
    }
}

impl LocaleSet {
    /// `locale_set`: the `<localeSet>` packet root; `config`: the config
    /// packet root.
    pub(crate) fn new(locale_set: Option<&XmlNode>, config: Option<&XmlNode>) -> Self {
        let custom = locale_set
            .map(|set| {
                set.descendants("locale")
                    .take(256)
                    .filter_map(Locale::from_xml)
                    .collect()
            })
            .unwrap_or_default();
        let ambient = config
            .and_then(config_locale)
            .unwrap_or_else(|| "en_US".to_string());
        Self { custom, ambient }
    }

    /// The ambient locale name (config locale, else en_US).
    pub(crate) fn ambient(&self) -> &str {
        &self.ambient
    }

    /// Locale by name (`None`/`ambient`/empty: the ambient locale).
    pub(crate) fn get(&self, name: Option<&str>) -> &Locale {
        let name = match name.map(str::trim) {
            None | Some("") | Some("ambient") => self.ambient.as_str(),
            Some(name) => name,
        };
        if let Some(found) = self.custom.iter().find(|locale| locale.name == name) {
            return found;
        }
        built_in_for(name)
    }
}

/// PDFium `GetConfigLocaleName`: `config > acrobat|present > common > locale`.
fn config_locale(config: &XmlNode) -> Option<String> {
    let config = if config.local_name == "config" {
        config
    } else {
        config.descendants("config").next()?
    };
    let section = config
        .child("acrobat")
        .or_else(|| config.child("present"))?;
    let locale = section.child("common")?.child("locale")?;
    let value = locale.text.trim();
    (!value.is_empty()).then(|| value.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(xml: &str) -> XmlNode {
        parse_xml(xml.as_bytes(), &XfaLimits::default())
            .unwrap()
            .root
    }

    #[test]
    fn built_in_locales_parse() {
        assert_eq!(built_ins().len(), BUILT_IN.len());
        let set = LocaleSet::default();
        let de = set.get(Some("de_DE"));
        assert_eq!(de.months[2], "März");
        assert_eq!(de.decimal, ",");
        assert_eq!(de.grouping, ".");
        assert_eq!(de.date_pattern(PatternLength::Med), "DD.MM.YYYY");
        assert_eq!(set.get(Some("fr_CA")).name, "fr_FR");
        assert_eq!(set.get(Some("fr_FR")).grouping, "\u{a0}");
        assert_eq!(set.get(Some("ja_JP")).currency_symbol, "\u{ffe5}");
        assert_eq!(set.get(Some("es_AR")).name, "es_MX");
        assert_eq!(set.get(Some("xx_YY")).name, "en_US");
        assert_eq!(set.get(None).name, "en_US");
    }

    #[test]
    fn locale_set_packet_and_config_ambient() {
        let set = parse(
            r#"<localeSet><locale name="xx_XX"><numberSymbols><numberSymbol name="decimal">;</numberSymbol></numberSymbols></locale></localeSet>"#,
        );
        let config =
            parse(r#"<config><acrobat><common><locale>de_CH</locale></common></acrobat></config>"#);
        let locales = LocaleSet::new(Some(&set), Some(&config));
        assert_eq!(locales.ambient(), "de_CH");
        assert_eq!(locales.get(None).name, "de_DE");
        let custom = locales.get(Some("xx_XX"));
        assert_eq!(custom.decimal, ";");
        assert_eq!(custom.grouping, ",");
    }
}
