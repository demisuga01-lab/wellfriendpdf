//! XFA picture clauses: `num{...}`, `date{...}`, `time{...}`,
//! `datetime{...}`, `text{...}` and bare patterns, the locale pattern
//! shorthands (`date.long{}`, `num.currency{}`, `time.short{}`...),
//! explicit locales (`date(fr_FR){...}`), PDFium's default display formats
//! for dates and numbers without a picture, and the inverse parsing used by
//! FormCalc (`Date2Num`, `Parse`, ...). Symbols come from the locale
//! (month/day/meridiem names, decimal/grouping/percent/currency signs).

use super::locale::{en_us, Locale, LocaleSet, PatternLength};

// Placeholders for locale-dependent number symbols while formatting.
const GROUP: char = '\u{E000}';
const DECIMAL: char = '\u{E001}';
const CURRENCY: char = '\u{E002}';
const PERCENT: char = '\u{E003}';
const MINUS: char = '\u{E004}';

/// Formats `value` with `picture` in en_US (alternatives separated by `|`;
/// the first one that applies wins). `kind` is the value type: `date`,
/// `time`, `dateTime`, `decimal`, `float`, `integer` or `text`.
#[cfg(test)]
pub(crate) fn format_value(value: &str, picture: Option<&str>, kind: &str) -> String {
    format_value_in(value, picture, kind, &LocaleSet::default(), None)
}

/// [`format_value`] in the locale `locale` (or the set's ambient locale);
/// a picture may name its own locale (`date(de_DE){...}`).
pub(crate) fn format_value_in(
    value: &str,
    picture: Option<&str>,
    kind: &str,
    locales: &LocaleSet,
    locale: Option<&str>,
) -> String {
    let base = locales.get(locale);
    let Some(picture) = picture.map(str::trim).filter(|p| !p.is_empty()) else {
        return default_format(value, kind, base);
    };
    for alternative in split_alternatives(picture) {
        let clause = categorize(alternative, kind);
        let locale = clause
            .locale
            .map(|name| locales.get(Some(name)))
            .unwrap_or(base);
        let pattern = clause.pattern(locale);
        // Locale patterns may carry a negative alternative
        // (`$z,zz9.99|($z,zz9.99)`).
        let pattern = match pattern.split_once('|') {
            Some((positive, negative)) if clause.category == Category::Num => {
                let negative_value = value.trim().parse::<f64>().is_ok_and(|v| v < 0.0);
                if negative_value {
                    negative.to_string()
                } else {
                    positive.to_string()
                }
            }
            _ => pattern,
        };
        let formatted = match clause.category {
            Category::Num => format_number(value, &pattern, locale),
            Category::Date => parse_iso_date(value).map(|date| format_date(date, &pattern, locale)),
            Category::Time => parse_iso_time(value).map(|time| format_time(time, &pattern, locale)),
            Category::DateTime => format_date_time(value, &pattern, locale),
            Category::Text => format_text(value, &pattern),
            Category::Null => value.trim().is_empty().then(|| strip_quotes(&pattern)),
            Category::Zero => value
                .trim()
                .parse::<f64>()
                .ok()
                .filter(|v| *v == 0.0)
                .map(|_| strip_quotes(&pattern)),
        };
        if let Some(formatted) = formatted {
            return formatted;
        }
    }
    value.to_string()
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Category {
    Num,
    Date,
    Time,
    DateTime,
    Text,
    Null,
    Zero,
}

struct Clause<'a> {
    category: Category,
    /// `.full`/`.long`/`.med`/`.short`, `.integer`/`.decimal`/`.currency`/`.percent`.
    qualifier: Option<&'a str>,
    locale: Option<&'a str>,
    body: &'a str,
}

impl Clause<'_> {
    /// The pattern text: the body, or the locale's pattern for an empty
    /// body with a qualifier.
    fn pattern(&self, locale: &Locale) -> String {
        if !self.body.trim().is_empty() {
            return self.body.to_string();
        }
        let length = self.qualifier.and_then(PatternLength::parse);
        match self.category {
            Category::Date => locale
                .date_pattern(length.unwrap_or(PatternLength::Med))
                .to_string(),
            Category::Time => locale
                .time_pattern(length.unwrap_or(PatternLength::Med))
                .to_string(),
            Category::DateTime => {
                let length = length.unwrap_or(PatternLength::Med);
                format!(
                    "{}T{}",
                    locale.date_pattern(length),
                    locale.time_pattern(length)
                )
            }
            Category::Num => match self.qualifier {
                Some("integer") => "z,zzz,zzz,zzz,zzz,zzz".to_string(),
                Some("currency") => locale.num_patterns[1].clone(),
                Some("percent") => locale.num_patterns[2].clone(),
                _ => locale.num_patterns[0].clone(),
            },
            _ => String::new(),
        }
    }
}

fn strip_quotes(pattern: &str) -> String {
    let mut out = String::new();
    let mut chars = pattern.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch == '\'' {
            if chars.peek() == Some(&'\'') {
                out.push('\'');
                chars.next();
            }
            continue;
        }
        out.push(ch);
    }
    out
}

