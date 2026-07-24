use super::{RuntimeInstant, XfaLimits};
use crate::error::{Result, WellfriendError};
use std::collections::BTreeMap;

#[derive(Debug, Clone)]
pub(crate) struct EvalOutcome {
    pub value: String,
    pub instructions: usize,
}

#[derive(Debug, Clone, PartialEq)]
enum Token {
    Number(f64),
    String(String),
    Identifier(String),
    LParen,
    RParen,
    Comma,
    Plus,
    Minus,
    Star,
    Slash,
    Amp,
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
    And,
    Or,
    Not,
    If,
    Then,
    Else,
    EndIf,
}

#[derive(Debug, Clone, PartialEq)]
enum Value {
    Number(f64),
    String(String),
    Bool(bool),
    Null,
}

impl Value {
    fn as_bool(&self) -> bool {
        match self {
            Self::Bool(value) => *value,
            Self::Number(value) => *value != 0.0,
            Self::String(value) => !value.is_empty(),
            Self::Null => false,
        }
    }

    fn as_number(&self) -> Result<f64> {
        let value = match self {
            Self::Number(value) => *value,
            Self::Bool(value) => i32::from(*value) as f64,
            Self::String(value) => value.parse::<f64>().map_err(|_| {
                WellfriendError::MalformedPdf("FormCalc numeric coercion failed".to_string())
            })?,
            Self::Null => 0.0,
        };
        if value.is_finite() {
            Ok(value)
        } else {
            Err(WellfriendError::MalformedPdf(
                "FormCalc produced a non-finite number".to_string(),
            ))
        }
    }

    fn into_string(self) -> String {
        match self {
            Self::Number(value) => format_number(value),
            Self::String(value) => value,
            Self::Bool(value) => value.to_string(),
            Self::Null => String::new(),
        }
    }
}

pub(crate) fn evaluate_formcalc(
    source: &str,
    fields: &BTreeMap<String, String>,
    limits: &XfaLimits,
    started: RuntimeInstant,
) -> Result<EvalOutcome> {
    reject_side_effects(source)?;
    if source.len() > limits.max_script_source_bytes {
        return Err(WellfriendError::ResourceLimit(format!(
            "XFA script source exceeds cap {}",
            limits.max_script_source_bytes
        )));
    }
    let tokens = tokenize(source, limits)?;
    let mut budget = Budget {
        instructions: 0,
        call_depth: 0,
        started,
        limits: limits.clone(),
    };
    let value = eval_tokens(&tokens, fields, &mut budget)?;
    let value = value.into_string();
    if value.len() > limits.max_script_string_bytes {
        return Err(WellfriendError::ResourceLimit(format!(
            "XFA script string result exceeds cap {}",
            limits.max_script_string_bytes
        )));
    }
    Ok(EvalOutcome {
        value,
        instructions: budget.instructions,
    })
}

fn reject_side_effects(source: &str) -> Result<()> {
    let lower = source.to_ascii_lowercase();
    const BLOCKED: &[(&str, &str)] = &[
        ("eval", "dynamic code evaluation"),
        ("exec", "process execution"),
        ("shell", "process execution"),
        ("xfa.host", "host API access"),
        ("app.", "host API access"),
        ("get(", "external data access"),
        ("post(", "network access"),
        ("put(", "external data access"),
        ("delete(", "external data mutation"),
        ("resolveNode", "arbitrary DOM access"),
        ("resolvenode", "arbitrary DOM access"),
        ("while", "loops"),
        (" for ", "loops"),
        ("foreach", "loops"),
        (" do ", "loops"),
    ];
    if let Some((_, reason)) = BLOCKED.iter().find(|(needle, _)| lower.contains(needle)) {
        return Err(WellfriendError::UnsupportedFeature(format!(
            "FormCalc sandbox blocked {reason}"
        )));
    }
    Ok(())
}

