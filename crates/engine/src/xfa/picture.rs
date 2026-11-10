//! XFA display picture clauses (en_US locale): `num{...}`, `date{...}`,
//! `time{...}`, `text{...}` and bare patterns, plus PDFium's default
//! display formats for date and numeric values without a picture.

/// Formats `value` with `picture` (alternatives separated by `|`; the first
/// one that applies wins). `kind` is the value type: `date`, `time`,
/// `dateTime`, `decimal`, `float`, `integer` or `text`.
pub(crate) fn format_value(value: &str, picture: Option<&str>, kind: &str) -> String {
    let Some(picture) = picture.map(str::trim).filter(|p| !p.is_empty()) else {
        return default_format(value, kind);
    };
    for alternative in split_alternatives(picture) {
        let (category, pattern) = categorize(alternative, kind);
        let formatted = match category {
            "num" => format_number(value, pattern),
            "date" => parse_iso_date(value).map(|date| format_date(date, pattern)),
            "time" => parse_iso_time(value).map(|time| format_time(time, pattern)),
            "datetime" => format_date_time(value, pattern),
            "text" => format_text(value, pattern),
            _ => None,
        };
        if let Some(formatted) = formatted {
            return formatted;
        }
    }
    value.to_string()
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

fn categorize<'a>(picture: &'a str, kind: &str) -> (&'static str, &'a str) {
    let trimmed = picture.trim();
    for (prefix, category) in [
        ("num", "num"),
        ("date", "date"),
        ("time", "time"),
        ("datetime", "datetime"),
        ("text", "text"),
    ] {
        if let Some(rest) = trimmed.strip_prefix(prefix) {
            let rest = rest.trim_start();
            // num.integer{...}, date(en_US){...}
            let open = rest.find('{');
            if let (Some(open), true) = (open, rest.ends_with('}')) {
                let head = &rest[..open];
                if head.is_empty() || head.starts_with('.') || head.starts_with('(') {
                    return (category, &rest[open + 1..rest.len() - 1]);
                }
            }
        }
    }
    let category = match kind {
        "date" => "date",
        "time" => "time",
        "dateTime" => "datetime",
        "decimal" | "float" | "integer" => "num",
        _ => "text",
    };
    (category, trimmed)
}

fn default_format(value: &str, kind: &str) -> String {
    match kind {
        // en_US medium date: "MMM D, YYYY".
        "date" => parse_iso_date(value)
            .map(|date| format_date(date, "MMM D, YYYY"))
            .unwrap_or_else(|| value.to_string()),
        "decimal" | "float" | "integer" => {
            group_plain_number(value).unwrap_or_else(|| value.to_string())
        }
        _ => value.to_string(),
    }
}

/// "1234567.50" -> "1,234,567.5" (the en_US `z,zzz...zz9.zzz` pattern).
fn group_plain_number(value: &str) -> Option<String> {
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
        out.push('-');
    }
    out.push_str(&group(int_part));
    let frac = frac_part.trim_end_matches('0');
    if !frac.is_empty() {
        out.push('.');
        out.push_str(frac);
    }
    Some(out)
}