fn split_alternatives(picture: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let mut depth = 0i32;
    let mut quoted = false;
    let mut start = 0;
    for (index, ch) in picture.char_indices() {
        match ch {
            '\'' => quoted = !quoted,
            '{' if !quoted => depth += 1,
            '}' if !quoted => depth -= 1,
            '|' if !quoted && depth == 0 => {
                out.push(&picture[start..index]);
                start = index + 1;
            }
            _ => {}
        }
    }
    out.push(&picture[start..]);
    out
}

fn categorize<'a>(picture: &'a str, kind: &str) -> Clause<'a> {
    let trimmed = picture.trim();
    for (prefix, category) in [
        ("datetime", Category::DateTime),
        ("date", Category::Date),
        ("time", Category::Time),
        ("num", Category::Num),
        ("text", Category::Text),
        ("null", Category::Null),
        ("zero", Category::Zero),
    ] {
        let Some(rest) = trimmed.strip_prefix(prefix) else {
            continue;
        };
        let rest = rest.trim_start();
        let (Some(open), true) = (rest.find('{'), rest.ends_with('}')) else {
            continue;
        };
        let head = &rest[..open];
        if !(head.is_empty() || head.starts_with('.') || head.starts_with('(')) {
            continue;
        }
        let (qualifier_part, locale) = match head.find('(') {
            Some(paren) => {
                let close = head[paren..].find(')').map_or(head.len(), |c| paren + c);
                (&head[..paren], Some(head[paren + 1..close].trim()))
            }
            None => (head, None),
        };
        let qualifier = qualifier_part
            .strip_prefix('.')
            .map(str::trim)
            .filter(|q| !q.is_empty());
        return Clause {
            category,
            qualifier,
            locale: locale.filter(|l| !l.is_empty()),
            body: &rest[open + 1..rest.len() - 1],
        };
    }
    let category = match kind {
        "date" => Category::Date,
        "time" => Category::Time,
        "dateTime" => Category::DateTime,
        "decimal" | "float" | "integer" => Category::Num,
        _ => Category::Text,
    };
    Clause {
        category,
        qualifier: None,
        locale: None,
        body: trimmed,
    }
}

fn default_format(value: &str, kind: &str, locale: &Locale) -> String {
    match kind {
        // The locale's medium date (en_US "MMM D, YYYY").
        "date" => parse_iso_date(value)
            .map(|date| format_date(date, locale.date_pattern(PatternLength::Med), locale))
            .unwrap_or_else(|| value.to_string()),
        "decimal" | "float" | "integer" => {
            group_plain_number(value, locale).unwrap_or_else(|| value.to_string())
        }
        _ => value.to_string(),
    }
}

/// "1234567.50" -> "1,234,567.5" (the locale's `z,zzz...zz9.zzz` pattern).
fn group_plain_number(value: &str, locale: &Locale) -> Option<String> {
    let value = value.trim();
    if value.is_empty() {
        return None;
    }
    let number = value.parse::<f64>().ok().filter(|v| v.is_finite())?;
    let negative = number < 0.0;
    let text = value.trim_start_matches(['-', '+']);
    let (int_part, frac_part) = text.split_once('.').unwrap_or((text, ""));
    if !int_part.chars().all(|ch| ch.is_ascii_digit())
        || !frac_part.chars().all(|ch| ch.is_ascii_digit())
    {
        return None;
    }
    let int_part = int_part.trim_start_matches('0');
    let int_part = if int_part.is_empty() { "0" } else { int_part };
    let mut out = String::new();
    if negative {
        out.push_str(&locale.minus);
    }
    out.push_str(&group(int_part, &locale.grouping));
    let frac = frac_part.trim_end_matches('0');
    if !frac.is_empty() {
        out.push_str(&locale.decimal);
        out.push_str(frac);
    }
    Some(out)
}

fn group(digits: &str, separator: &str) -> String {
    let mut out = String::new();
    for (index, ch) in digits.chars().enumerate() {
        if index > 0 && (digits.len() - index).is_multiple_of(3) {
            out.push_str(separator);
        }
        out.push(ch);
    }
    out
}

/// Tokens of a picture: symbols (letters repeated) and quoted/other literals.
fn tokens(pattern: &str) -> Vec<(char, usize, String)> {
    let mut out: Vec<(char, usize, String)> = Vec::new();
    let mut chars = pattern.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch == '\'' {
            let mut literal = String::new();
            while let Some(next) = chars.next() {
                if next == '\'' {
                    if chars.peek() == Some(&'\'') {
                        literal.push('\'');
                        chars.next();
                        continue;
                    }
                    break;
                }
                literal.push(next);
            }
            out.push(('\'', 0, literal));
            continue;
        }
        if ch.is_ascii_alphabetic() {
            match out.last_mut() {
                Some((symbol, count, _)) if *symbol == ch => *count += 1,
                _ => out.push((ch, 1, String::new())),
            }
            continue;
        }
        out.push(('\'', 0, ch.to_string()));
    }
    out
}

pub(crate) fn parse_iso_date(value: &str) -> Option<(i32, u32, u32)> {
    let value = value.trim();
    let value = value.split(['T', ' ']).next()?;
    let (year, month, day) = if value.contains('-') {
        let mut parts = value.split('-');
        (parts.next()?, parts.next()?, parts.next().unwrap_or("1"))
    } else if value.len() == 8 && value.bytes().all(|b| b.is_ascii_digit()) {
        (&value[..4], &value[4..6], &value[6..8])
    } else {
        return None;
    };
    let year = year.parse::<i32>().ok()?;
    let month = month.parse::<u32>().ok().filter(|m| (1..=12).contains(m))?;
    let day = day.parse::<u32>().ok().filter(|d| (1..=31).contains(d))?;
    Some((year, month, day))
}