fn tokenize(source: &str, limits: &XfaLimits) -> Result<Vec<Token>> {
    let chars: Vec<char> = source.chars().collect();
    let mut out = Vec::new();
    let mut pos = 0usize;
    while pos < chars.len() {
        let ch = chars[pos];
        if ch.is_whitespace() || ch == ';' {
            pos += 1;
            continue;
        }
        let token = match ch {
            '(' => {
                pos += 1;
                Token::LParen
            }
            ')' => {
                pos += 1;
                Token::RParen
            }
            ',' => {
                pos += 1;
                Token::Comma
            }
            '+' => {
                pos += 1;
                Token::Plus
            }
            '-' => {
                pos += 1;
                Token::Minus
            }
            '*' => {
                pos += 1;
                Token::Star
            }
            '/' => {
                pos += 1;
                Token::Slash
            }
            '&' => {
                pos += 1;
                Token::Amp
            }
            '=' => {
                pos += 1;
                if chars.get(pos) == Some(&'=') {
                    pos += 1;
                }
                Token::Eq
            }
            '!' if chars.get(pos + 1) == Some(&'=') => {
                pos += 2;
                Token::Ne
            }
            '<' => {
                pos += 1;
                if chars.get(pos) == Some(&'=') {
                    pos += 1;
                    Token::Le
                } else if chars.get(pos) == Some(&'>') {
                    pos += 1;
                    Token::Ne
                } else {
                    Token::Lt
                }
            }
            '>' => {
                pos += 1;
                if chars.get(pos) == Some(&'=') {
                    pos += 1;
                    Token::Ge
                } else {
                    Token::Gt
                }
            }
            '\'' | '"' => {
                let quote = ch;
                pos += 1;
                let mut value = String::new();
                while pos < chars.len() && chars[pos] != quote {
                    if chars[pos] == '\\' {
                        pos += 1;
                        let escaped = *chars.get(pos).ok_or_else(|| {
                            WellfriendError::MalformedPdf(
                                "unterminated FormCalc string escape".into(),
                            )
                        })?;
                        value.push(match escaped {
                            'n' => '\n',
                            'r' => '\r',
                            't' => '\t',
                            other => other,
                        });
                    } else {
                        value.push(chars[pos]);
                    }
                    pos += 1;
                    if value.len() > limits.max_script_string_bytes {
                        return Err(WellfriendError::ResourceLimit(format!(
                            "XFA script string exceeds cap {}",
                            limits.max_script_string_bytes
                        )));
                    }
                }
                if chars.get(pos) != Some(&quote) {
                    return Err(WellfriendError::MalformedPdf(
                        "unterminated FormCalc string".to_string(),
                    ));
                }
                pos += 1;
                Token::String(value)
            }
            digit if digit.is_ascii_digit() || digit == '.' => {
                let start = pos;
                pos += 1;
                while chars.get(pos).is_some_and(|ch| {
                    ch.is_ascii_digit() || matches!(ch, '.' | 'e' | 'E' | '+' | '-')
                }) {
                    if matches!(chars[pos], '+' | '-')
                        && !matches!(chars.get(pos.wrapping_sub(1)), Some('e' | 'E'))
                    {
                        break;
                    }
                    pos += 1;
                }
                let text: String = chars[start..pos].iter().collect();
                let number = text.parse::<f64>().map_err(|_| {
                    WellfriendError::MalformedPdf("invalid FormCalc number".to_string())
                })?;
                if !number.is_finite() {
                    return Err(WellfriendError::MalformedPdf(
                        "non-finite FormCalc number is forbidden".to_string(),
                    ));
                }
                Token::Number(number)
            }
            ident if is_identifier_char(ident) => {
                let start = pos;
                pos += 1;
                while chars.get(pos).is_some_and(|ch| is_identifier_char(*ch)) {
                    pos += 1;
                }
                let value: String = chars[start..pos].iter().collect();
                match value.to_ascii_lowercase().as_str() {
                    "and" => Token::And,
                    "or" => Token::Or,
                    "not" => Token::Not,
                    "if" => Token::If,
                    "then" => Token::Then,
                    "else" => Token::Else,
                    "endif" => Token::EndIf,
                    _ => Token::Identifier(value),
                }
            }
            _ => {
                return Err(WellfriendError::UnsupportedFeature(
                    "FormCalc construct is outside the pure-expression subset".to_string(),
                ))
            }
        };
        out.push(token);
        if out.len() > limits.max_script_instructions {
            return Err(WellfriendError::ResourceLimit(format!(
                "XFA script token count exceeds instruction cap {}",
                limits.max_script_instructions
            )));
        }
    }
    Ok(out)
}

fn is_identifier_char(ch: char) -> bool {
    ch.is_alphanumeric() || matches!(ch, '_' | '$' | '.' | '[' | ']' | '#' | ':')
}

struct Budget {
    instructions: usize,
    call_depth: usize,
    started: RuntimeInstant,
    limits: XfaLimits,
}

impl Budget {
    fn charge(&mut self) -> Result<()> {
        self.instructions = self.instructions.saturating_add(1);
        if self.instructions > self.limits.max_script_instructions {
            return Err(WellfriendError::ResourceLimit(format!(
                "XFA script instruction cap {} exceeded",
                self.limits.max_script_instructions
            )));
        }
        if self.started.elapsed_millis() > u128::from(self.limits.max_runtime_ms) {
            return Err(WellfriendError::ResourceLimit(format!(
                "XFA runtime exceeded {} ms",
                self.limits.max_runtime_ms
            )));
        }
        Ok(())
    }