fn group(digits: &str) -> String {
    let mut out = String::new();
    for (index, ch) in digits.chars().enumerate() {
        if index > 0 && (digits.len() - index).is_multiple_of(3) {
            out.push(',');
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

const MONTHS: [&str; 12] = [
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
];
const DAYS: [&str; 7] = [
    "Sunday",
    "Monday",
    "Tuesday",
    "Wednesday",
    "Thursday",
    "Friday",
    "Saturday",
];

fn parse_iso_date(value: &str) -> Option<(i32, u32, u32)> {
    let value = value.trim();
    let value = value.split(['T', ' ']).next()?;
    let (year, month, day) = if value.contains('-') {
        let mut parts = value.split('-');
        (parts.next()?, parts.next()?, parts.next()?)
    } else if value.len() == 8 {
        (&value[..4], &value[4..6], &value[6..8])
    } else {
        return None;
    };
    let year = year.parse::<i32>().ok()?;
    let month = month.parse::<u32>().ok().filter(|m| (1..=12).contains(m))?;
    let day = day.parse::<u32>().ok().filter(|d| (1..=31).contains(d))?;
    Some((year, month, day))
}

fn parse_iso_time(value: &str) -> Option<(u32, u32, u32)> {
    let value = value.trim();
    let value = value.rsplit('T').next()?;
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

fn weekday(year: i32, month: u32, day: u32) -> usize {
    // Sakamoto's algorithm.
    let offsets = [0, 3, 2, 5, 0, 3, 5, 1, 4, 6, 2, 4];
    let y = if month < 3 { year - 1 } else { year };
    let value = y + y / 4 - y / 100 + y / 400 + offsets[(month - 1) as usize] + day as i32;
    value.rem_euclid(7) as usize
}

fn format_date((year, month, day): (i32, u32, u32), pattern: &str) -> String {
    let mut out = String::new();
    for (symbol, count, literal) in tokens(pattern) {
        match (symbol, count) {
            ('\'', _) => out.push_str(&literal),
            ('Y', 2) => out.push_str(&format!("{:02}", year.rem_euclid(100))),
            ('Y', _) => out.push_str(&format!("{year:04}")),
            ('M', 1) => out.push_str(&month.to_string()),
            ('M', 2) => out.push_str(&format!("{month:02}")),
            ('M', 3) => out.push_str(&MONTHS[(month - 1) as usize][..3]),
            ('M', _) => out.push_str(MONTHS[(month - 1) as usize]),
            ('D', 1) => out.push_str(&day.to_string()),
            ('D', _) => out.push_str(&format!("{day:02}")),
            ('E', 1) => out.push_str(&(weekday(year, month, day) + 1).to_string()),
            ('E', 3) => out.push_str(&DAYS[weekday(year, month, day)][..3]),
            ('E', _) => out.push_str(DAYS[weekday(year, month, day)]),
            (other, count) => {
                for _ in 0..count {
                    out.push(other);
                }
            }
        }
    }
    out
}

fn format_time((hour, minute, second): (u32, u32, u32), pattern: &str) -> String {
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
            ('M', 1) => out.push_str(&minute.to_string()),
            ('M', _) => out.push_str(&format!("{minute:02}")),
            ('S', 1) => out.push_str(&second.to_string()),
            ('S', _) => out.push_str(&format!("{second:02}")),
            ('A', _) => out.push_str(if hour < 12 { "AM" } else { "PM" }),
            ('Z', _) => {}
            (other, count) => {
                for _ in 0..count {
                    out.push(other);
                }
            }
        }
    }
    out
}

fn format_date_time(value: &str, pattern: &str) -> Option<String> {
    let (date_pattern, time_pattern) = pattern.split_once('T').unwrap_or((pattern, ""));
    let date = parse_iso_date(value)?;
    let mut out = format_date(date, date_pattern);
    if !time_pattern.is_empty() {
        out.push(' ');
        out.push_str(&format_time(
            parse_iso_time(value).unwrap_or((0, 0, 0)),
            time_pattern,
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

/// Numeric picture formatting (`z`, `Z`, `9`, `,`, `.`/`v`, `$`, `%`,
/// `s`/`S`, `(`/`)` and literals).
fn format_number(value: &str, pattern: &str) -> Option<String> {
    let number = value.trim().parse::<f64>().ok().filter(|v| v.is_finite())?;
    let percent = pattern.contains('%');
    let number = if percent { number * 100.0 } else { number };
    let negative = number < 0.0;
    let decimal_index = pattern.find(['.', 'v', 'V']);
    let (int_pattern, frac_pattern) = match decimal_index {
        Some(index) => (&pattern[..index], &pattern[index + 1..]),
        None => (pattern, ""),
    };
    let frac_digits = frac_pattern
        .chars()
        .filter(|ch| matches!(ch, '9' | 'z' | 'Z' | '8'))
        .count();
    let scaled = format!("{:.*}", frac_digits, number.abs());
    let (int_value, frac_value) = scaled.split_once('.').unwrap_or((&scaled, ""));
    let int_value = int_value.trim_start_matches('0');
    // Integral part, filled right to left.
    let mut digits: Vec<char> = int_value.chars().collect();
    let mut int_out: Vec<char> = Vec::new();
    let pattern_chars: Vec<char> = int_pattern.chars().collect();
    let digit_slots = pattern_chars
        .iter()
        .filter(|ch| matches!(ch, '9' | 'z' | 'Z' | '8'))
        .count();
    if digits.len() > digit_slots {
        // Overflow: PDFium leaves the value unformatted.
        return None;
    }
    let mut quoted = false;
    for (position, &ch) in pattern_chars.iter().enumerate().rev() {
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
                    int_out.push(',');
                }
            }
            '$' => int_out.push('$'),
            's' => int_out.push(if negative { '-' } else { ' ' }),
            'S' => int_out.push(if negative { '-' } else { '+' }),
            '(' => int_out.push(if negative { '(' } else { ' ' }),
            ')' => int_out.push(if negative { ')' } else { ' ' }),
            '%' => int_out.push('%'),
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
        for ch in frac_pattern.chars() {
            if ch == '\'' {
                quoted = !quoted;
                continue;
            }
            if quoted {
                frac_out.push(ch);
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
                '%' => frac_out.push('%'),
                ')' => frac_out.push(if negative { ')' } else { ' ' }),
                's' => frac_out.push(if negative { '-' } else { ' ' }),
                other if other.is_ascii_alphabetic() => {}
                other => frac_out.push(other),
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
            out.push('.');
            out.push_str(&frac_out);
        }
    }
    let has_sign = pattern.contains(['s', 'S', '(']);
    if negative && !has_sign && out.chars().any(|c| c.is_ascii_digit() && c != '0') {
        out.insert(0, '-');
    }
    Some(out)
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
    }

    #[test]
    fn text_pictures() {
        assert_eq!(
            format_value("123456789", Some("text{999-99-9999}"), "text"),
            "123-45-6789"
        );
        assert_eq!(format_value("12", Some("text{999}"), "text"), "12");
    }
}