pub(crate) fn parse_iso_time(value: &str) -> Option<(u32, u32, u32)> {
    let value = value.trim();
    let value = value.rsplit('T').next()?;
    let value = value.trim_end_matches('Z');
    let value = value.split(['+']).next().unwrap_or(value);
    let mut parts = value.split(':');
    let hour = parts
        .next()?
        .trim()
        .parse::<u32>()
        .ok()
        .filter(|h| *h < 24)?;
    let minute = parts
        .next()
        .map_or(Some(0), |m| m.trim().parse::<u32>().ok())?;
    let second = parts
        .next()
        .map_or(Some(0), |s| s.get(..2).unwrap_or(s).parse::<u32>().ok())?;
    Some((hour, minute.min(59), second.min(59)))
}

/// 0 = Sunday.
pub(crate) fn weekday(year: i32, month: u32, day: u32) -> usize {
    // Sakamoto's algorithm.
    let offsets = [0, 3, 2, 5, 0, 3, 5, 1, 4, 6, 2, 4];
    let y = if month < 3 { year - 1 } else { year };
    let value = y + y / 4 - y / 100 + y / 400 + offsets[(month - 1) as usize] + day as i32;
    value.rem_euclid(7) as usize
}

pub(crate) fn is_leap(year: i32) -> bool {
    (year % 4 == 0 && year % 100 != 0) || year % 400 == 0
}

pub(crate) fn days_in_month(year: i32, month: u32) -> u32 {
    match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        _ if is_leap(year) => 29,
        _ => 28,
    }
}

fn day_of_year(year: i32, month: u32, day: u32) -> u32 {
    (1..month).map(|m| days_in_month(year, m)).sum::<u32>() + day
}

fn format_date((year, month, day): (i32, u32, u32), pattern: &str, locale: &Locale) -> String {
    let mut out = String::new();
    let month_index = (month.clamp(1, 12) - 1) as usize;
    let wd = weekday(year, month, day);
    for (symbol, count, literal) in tokens(pattern) {
        match (symbol, count) {
            ('\'', _) => out.push_str(&literal),
            ('Y', 2) => out.push_str(&format!("{:02}", year.rem_euclid(100))),
            ('Y', _) => out.push_str(&format!("{year:04}")),
            ('M', 1) => out.push_str(&month.to_string()),
            ('M', 2) => out.push_str(&format!("{month:02}")),
            ('M', 3) => out.push_str(&locale.months_abbr[month_index]),
            ('M', _) => out.push_str(&locale.months[month_index]),
            ('D', 1) => out.push_str(&day.to_string()),
            ('D', _) => out.push_str(&format!("{day:02}")),
            ('J', 1) => out.push_str(&day_of_year(year, month, day).to_string()),
            ('J', _) => out.push_str(&format!("{:03}", day_of_year(year, month, day))),
            ('E', 1) => out.push_str(&(wd + 1).to_string()),
            ('E', 3) => out.push_str(&locale.days_abbr[wd]),
            ('E', _) => out.push_str(&locale.days[wd]),
            // ISO day of week: Monday = 1.
            ('e', _) => out.push_str(&(if wd == 0 { 7 } else { wd }).to_string()),
            ('G', _) => out.push_str(&locale.eras[usize::from(year > 0)]),
            ('w', _) => out.push_str(&day.div_ceil(7).to_string()),
            ('W', _) => out.push_str(&format!("{:02}", iso_week(year, month, day))),
            (other, count) => {
                for _ in 0..count {
                    out.push(other);
                }
            }
        }
    }
    out
}

fn iso_week(year: i32, month: u32, day: u32) -> u32 {
    let ordinal = day_of_year(year, month, day) as i32;
    let wd = weekday(year, month, day) as i32;
    let iso_wd = if wd == 0 { 7 } else { wd };
    let week = (ordinal - iso_wd + 10) / 7;
    week.clamp(1, 53) as u32
}

fn format_time((hour, minute, second): (u32, u32, u32), pattern: &str, locale: &Locale) -> String {
    let mut out = String::new();
    let twelve = |hour: u32| {
        if hour.is_multiple_of(12) {
            12
        } else {
            hour % 12
        }
    };
    for (symbol, count, literal) in tokens(pattern) {
        match (symbol, count) {
            ('\'', _) => out.push_str(&literal),
            ('H', 1) => out.push_str(&hour.to_string()),
            ('H', _) => out.push_str(&format!("{hour:02}")),
            ('h', 1) => out.push_str(&twelve(hour).to_string()),
            ('h', _) => out.push_str(&format!("{:02}", twelve(hour))),
            ('K', 1) => out.push_str(&(hour % 12).to_string()),
            ('K', _) => out.push_str(&format!("{:02}", hour % 12)),
            ('k', 1) => out.push_str(&(if hour == 0 { 24 } else { hour }).to_string()),
            ('k', _) => out.push_str(&format!("{:02}", if hour == 0 { 24 } else { hour })),
            ('M', 1) => out.push_str(&minute.to_string()),
            ('M', _) => out.push_str(&format!("{minute:02}")),
            ('S', 1) => out.push_str(&second.to_string()),
            ('S', _) => out.push_str(&format!("{second:02}")),
            ('F', _) => out.push_str("000"),
            ('A', _) => out.push_str(&locale.meridiem[usize::from(hour >= 12)]),
            ('Z' | 'z', _) => {}
            (other, count) => {
                for _ in 0..count {
                    out.push(other);
                }
            }
        }
    }
    out.trim_end().to_string()
}