    fn enter(&mut self) -> Result<()> {
        self.call_depth = self.call_depth.saturating_add(1);
        if self.call_depth > self.limits.max_script_call_depth {
            return Err(WellfriendError::ResourceLimit(format!(
                "XFA script call depth exceeds cap {}",
                self.limits.max_script_call_depth
            )));
        }
        Ok(())
    }

    fn leave(&mut self) {
        self.call_depth = self.call_depth.saturating_sub(1);
    }
}

fn eval_tokens(
    tokens: &[Token],
    fields: &BTreeMap<String, String>,
    budget: &mut Budget,
) -> Result<Value> {
    if tokens.first() == Some(&Token::If) {
        let then = find_keyword(tokens, Token::Then, 1).ok_or_else(|| {
            WellfriendError::MalformedPdf("FormCalc if expression is missing then".into())
        })?;
        let else_pos = find_keyword(tokens, Token::Else, then + 1).ok_or_else(|| {
            WellfriendError::MalformedPdf("FormCalc if expression is missing else".into())
        })?;
        let end = find_keyword(tokens, Token::EndIf, else_pos + 1).ok_or_else(|| {
            WellfriendError::MalformedPdf("FormCalc if expression is missing endif".into())
        })?;
        if end + 1 != tokens.len() {
            return Err(WellfriendError::UnsupportedFeature(
                "statements after FormCalc endif are outside the subset".to_string(),
            ));
        }
        budget.charge()?;
        let condition = eval_tokens(&tokens[1..then], fields, budget)?.as_bool();
        return if condition {
            eval_tokens(&tokens[then + 1..else_pos], fields, budget)
        } else {
            eval_tokens(&tokens[else_pos + 1..end], fields, budget)
        };
    }
    let mut parser = ExpressionParser {
        tokens,
        pos: 0,
        fields,
        budget,
    };
    let value = parser.parse_or()?;
    if parser.pos != tokens.len() {
        return Err(WellfriendError::UnsupportedFeature(
            "FormCalc statement is outside the pure-expression subset".to_string(),
        ));
    }
    Ok(value)
}

fn find_keyword(tokens: &[Token], target: Token, start: usize) -> Option<usize> {
    tokens
        .iter()
        .enumerate()
        .skip(start)
        .find(|(_, token)| **token == target)
        .map(|(index, _)| index)
}

struct ExpressionParser<'tokens, 'fields, 'budget> {
    tokens: &'tokens [Token],
    pos: usize,
    fields: &'fields BTreeMap<String, String>,
    budget: &'budget mut Budget,
}