fn format_date_time(value: &str, pattern: &str, locale: &Locale) -> Option<String> {
    let (date_pattern, time_pattern) = pattern.split_once('T').unwrap_or((pattern, ""));
    let date = parse_iso_date(value)?;
    let mut out = format_date(date, date_pattern, locale);
    if !time_pattern.is_empty() {
        out.push(' ');
        out.push_str(&format_time(
            parse_iso_time(value).unwrap_or((0, 0, 0)),
            time_pattern,
            locale,
        ));
    }
    Some(out)
}

fn format_text(value: &str, pattern: &str) -> Option<String> {
    let mut chars = value.chars();
    let mut out = String::new();
    let mut pattern_chars = pattern.chars().peekable();
    while let Some(symbol) = pattern_chars.next() {
        if symbol == '\'' {
            for next in pattern_chars.by_ref() {
                if next == '\'' {
                    break;
                }
                out.push(next);
            }
            continue;
        }
        let ok: fn(char) -> bool = match symbol {
            '9' => |ch| ch.is_ascii_digit(),
            'A' => |ch| ch.is_alphabetic(),
            'O' | '0' => |ch| ch.is_alphanumeric(),
            'X' => |_| true,
            literal => {
                out.push(literal);
                continue;
            }
        };
        let ch = chars.next()?;
        if !ok(ch) {
            return None;
        }
        out.push(ch);
    }
    chars.next().is_none().then_some(out)
}

fn localize(text: &str, locale: &Locale, currency: bool) -> String {
    let mut out = String::with_capacity(text.len());
    for ch in text.chars() {
        match ch {
            GROUP => out.push_str(&locale.grouping),
            DECIMAL => out.push_str(if currency {
                &locale.currency_decimal
            } else {
                &locale.decimal
            }),
            CURRENCY => out.push_str(&locale.currency_symbol),
            PERCENT => out.push_str(&locale.percent),
            MINUS => out.push_str(&locale.minus),
            other => out.push(other),
        }
    }
    out
}

/// Numeric picture formatting (`z`, `Z`, `9`, `8`, `,`, `.`/`v`, `$`, `%`,
/// `s`/`S`, `(`/`)`, `CR`/`DB`, `E` and literals) with locale symbols.
fn format_number(value: &str, pattern: &str, locale: &Locale) -> Option<String> {
    let number = value.trim().parse::<f64>().ok().filter(|v| v.is_finite())?;
    let currency = pattern.contains('$');
    let raw = format_number_raw(number, pattern)?;
    Some(localize(&raw, locale, currency))
}

fn format_number_raw(number: f64, pattern: &str) -> Option<String> {
    // A trailing credit/debit sign (`CR`, `DB`) after an integral pattern.
    let unquoted_tail = strip_quoted_sections(pattern);
    if find_unquoted(pattern, &['.', 'v', 'V']).is_none() {
        for suffix in ["CR", "cr", "DB", "db"] {
            if unquoted_tail.trim_end().ends_with(suffix) {
                if let Some(at) = pattern.rfind(suffix) {
                    let body = format_number_raw(number.abs(), &pattern[..at])?;
                    let sign = if number < 0.0 { suffix } else { "  " };
                    return Some(format!("{body}{sign}{}", &pattern[at + 2..]));
                }
            }
        }
    }
    let percent = pattern.contains('%');
    let number = if percent { number * 100.0 } else { number };
    let negative = number < 0.0;
    // Scientific notation: `E` in the pattern.
    let unquoted: String = strip_quoted_sections(pattern);
    let (mantissa, exponent) = if unquoted.contains('E') {
        if number == 0.0 {
            (0.0, 0i32)
        } else {
            let exp = number.abs().log10().floor() as i32;
            (number.abs() / 10f64.powi(exp), exp)
        }
    } else {
        (number.abs(), 0)
    };
    let decimal_index = find_unquoted(pattern, &['.', 'v', 'V']);
    let (int_pattern, frac_pattern) = match decimal_index {
        Some(index) => (&pattern[..index], &pattern[index + 1..]),
        None => (pattern, ""),
    };
    let frac_digits = count_unquoted(frac_pattern, &['9', 'z', 'Z', '8']);
    let scaled = format!("{:.*}", frac_digits, mantissa);
    let (int_value, frac_value) = scaled.split_once('.').unwrap_or((&scaled, ""));
    let int_value = int_value.trim_start_matches('0');
    // Integral part, filled right to left.
    let mut digits: Vec<char> = int_value.chars().collect();
    let mut int_out: Vec<char> = Vec::new();
    let pattern_chars: Vec<char> = int_pattern.chars().collect();
    let digit_slots = count_unquoted(int_pattern, &['9', 'z', 'Z', '8']);
    if digits.len() > digit_slots {
        // Overflow: PDFium leaves the value unformatted.
        return None;
    }
    let mut quoted = false;
    let mut position = pattern_chars.len();
    while position > 0 {
        position -= 1;
        let ch = pattern_chars[position];
        if ch == '\'' {
            quoted = !quoted;
            continue;
        }
        if quoted {
            int_out.push(ch);
            continue;
        }
        match ch {
            '9' | '8' => int_out.push(digits.pop().unwrap_or('0')),
            'z' => {
                if let Some(digit) = digits.pop() {
                    int_out.push(digit);
                }
            }
            'Z' => int_out.push(digits.pop().unwrap_or(' ')),
            ',' => {
                let forced_left = pattern_chars[..position]
                    .iter()
                    .any(|c| matches!(c, '9' | '8'));
                if !digits.is_empty() || forced_left {
                    int_out.push(GROUP);
                }
            }
            '$' => int_out.push(CURRENCY),
            's' => int_out.push(if negative { MINUS } else { ' ' }),
            'S' => int_out.push(if negative { MINUS } else { '+' }),
            '(' => int_out.push(if negative { '(' } else { ' ' }),
            ')' => int_out.push(if negative { ')' } else { ' ' }),
            '%' => int_out.push(PERCENT),
            other if other.is_ascii_alphabetic() => {}
            other => int_out.push(other),
        }
    }
    int_out.reverse();
    let mut out: String = int_out.into_iter().collect();
    if decimal_index.is_some() {
        let mut frac_out = String::new();
        let frac_chars: Vec<char> = frac_value.chars().collect();
        let mut index = 0;
        let mut trailing_optional = 0;
        let mut quoted = false;
        let mut tail = String::new();
        let mut symbols = frac_pattern.chars().peekable();
        while let Some(ch) = symbols.next() {
            if ch == '\'' {
                quoted = !quoted;
                continue;
            }
            if quoted {
                tail.push(ch);
                continue;
            }
            match ch {
                '9' | '8' => {
                    frac_out.push(*frac_chars.get(index).unwrap_or(&'0'));
                    index += 1;
                    trailing_optional = 0;
                }
                'z' | 'Z' => {
                    frac_out.push(*frac_chars.get(index).unwrap_or(&'0'));
                    index += 1;
                    trailing_optional += 1;
                }
                '%' => tail.push(PERCENT),
                ')' => tail.push(if negative { ')' } else { ' ' }),
                's' => tail.push(if negative { MINUS } else { ' ' }),
                'S' => tail.push(if negative { MINUS } else { '+' }),
                '$' => tail.push(CURRENCY),
                'C' | 'c' if matches!(symbols.peek(), Some('R' | 'r')) => {
                    let upper = ch == 'C';
                    symbols.next();
                    tail.push_str(match (negative, upper) {
                        (true, true) => "CR",
                        (true, false) => "cr",
                        _ => "  ",
                    });
                }
                'D' | 'd' if matches!(symbols.peek(), Some('B' | 'b')) => {
                    let upper = ch == 'D';
                    symbols.next();
                    tail.push_str(match (negative, upper) {
                        (true, true) => "DB",
                        (true, false) => "db",
                        _ => "  ",
                    });
                }
                'E' => tail.push_str(&format!("E{exponent}")),
                other if other.is_ascii_alphabetic() => {}
                other => tail.push(other),
            }
        }
        // Optional trailing zero digits are dropped.
        let mut trimmed: Vec<char> = frac_out.chars().collect();
        let mut removable = trailing_optional;
        while removable > 0 && trimmed.last() == Some(&'0') {
            trimmed.pop();
            removable -= 1;
        }
        let frac_out: String = trimmed.into_iter().collect();
        if !frac_out.is_empty() {
            out.push(DECIMAL);
            out.push_str(&frac_out);
        }
        out.push_str(&tail);
    }
    let has_sign = unquoted.contains(['s', 'S', '('])
        || unquoted.contains("CR")
        || unquoted.contains("DB")
        || unquoted.contains("cr")
        || unquoted.contains("db");
    if negative && !has_sign && out.chars().any(|c| c.is_ascii_digit() && c != '0') {
        // A leading minus after any currency/literal prefix.
        let insert_at = out
            .char_indices()
            .find(|(_, c)| c.is_ascii_digit())
            .map_or(0, |(i, _)| i);
        let insert_at = if out[..insert_at].contains(CURRENCY) {
            0
        } else {
            insert_at
        };
        out.insert(insert_at, MINUS);
    }
    Some(out)
}

fn strip_quoted_sections(pattern: &str) -> String {
    let mut out = String::new();
    let mut quoted = false;
    for ch in pattern.chars() {
        if ch == '\'' {
            quoted = !quoted;
        } else if !quoted {
            out.push(ch);
        }
    }
    out
}

fn find_unquoted(pattern: &str, wanted: &[char]) -> Option<usize> {
    let mut quoted = false;
    for (index, ch) in pattern.char_indices() {
        if ch == '\'' {
            quoted = !quoted;
        } else if !quoted && wanted.contains(&ch) {
            return Some(index);
        }
    }
    None
}