impl ExpressionParser<'_, '_, '_> {
    fn parse_or(&mut self) -> Result<Value> {
        let mut value = self.parse_and()?;
        while self.consume(&Token::Or) {
            self.budget.charge()?;
            value = Value::Bool(value.as_bool() || self.parse_and()?.as_bool());
        }
        Ok(value)
    }

    fn parse_and(&mut self) -> Result<Value> {
        let mut value = self.parse_equality()?;
        while self.consume(&Token::And) {
            self.budget.charge()?;
            value = Value::Bool(value.as_bool() && self.parse_equality()?.as_bool());
        }
        Ok(value)
    }

    fn parse_equality(&mut self) -> Result<Value> {
        let mut value = self.parse_comparison()?;
        loop {
            if self.consume(&Token::Eq) {
                self.budget.charge()?;
                value = Value::Bool(compare_values(&value, &self.parse_comparison()?) == 0);
            } else if self.consume(&Token::Ne) {
                self.budget.charge()?;
                value = Value::Bool(compare_values(&value, &self.parse_comparison()?) != 0);
            } else {
                break;
            }
        }
        Ok(value)
    }

    fn parse_comparison(&mut self) -> Result<Value> {
        let mut value = self.parse_term()?;
        loop {
            let operation = if self.consume(&Token::Lt) {
                Some(-2)
            } else if self.consume(&Token::Le) {
                Some(-1)
            } else if self.consume(&Token::Gt) {
                Some(2)
            } else if self.consume(&Token::Ge) {
                Some(1)
            } else {
                None
            };
            let Some(operation) = operation else { break };
            self.budget.charge()?;
            let ordering = compare_values(&value, &self.parse_term()?);
            value = Value::Bool(match operation {
                -2 => ordering < 0,
                -1 => ordering <= 0,
                1 => ordering >= 0,
                _ => ordering > 0,
            });
        }
        Ok(value)
    }

    fn parse_term(&mut self) -> Result<Value> {
        let mut value = self.parse_factor()?;
        loop {
            if self.consume(&Token::Plus) {
                self.budget.charge()?;
                let right = self.parse_factor()?;
                // FormCalc uses `&`/Concat for text. `+` is numeric and
                // therefore coerces field values that originate as XML text.
                value = Value::Number(value.as_number()? + right.as_number()?);
            } else if self.consume(&Token::Minus) {
                self.budget.charge()?;
                value = Value::Number(value.as_number()? - self.parse_factor()?.as_number()?);
            } else if self.consume(&Token::Amp) {
                self.budget.charge()?;
                value = Value::String(value.into_string() + &self.parse_factor()?.into_string());
            } else {
                break;
            }
        }
        Ok(value)
    }

    fn parse_factor(&mut self) -> Result<Value> {
        let mut value = self.parse_unary()?;
        loop {
            if self.consume(&Token::Star) {
                self.budget.charge()?;
                value = Value::Number(value.as_number()? * self.parse_unary()?.as_number()?);
            } else if self.consume(&Token::Slash) {
                self.budget.charge()?;
                let divisor = self.parse_unary()?.as_number()?;
                if divisor == 0.0 {
                    return Err(WellfriendError::MalformedPdf(
                        "FormCalc division by zero".to_string(),
                    ));
                }
                value = Value::Number(value.as_number()? / divisor);
            } else {
                break;
            }
        }
        Ok(value)
    }

    fn parse_unary(&mut self) -> Result<Value> {
        if self.consume(&Token::Minus) {
            self.budget.charge()?;
            return Ok(Value::Number(-self.parse_unary()?.as_number()?));
        }
        if self.consume(&Token::Plus) {
            self.budget.charge()?;
            return Ok(Value::Number(self.parse_unary()?.as_number()?));
        }
        if self.consume(&Token::Not) {
            self.budget.charge()?;
            return Ok(Value::Bool(!self.parse_unary()?.as_bool()));
        }
        self.parse_primary()
    }

    fn parse_primary(&mut self) -> Result<Value> {
        self.budget.charge()?;
        let token = self.tokens.get(self.pos).cloned().ok_or_else(|| {
            WellfriendError::MalformedPdf("incomplete FormCalc expression".into())
        })?;
        self.pos += 1;
        match token {
            Token::Number(value) => Ok(Value::Number(value)),
            Token::String(value) => Ok(Value::String(value)),
            Token::Identifier(name) if self.consume(&Token::LParen) => {
                self.budget.enter()?;
                let result = self.parse_function(&name);
                self.budget.leave();
                result
            }
            Token::Identifier(name) => Ok(resolve_field(&name, self.fields)),
            Token::LParen => {
                let value = self.parse_or()?;
                if !self.consume(&Token::RParen) {
                    return Err(WellfriendError::MalformedPdf(
                        "FormCalc expression is missing ')'".to_string(),
                    ));
                }
                Ok(value)
            }
            _ => Err(WellfriendError::UnsupportedFeature(
                "FormCalc construct is outside the pure-expression subset".to_string(),
            )),
        }
    }

    fn parse_function(&mut self, name: &str) -> Result<Value> {
        let mut args = Vec::new();
        if !self.consume(&Token::RParen) {
            loop {
                args.push(self.parse_or()?);
                if self.consume(&Token::RParen) {
                    break;
                }
                if !self.consume(&Token::Comma) {
                    return Err(WellfriendError::MalformedPdf(
                        "FormCalc function arguments are malformed".to_string(),
                    ));
                }
                if args.len() > self.budget.limits.max_script_object_properties {
                    return Err(WellfriendError::ResourceLimit(
                        "XFA script argument count exceeded object/property cap".to_string(),
                    ));
                }
            }
        }
        pure_function(name, args)
    }

    fn consume(&mut self, token: &Token) -> bool {
        if self.tokens.get(self.pos) == Some(token) {
            self.pos += 1;
            true
        } else {
            false
        }
    }
}

fn resolve_field(name: &str, fields: &BTreeMap<String, String>) -> Value {
    if name.eq_ignore_ascii_case("true") {
        return Value::Bool(true);
    }
    if name.eq_ignore_ascii_case("false") {
        return Value::Bool(false);
    }
    if name.eq_ignore_ascii_case("null") {
        return Value::Null;
    }
    let normalized = name.trim_start_matches("$record.");
    fields
        .get(name)
        .or_else(|| fields.get(normalized))
        .or_else(|| {
            let tail = normalized.rsplit('.').next().unwrap_or(normalized);
            fields
                .iter()
                .find(|(key, _)| {
                    key.rsplit('.')
                        .next()
                        .is_some_and(|segment| segment == tail)
                })
                .map(|(_, value)| value)
        })
        .cloned()
        .map(Value::String)
        .unwrap_or(Value::Null)
}