fn count_unquoted(pattern: &str, wanted: &[char]) -> usize {
    strip_quoted_sections(pattern)
        .chars()
        .filter(|ch| wanted.contains(ch))
        .count()
}

// ---------------------------------------------------------------------
// Parsing (input/edit pictures; FormCalc Date2Num, Time2Num, Parse).
// ---------------------------------------------------------------------

/// Parses `text` with a date pattern (no category wrapper) into
/// (year, month, day).
pub(crate) fn parse_date(text: &str, pattern: &str, locale: &Locale) -> Option<(i32, u32, u32)> {
    let mut input = Cursor::new(text.trim());
    let (mut year, mut month, mut day) = (1900i32, 1u32, 1u32);
    let mut julian = None;
    for (symbol, count, literal) in tokens(pattern) {
        match (symbol, count) {
            ('\'', _) => input.literal(&literal)?,
            ('Y', 2) => {
                let yy = input.digits(2, 2)? as i32;
                year = if yy < 30 { 2000 + yy } else { 1900 + yy };
            }
            ('Y', _) => year = input.digits(4, 4)? as i32,
            ('M', 1) => month = input.digits(1, 2)?,
            ('M', 2) => month = input.digits(2, 2)?,
            ('M', 3) => month = input.name(&locale.months_abbr)? as u32 + 1,
            ('M', _) => month = input.name(&locale.months)? as u32 + 1,
            ('D', 1) => day = input.digits(1, 2)?,
            ('D', _) => day = input.digits(2, 2)?,
            ('J', 1) => julian = Some(input.digits(1, 3)?),
            ('J', _) => julian = Some(input.digits(3, 3)?),
            ('E', 1) | ('e', _) => {
                input.digits(1, 1)?;
            }
            ('E', 3) => {
                input.name(&locale.days_abbr)?;
            }
            ('E', _) => {
                input.name(&locale.days)?;
            }
            ('G', _) => {
                input.name(&locale.eras)?;
            }
            ('w', _) => {
                input.digits(1, 1)?;
            }
            ('W', _) => {
                input.digits(2, 2)?;
            }
            (other, count) => {
                for _ in 0..count {
                    input.literal(&other.to_string())?;
                }
            }
        }
    }
    if !input.at_end() {
        return None;
    }
    if let Some(julian) = julian {
        let mut remaining = julian;
        month = 1;
        while month < 12 && remaining > days_in_month(year, month) {
            remaining -= days_in_month(year, month);
            month += 1;
        }
        day = remaining;
    }
    ((1..=12).contains(&month) && day >= 1 && day <= days_in_month(year, month))
        .then_some((year, month, day))
}

/// Parses `text` with a time pattern into (hour, minute, second, millis).
pub(crate) fn parse_time(
    text: &str,
    pattern: &str,
    locale: &Locale,
) -> Option<(u32, u32, u32, u32)> {
    let mut input = Cursor::new(text.trim());
    let (mut hour, mut minute, mut second, mut millis) = (0u32, 0u32, 0u32, 0u32);
    let mut pm = None;
    for (symbol, count, literal) in tokens(pattern) {
        match (symbol, count) {
            ('\'', _) => input.literal(&literal)?,
            ('H' | 'h' | 'K' | 'k', 1) => hour = input.digits(1, 2)?,
            ('H' | 'h' | 'K' | 'k', _) => hour = input.digits(2, 2)?,
            ('M', 1) => minute = input.digits(1, 2)?,
            ('M', _) => minute = input.digits(2, 2)?,
            ('S', 1) => second = input.digits(1, 2)?,
            ('S', _) => second = input.digits(2, 2)?,
            ('F', _) => millis = input.digits(1, 3)?,
            ('A', _) => pm = Some(input.name(&locale.meridiem)? == 1),
            ('Z' | 'z', _) => input.skip_zone(),
            (other, count) => {
                for _ in 0..count {
                    input.literal(&other.to_string())?;
                }
            }
        }
    }
    input.skip_spaces();
    if !input.at_end() {
        return None;
    }
    match pm {
        Some(true) if hour < 12 => hour += 12,
        Some(false) if hour == 12 => hour = 0,
        _ => {}
    }
    (hour < 24 && minute < 60 && second < 60).then_some((hour, minute, second, millis))
}

/// Parses a localized number with a numeric pattern (lenient: grouping,
/// currency, percent and literals are skipped; parentheses, `CR`/`DB` and
/// signs make it negative).
pub(crate) fn parse_number(text: &str, pattern: &str, locale: &Locale) -> Option<f64> {
    let text = text.trim();
    if text.is_empty() {
        return None;
    }
    let mut negative = false;
    let mut digits = String::new();
    let mut seen_decimal = false;
    let mut rest = text;
    while let Some(ch) = rest.chars().next() {
        let len = ch.len_utf8();
        if ch.is_ascii_digit() {
            digits.push(ch);
        } else if !seen_decimal
            && (rest.starts_with(locale.decimal.as_str())
                || rest.starts_with(locale.currency_decimal.as_str()))
            && !locale.decimal.is_empty()
        {
            digits.push('.');
            seen_decimal = true;
            rest = &rest[locale.decimal.len().max(1)..];
            continue;
        } else if ch == '-'
            || ch == '('
            || rest.starts_with(locale.minus.as_str())
            || ["CR", "DB", "cr", "db"]
                .iter()
                .any(|sign| rest.starts_with(sign))
        {
            negative = true;
        }
        rest = &rest[len..];
    }
    if digits.is_empty() || digits == "." {
        return None;
    }
    let mut value = digits.parse::<f64>().ok()?;
    if pattern.contains('%') || text.contains(locale.percent.as_str()) {
        value /= 100.0;
    }
    Some(if negative { -value } else { value })
}

struct Cursor<'a> {
    rest: &'a str,
}

impl<'a> Cursor<'a> {
    fn new(text: &'a str) -> Self {
        Self { rest: text }
    }

    fn at_end(&self) -> bool {
        self.rest.is_empty()
    }

    fn skip_spaces(&mut self) {
        self.rest = self.rest.trim_start();
    }

    fn literal(&mut self, literal: &str) -> Option<()> {
        if let Some(rest) = self.rest.strip_prefix(literal) {
            self.rest = rest;
            return Some(());
        }
        // Whitespace literals match any run of whitespace.
        if literal.trim().is_empty() {
            self.rest = self.rest.trim_start();
            return Some(());
        }
        None
    }

    fn digits(&mut self, min: usize, max: usize) -> Option<u32> {
        let count = self
            .rest
            .bytes()
            .take(max)
            .take_while(u8::is_ascii_digit)
            .count();
        if count < min {
            return None;
        }
        let value = self.rest[..count].parse::<u32>().ok()?;
        self.rest = &self.rest[count..];
        Some(value)
    }

    /// Longest case-insensitive name match; returns its index.
    fn name(&mut self, names: &[String]) -> Option<usize> {
        let lower = self.rest.to_lowercase();
        let (index, len) = names
            .iter()
            .enumerate()
            .filter(|(_, name)| !name.is_empty() && lower.starts_with(&name.to_lowercase()))
            .map(|(index, name)| (index, name.len()))
            .max_by_key(|(_, len)| *len)?;
        self.rest = self.rest.get(len..).unwrap_or("");
        Some(index)
    }

    fn skip_zone(&mut self) {
        let trimmed = self.rest.trim_start();
        let end = trimmed
            .char_indices()
            .find(|(_, c)| !(c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | ':')))
            .map_or(trimmed.len(), |(i, _)| i);
        self.rest = &trimmed[end..];
    }
}

/// Parses `text` against a full picture clause (with category prefix and
/// `|` alternatives); returns the canonical value (`YYYY-MM-DD`,
/// `HH:MM:SS`, a number, or the text).
pub(crate) fn parse_value_in(
    text: &str,
    picture: &str,
    kind: &str,
    locales: &LocaleSet,
    locale: Option<&str>,
) -> Option<String> {
    let base = locales.get(locale);
    for alternative in split_alternatives(picture.trim()) {
        let clause = categorize(alternative, kind);
        let locale = clause
            .locale
            .map(|name| locales.get(Some(name)))
            .unwrap_or(base);
        let pattern = clause.pattern(locale);
        let parsed = match clause.category {
            Category::Date => {
                parse_date(text, &pattern, locale).map(|(y, m, d)| format!("{y:04}-{m:02}-{d:02}"))
            }
            Category::Time => parse_time(text, &pattern, locale)
                .map(|(h, m, s, _)| format!("{h:02}:{m:02}:{s:02}")),
            Category::DateTime => {
                let (date_pattern, time_pattern) =
                    pattern.split_once('T').unwrap_or((&pattern, ""));
                // Split the text at the first whitespace after the date.
                let mut found = None;
                for (index, _) in text.match_indices(char::is_whitespace) {
                    if let (Some((y, m, d)), Some((h, mi, s, _))) = (
                        parse_date(&text[..index], date_pattern, locale),
                        parse_time(&text[index..], time_pattern, locale),
                    ) {
                        found = Some(format!("{y:04}-{m:02}-{d:02}T{h:02}:{mi:02}:{s:02}"));
                        break;
                    }
                }
                found
            }
            Category::Num => parse_number(text, &pattern, locale).map(super::som::format_number),
            Category::Text => {
                let mut out = String::new();
                let mut chars = text.chars();
                let mut ok = true;
                for (symbol, _, literal) in text_tokens(&pattern) {
                    if symbol == '\'' {
                        for expected in literal.chars() {
                            if chars.next() != Some(expected) {
                                ok = false;
                            }
                        }
                    } else if let Some(ch) = chars.next() {
                        out.push(ch);
                    } else {
                        ok = false;
                    }
                }
                (ok && chars.next().is_none()).then_some(out)
            }
            Category::Null | Category::Zero => None,
        };
        if parsed.is_some() {
            return parsed;
        }
    }
    None
}

/// Text-picture tokens: one per symbol character (`9 A O X 0`) or literal.
fn text_tokens(pattern: &str) -> Vec<(char, usize, String)> {
    let mut out = Vec::new();
    for (symbol, count, literal) in tokens(pattern) {
        if symbol == '\'' {
            if literal.len() == 1 && "90".contains(literal.as_str()) {
                out.push(('9', 1, String::new()));
            } else {
                out.push(('\'', 0, literal));
            }
        } else if matches!(symbol, 'A' | 'O' | 'X') {
            for _ in 0..count {
                out.push((symbol, 1, String::new()));
            }
        } else {
            out.push(('\'', 0, std::iter::repeat_n(symbol, count).collect()));
        }
    }
    out
}