fn pure_function(name: &str, args: Vec<Value>) -> Result<Value> {
    let lower = name.to_ascii_lowercase();
    match lower.as_str() {
        "sum" => Ok(Value::Number(
            args.iter()
                .map(Value::as_number)
                .collect::<Result<Vec<_>>>()?
                .into_iter()
                .sum(),
        )),
        "avg" | "average" => {
            if args.is_empty() {
                Ok(Value::Number(0.0))
            } else {
                let values = args
                    .iter()
                    .map(Value::as_number)
                    .collect::<Result<Vec<_>>>()?;
                Ok(Value::Number(
                    values.iter().sum::<f64>() / values.len() as f64,
                ))
            }
        }
        "min" => number_extreme(&args, f64::min),
        "max" => number_extreme(&args, f64::max),
        "abs" => one_number(args, f64::abs),
        "round" => one_number(args, f64::round),
        "floor" => one_number(args, f64::floor),
        "ceil" | "ceiling" => one_number(args, f64::ceil),
        "concat" => Ok(Value::String(
            args.into_iter().map(Value::into_string).collect(),
        )),
        "len" => Ok(Value::Number(
            exactly_one(args)?.into_string().chars().count() as f64,
        )),
        "upper" => Ok(Value::String(
            exactly_one(args)?.into_string().to_uppercase(),
        )),
        "lower" => Ok(Value::String(
            exactly_one(args)?.into_string().to_lowercase(),
        )),
        _ => Err(WellfriendError::UnsupportedFeature(format!(
            "FormCalc pure function '{name}' is not whitelisted"
        ))),
    }
}

fn one_number(args: Vec<Value>, op: fn(f64) -> f64) -> Result<Value> {
    let value = exactly_one(args)?.as_number()?;
    let result = op(value);
    if result.is_finite() {
        Ok(Value::Number(result))
    } else {
        Err(WellfriendError::MalformedPdf(
            "FormCalc produced a non-finite number".to_string(),
        ))
    }
}

fn exactly_one(mut args: Vec<Value>) -> Result<Value> {
    if args.len() != 1 {
        return Err(WellfriendError::MalformedPdf(
            "FormCalc function expects exactly one argument".to_string(),
        ));
    }
    Ok(args.remove(0))
}

fn number_extreme(args: &[Value], op: fn(f64, f64) -> f64) -> Result<Value> {
    let mut values = args.iter().map(Value::as_number);
    let Some(first) = values.next() else {
        return Ok(Value::Number(0.0));
    };
    let mut result = first?;
    for value in values {
        result = op(result, value?);
    }
    Ok(Value::Number(result))
}

fn compare_values(left: &Value, right: &Value) -> i8 {
    if let (Ok(left), Ok(right)) = (left.as_number(), right.as_number()) {
        return if left < right {
            -1
        } else if left > right {
            1
        } else {
            0
        };
    }
    left.clone()
        .into_string()
        .cmp(&right.clone().into_string())
        .then(std::cmp::Ordering::Equal) as i8
}

fn format_number(value: f64) -> String {
    if value.fract() == 0.0 {
        format!("{value:.0}")
    } else {
        let formatted = format!("{value:.12}");
        formatted
            .trim_end_matches('0')
            .trim_end_matches('.')
            .to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn evaluates_bounded_formcalc_expression() {
        let fields = BTreeMap::from([
            ("Amount".to_string(), "10".to_string()),
            ("Tax".to_string(), "0.2".to_string()),
        ]);
        let result = evaluate_formcalc(
            "if Amount > 0 then Round(Amount * (1 + Tax)) else 0 endif",
            &fields,
            &XfaLimits::default(),
            RuntimeInstant::now(),
        )
        .unwrap();
        assert_eq!(result.value, "12");
        assert!(result.instructions > 0);
    }

    #[test]
    fn blocks_side_effects_and_instruction_bombs() {
        assert!(evaluate_formcalc(
            "Get('https://example.invalid')",
            &BTreeMap::new(),
            &XfaLimits::default(),
            RuntimeInstant::now(),
        )
        .is_err());
        let limits = XfaLimits {
            max_script_instructions: 2,
            ..XfaLimits::default()
        };
        assert!(evaluate_formcalc(
            "1 + 2 + 3",
            &BTreeMap::new(),
            &limits,
            RuntimeInstant::now(),
        )
        .is_err());
    }
}