/// en_US locale (for callers without a locale set).
#[allow(dead_code)]
pub(crate) fn default_locale() -> &'static Locale {
    en_us()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numeric_pictures() {
        assert_eq!(
            format_value("1234.5", Some("num{zzz,zz9.99}"), "decimal"),
            "1,234.50"
        );
        assert_eq!(
            format_value("12", Some("num{zzz,zz9.99}"), "decimal"),
            "12.00"
        );
        assert_eq!(
            format_value("0.5", Some("num{$z,zz9.99}"), "decimal"),
            "$0.50"
        );
        assert_eq!(format_value("1234567", None, "integer"), "1,234,567");
        assert_eq!(format_value("1234.500", None, "decimal"), "1,234.5");
        assert_eq!(format_value("-5", Some("num{zz9}"), "integer"), "-5");
        assert_eq!(format_value("-5", Some("num{zz9CR}"), "integer"), "5CR");
    }

    #[test]
    fn date_pictures() {
        assert_eq!(
            format_value("2024-03-07", Some("date{MM/DD/YYYY}"), "date"),
            "03/07/2024"
        );
        assert_eq!(format_value("2024-03-07", None, "date"), "Mar 7, 2024");
        assert_eq!(
            format_value("2024-03-07", Some("date{EEEE, MMMM D, YYYY}"), "date"),
            "Thursday, March 7, 2024"
        );
        assert_eq!(
            format_value("13:05:00", Some("time{h:MM A}"), "time"),
            "1:05 PM"
        );
        assert_eq!(format_value("bad", Some("date{MM/DD/YYYY}"), "date"), "bad");
        assert_eq!(format_value("2024-03-07", Some("date{JJJ}"), "date"), "067");
    }

    #[test]
    fn text_pictures() {
        assert_eq!(
            format_value("123456789", Some("text{999-99-9999}"), "text"),
            "123-45-6789"
        );
        assert_eq!(format_value("12", Some("text{999}"), "text"), "12");
    }

    #[test]
    fn localized_pictures() {
        let set = LocaleSet::default();
        let de = |value: &str, picture: Option<&str>, kind: &str| {
            format_value_in(value, picture, kind, &set, Some("de_DE"))
        };
        assert_eq!(de("1234.5", Some("num{zzz,zz9.99}"), "decimal"), "1.234,50");
        assert_eq!(de("1234567.25", None, "decimal"), "1.234.567,25");
        assert_eq!(de("2024-03-07", None, "date"), "07.03.2024");
        assert_eq!(
            de("2024-03-07", Some("date.long{}"), "date"),
            "7. März 2024"
        );
        assert_eq!(
            de("1234.5", Some("num.currency{}"), "decimal"),
            "1.234,50 €"
        );
        assert_eq!(
            format_value_in(
                "1234.5",
                Some("num{z,zz9.99}"),
                "decimal",
                &set,
                Some("fr_FR")
            ),
            "1\u{a0}234,50"
        );
        assert_eq!(
            format_value_in(
                "2024-03-07",
                Some("date.long{}"),
                "date",
                &set,
                Some("ja_JP")
            ),
            "2024年3月7日"
        );
        assert_eq!(
            format_value_in(
                "2024-03-07",
                Some("date(fr_FR){EEEE D MMMM YYYY}"),
                "date",
                &set,
                None
            ),
            "jeudi 7 mars 2024"
        );
        assert_eq!(
            format_value_in(
                "1234.5",
                Some("num.currency{}"),
                "decimal",
                &set,
                Some("en_US")
            ),
            "$1,234.50"
        );
        assert_eq!(
            format_value_in(
                "0.25",
                Some("num.percent{}"),
                "decimal",
                &set,
                Some("en_US")
            ),
            "25%"
        );
    }

    #[test]
    fn parsing_inverts_formatting() {
        let set = LocaleSet::default();
        let en = set.get(None);
        let de = set.get(Some("de_DE"));
        assert_eq!(
            parse_date("Mar 7, 2024", "MMM D, YYYY", en),
            Some((2024, 3, 7))
        );
        assert_eq!(
            parse_date("07.03.2024", "DD.MM.YYYY", de),
            Some((2024, 3, 7))
        );
        assert_eq!(
            parse_date("7. März 2024", "D. MMMM YYYY", de),
            Some((2024, 3, 7))
        );
        assert_eq!(parse_date("2024067", "YYYYJJJ", en), Some((2024, 3, 7)));
        assert_eq!(parse_date("02/30/2024", "MM/DD/YYYY", en), None);
        assert_eq!(parse_time("1:05 PM", "h:MM A", en), Some((13, 5, 0, 0)));
        assert_eq!(parse_number("1.234,50", "z,zz9.99", de), Some(1234.5));
        assert_eq!(parse_number("($1,234.50)", "$z,zz9.99", en), Some(-1234.5));
        assert_eq!(
            parse_value_in(
                "07.03.2024",
                "date{DD.MM.YYYY}",
                "date",
                &set,
                Some("de_DE")
            ),
            Some("2024-03-07".to_string())
        );
    }
}
