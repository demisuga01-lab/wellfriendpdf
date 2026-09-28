//! Typed PDF Type 4 calculator compiler/evaluator. PDF numeric syntax is not
//! general PostScript syntax, and conditional blocks are not operand values.
//! Integer/real arithmetic uses an explicit portable i32 / finite-f64 profile.
use super::{MAX_TYPE4_PROGRAM_BYTES, MAX_TYPE4_STACK, MAX_TYPE4_TOKENS};
use std::sync::Arc;

const MAX_DEPTH: usize = 64;

#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) enum Number {
    Integer(i32),
    Real(f64),
}

impl Number {
    fn real(self) -> f64 {
        match self {
            Self::Integer(n) => f64::from(n),
            Self::Real(n) => n,
        }
    }

    fn integer(self) -> Result<i32, Error> {
        match self {
            Self::Integer(n) => Ok(n),
            _ => Err(Error::TypeCheck),
        }
    }

    fn finite(value: f64) -> Result<Self, Error> {
        value
            .is_finite()
            .then_some(Self::Real(value))
            .ok_or(Error::UndefinedResult)
    }

    fn convert_integer(self) -> Result<Self, Error> {
        let value = self.real().trunc();
        if value < f64::from(i32::MIN) || value > f64::from(i32::MAX) || !value.is_finite() {
            return Err(Error::RangeCheck);
        }
        Ok(Self::Integer(value as i32))
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) enum Value {
    Number(Number),
    Bool(bool),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Error {
    StackUnderflow,
    StackOverflow,
    TypeCheck,
    RangeCheck,
    UndefinedResult,
    WorkLimit,
    Cancelled,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Op {
    Add,
    Sub,
    Mul,
    Div,
    Idiv,
    Mod,
    Neg,
    Abs,
    Sqrt,
    Sin,
    Cos,
    Atan,
    Exp,
    Ln,
    Log,
    Cvi,
    Cvr,
    Truncate,
    Floor,
    Ceiling,
    Round,
    Dup,
    Pop,
    Exch,
    Copy,
    Index,
    Roll,
    Eq,
    Ne,
    Gt,
    Ge,
    Lt,
    Le,
    And,
    Or,
    Xor,
    Not,
    Bitshift,
    // Parser markers only: execution never receives unbound branch operators.
    If,
    IfElse,
}

impl Op {
    fn parse(word: &[u8]) -> Option<Self> {
        Some(match word {
            b"add" => Self::Add,
            b"sub" => Self::Sub,
            b"mul" => Self::Mul,
            b"div" => Self::Div,
            b"idiv" => Self::Idiv,
            b"mod" => Self::Mod,
            b"neg" => Self::Neg,
            b"abs" => Self::Abs,
            b"sqrt" => Self::Sqrt,
            b"sin" => Self::Sin,
            b"cos" => Self::Cos,
            b"atan" => Self::Atan,
            b"exp" => Self::Exp,
            b"ln" => Self::Ln,
            b"log" => Self::Log,
            b"cvi" => Self::Cvi,
            b"cvr" => Self::Cvr,
            b"truncate" => Self::Truncate,
            b"floor" => Self::Floor,
            b"ceiling" => Self::Ceiling,
            b"round" => Self::Round,
            b"dup" => Self::Dup,
            b"pop" => Self::Pop,
            b"exch" => Self::Exch,
            b"copy" => Self::Copy,
            b"index" => Self::Index,
            b"roll" => Self::Roll,
            b"eq" => Self::Eq,
            b"ne" => Self::Ne,
            b"gt" => Self::Gt,
            b"ge" => Self::Ge,
            b"lt" => Self::Lt,
            b"le" => Self::Le,
            b"and" => Self::And,
            b"or" => Self::Or,
            b"xor" => Self::Xor,
            b"not" => Self::Not,
            b"bitshift" => Self::Bitshift,
            b"if" => Self::If,
            b"ifelse" => Self::IfElse,
            _ => return None,
        })
    }
}

#[derive(Clone, Debug)]
pub(super) enum Instruction {
    Push(Value),
    Operator(Op),
    Branch {
        yes: Arc<[Instruction]>,
        no: Option<Arc<[Instruction]>>,
    },
}

#[derive(Clone, Copy, Debug)]
enum Token {
    Value(Value),
    Operator(Op),
    Start,
    End,
}

fn whitespace(byte: u8) -> bool {
    matches!(byte, 0 | 9 | 10 | 12 | 13 | 32)
}

fn number(word: &[u8]) -> Option<Number> {
    let mut digits = 0;
    let mut point = false;
    for (index, &byte) in word.iter().enumerate() {
        match byte {
            b'+' | b'-' if index == 0 => {}
            b'.' if !point => point = true,
            b'0'..=b'9' => digits += 1,
            _ => return None,
        }
    }
    if digits == 0 {
        return None;
    }
    let text = std::str::from_utf8(word).ok()?;
    if !point {
        if let Ok(value) = text.parse::<i32>() {
            return Some(Number::Integer(value));
        }
    }
    Number::finite(text.parse::<f64>().ok()?).ok()
}

fn tokenize(program: &[u8]) -> Option<Vec<Token>> {
    let mut tokens = Vec::new();
    let mut cursor = 0;
    while cursor < program.len() {
        if cursor.is_multiple_of(4096) {
            crate::cancel::check_current_cancel("calculator lexical scan").ok()?;
        }
        match program[cursor] {
            byte if whitespace(byte) => {
                cursor += 1;
                continue;
            }
            b'%' => {
                // Comments may contain non-ASCII bytes; only tokens use ASCII
                // numeric/operator syntax. CR/LF terminate a PDF comment.
                while cursor < program.len() && !matches!(program[cursor], b'\r' | b'\n') {
                    if cursor.is_multiple_of(4096) {
                        crate::cancel::check_current_cancel("calculator comment scan").ok()?;
                    }
                    cursor += 1;
                }
                continue;
            }
            b'{' => {
                tokens.push(Token::Start);
                cursor += 1;
            }
            b'}' => {
                tokens.push(Token::End);
                cursor += 1;
            }
            _ => {
                let start = cursor;
                while cursor < program.len()
                    && !whitespace(program[cursor])
                    && !matches!(program[cursor], b'{' | b'}' | b'%')
                {
                    if cursor.is_multiple_of(4096) {
                        crate::cancel::check_current_cancel("calculator token scan").ok()?;
                    }
                    cursor += 1;
                }
                let word = &program[start..cursor];
                let token = match word {
                    b"true" => Token::Value(Value::Bool(true)),
                    b"false" => Token::Value(Value::Bool(false)),
                    _ => {
                        if let Some(value) = number(word) {
                            Token::Value(Value::Number(value))
                        } else {
                            Token::Operator(Op::parse(word)?)
                        }
                    }
                };
                tokens.push(token);
            }
        }
        if tokens.len() > MAX_TYPE4_TOKENS {
            return None;
        }
    }
    Some(tokens)
}

pub(super) fn compile(program: &[u8]) -> Option<(Arc<[Instruction]>, usize)> {
    if program.len() > MAX_TYPE4_PROGRAM_BYTES {
        return None;
    }
    crate::cancel::check_current_cancel("calculator compilation").ok()?;
    let tokens = tokenize(program)?;
    // Conservative retained-code charge, including per-block Arc headers and
    // temporary capacity slack. No source strings or operand procedures remain.
    let storage = tokens
        .len()
        .checked_mul(2 * std::mem::size_of::<Instruction>() + 2 * std::mem::size_of::<usize>())?;
    let mut tokens = tokens.into_iter().peekable();
    if !matches!(tokens.next(), Some(Token::Start)) {
        return None;
    }
    let body = compile_body(&mut tokens, 0)?;
    if tokens.next().is_some() {
        return None;
    }
    Some((body, storage))
}

fn compile_body(
    tokens: &mut std::iter::Peekable<std::vec::IntoIter<Token>>,
    depth: usize,
) -> Option<Arc<[Instruction]>> {
    if depth > MAX_DEPTH {
        return None;
    }
    crate::cancel::check_current_cancel("calculator branch compilation").ok()?;
    let mut body = Vec::new();
    while let Some(token) = tokens.next() {
        match token {
            Token::Value(value) => body.push(Instruction::Push(value)),
            Token::Operator(Op::If | Op::IfElse) => return None,
            Token::Operator(operator) => body.push(Instruction::Operator(operator)),
            Token::Start => {
                let yes = compile_body(tokens, depth + 1)?;
                let no = if matches!(tokens.peek(), Some(Token::Start)) {
                    tokens.next();
                    let no = compile_body(tokens, depth + 1)?;
                    if !matches!(tokens.next(), Some(Token::Operator(Op::IfElse))) {
                        return None;
                    }
                    Some(no)
                } else {
                    if !matches!(tokens.next(), Some(Token::Operator(Op::If))) {
                        return None;
                    }
                    None
                };
                body.push(Instruction::Branch { yes, no });
            }
            Token::End => return Some(body.into()),
        }
    }
    None
}

pub(super) fn initial_stack(inputs: &[f64]) -> Option<Vec<Value>> {
    if inputs.len() > MAX_TYPE4_STACK || inputs.iter().any(|v| !v.is_finite()) {
        return None;
    }
    // The native function API supplies real-valued samples. Integer-only
    // operations require an explicit cvi when applied to an input sample.
    Some(
        inputs
            .iter()
            .map(|&v| Value::Number(Number::Real(v)))
            .collect(),
    )
}

pub(super) fn numeric_outputs(stack: &[Value], count: usize) -> Option<Vec<f64>> {
    if count == 0 || stack.len() != count {
        return None;
    }
    stack
        .iter()
        .map(|value| match value {
            Value::Number(n) if n.real().is_finite() => Some(n.real()),
            _ => None,
        })
        .collect()
}

fn charge(remaining: &mut usize, units: usize) -> Result<(), Error> {
    crate::cancel::check_current_cancel("calculator execution").map_err(|_| Error::Cancelled)?;
    *remaining = remaining.checked_sub(units).ok_or(Error::WorkLimit)?;
    Ok(())
}

pub(super) fn execute(
    program: &[Instruction],
    stack: &mut Vec<Value>,
    depth: usize,
    remaining: &mut usize,
) -> Result<(), Error> {
    if depth > MAX_DEPTH {
        return Err(Error::WorkLimit);
    }
    charge(remaining, stack.len())?;
    check_stack(stack)?;
    for instruction in program {
        charge(remaining, 1)?;
        match instruction {
            Instruction::Push(value) => stack.push(*value),
            Instruction::Operator(operator) => execute_op(*operator, stack, remaining)?,
            Instruction::Branch { yes, no } => {
                let Value::Bool(condition) = stack.pop().ok_or(Error::StackUnderflow)? else {
                    return Err(Error::TypeCheck);
                };
                if condition {
                    execute(yes, stack, depth + 1, remaining)?;
                } else if let Some(no) = no {
                    execute(no, stack, depth + 1, remaining)?;
                }
            }
        }
        charge(remaining, stack.len())?;
        check_stack(stack)?;
    }
    Ok(())
}

fn check_stack(stack: &[Value]) -> Result<(), Error> {
    if stack.len() > MAX_TYPE4_STACK {
        return Err(Error::StackOverflow);
    }
    if stack
        .iter()
        .any(|v| matches!(v, Value::Number(Number::Real(n)) if !n.is_finite()))
    {
        return Err(Error::UndefinedResult);
    }
    Ok(())
}

fn pop_number(stack: &mut Vec<Value>) -> Result<Number, Error> {
    match stack.pop().ok_or(Error::StackUnderflow)? {
        Value::Number(n) => Ok(n),
        _ => Err(Error::TypeCheck),
    }
}

fn pop_integer(stack: &mut Vec<Value>) -> Result<i32, Error> {
    pop_number(stack)?.integer()
}

fn binary_number(op: Op, a: Number, b: Number) -> Result<Number, Error> {
    if let (Number::Integer(a), Number::Integer(b)) = (a, b) {
        let exact = match op {
            Op::Add => a.checked_add(b),
            Op::Sub => a.checked_sub(b),
            Op::Mul => a.checked_mul(b),
            _ => None,
        };
        if let Some(n) = exact {
            return Ok(Number::Integer(n));
        }
    }
    let (a, b) = (a.real(), b.real());
    Number::finite(match op {
        Op::Add => a + b,
        Op::Sub => a - b,
        Op::Mul => a * b,
        _ => return Err(Error::TypeCheck),
    })
}

fn unary_number(op: Op, number: Number) -> Result<Number, Error> {
    if let Number::Integer(value) = number {
        let result = match op {
            Op::Neg => value.checked_neg(),
            Op::Abs => value.checked_abs(),
            Op::Floor | Op::Ceiling | Op::Round | Op::Truncate => Some(value),
            _ => None,
        };
        if let Some(value) = result {
            return Ok(Number::Integer(value));
        }
    }
    let value = number.real();
    Number::finite(match op {
        Op::Neg => -value,
        Op::Abs => value.abs(),
        Op::Floor => value.floor(),
        Op::Ceiling => value.ceil(),
        Op::Truncate => value.trunc(),
        Op::Round => {
            // Do not use round() (negative ties go the wrong way), or
            // floor(x+.5) (large integral reals can spuriously increment).
            let low = value.floor();
            if value - low >= 0.5 {
                low + 1.0
            } else {
                low
            }
        }
        _ => return Err(Error::TypeCheck),
    })
}

fn equal(a: Value, b: Value) -> bool {
    match (a, b) {
        (Value::Number(a), Value::Number(b)) => a.real() == b.real(),
        (Value::Bool(a), Value::Bool(b)) => a == b,
        _ => false,
    }
}

fn logical(op: Op, a: Value, b: Value) -> Result<Value, Error> {
    Ok(match (a, b) {
        (Value::Bool(a), Value::Bool(b)) => Value::Bool(match op {
            Op::And => a & b,
            Op::Or => a | b,
            Op::Xor => a ^ b,
            _ => return Err(Error::TypeCheck),
        }),
        (Value::Number(Number::Integer(a)), Value::Number(Number::Integer(b))) => {
            Value::Number(Number::Integer(match op {
                Op::And => a & b,
                Op::Or => a | b,
                Op::Xor => a ^ b,
                _ => return Err(Error::TypeCheck),
            }))
        }
        _ => return Err(Error::TypeCheck),
    })
}

fn trig(op: Op, degrees: f64) -> f64 {
    let angle = degrees.rem_euclid(360.0);
    match (op, angle) {
        (Op::Sin, 0.0 | 180.0 | 360.0) | (Op::Cos, 90.0 | 270.0) => 0.0,
        (Op::Sin, 90.0) | (Op::Cos, 0.0 | 360.0) => 1.0,
        (Op::Sin, 270.0) | (Op::Cos, 180.0) => -1.0,
        (Op::Sin, _) => angle.to_radians().sin(),
        _ => angle.to_radians().cos(),
    }
}

fn execute_op(op: Op, stack: &mut Vec<Value>, remaining: &mut usize) -> Result<(), Error> {
    use Op::*;
    match op {
        Add | Sub | Mul => {
            let b = pop_number(stack)?;
            let a = pop_number(stack)?;
            stack.push(Value::Number(binary_number(op, a, b)?));
        }
        Div => {
            let b = pop_number(stack)?.real();
            let a = pop_number(stack)?.real();
            if b == 0.0 {
                return Err(Error::UndefinedResult);
            }
            stack.push(Value::Number(Number::finite(a / b)?));
        }
        Idiv | Mod => {
            let b = pop_integer(stack)?;
            let a = pop_integer(stack)?;
            if b == 0 {
                return Err(Error::UndefinedResult);
            }
            let result = if op == Idiv {
                a.checked_div(b).ok_or(Error::UndefinedResult)?
            } else {
                (i64::from(a) % i64::from(b)) as i32
            };
            stack.push(Value::Number(Number::Integer(result)));
        }
        Neg | Abs | Truncate | Floor | Ceiling | Round => {
            let value = pop_number(stack)?;
            stack.push(Value::Number(unary_number(op, value)?));
        }
        Sqrt | Sin | Cos | Ln | Log => {
            let value = pop_number(stack)?.real();
            let value = match op {
                Sqrt if value >= 0.0 => value.sqrt(),
                Sin | Cos => trig(op, value),
                Ln if value > 0.0 => value.ln(),
                Log if value > 0.0 => value.log10(),
                _ => return Err(Error::UndefinedResult),
            };
            stack.push(Value::Number(Number::finite(value)?));
        }
        Atan => {
            let den = pop_number(stack)?.real();
            let num = pop_number(stack)?.real();
            if den == 0.0 && num == 0.0 {
                return Err(Error::UndefinedResult);
            }
            let angle = num.atan2(den).to_degrees().rem_euclid(360.0);
            stack.push(Value::Number(Number::finite(if angle == 360.0 {
                0.0
            } else {
                angle
            })?));
        }
        Exp => {
            let exponent = pop_number(stack)?.real();
            let base = pop_number(stack)?.real();
            if (base < 0.0 && exponent.fract() != 0.0) || (base == 0.0 && exponent < 0.0) {
                return Err(Error::UndefinedResult);
            }
            stack.push(Value::Number(Number::finite(base.powf(exponent))?));
        }
        Cvi | Cvr => {
            let value = pop_number(stack)?;
            stack.push(Value::Number(if op == Cvi {
                value.convert_integer()?
            } else {
                Number::finite(value.real())?
            }));
        }
        Dup => {
            let value = *stack.last().ok_or(Error::StackUnderflow)?;
            stack.push(value);
        }
        Pop => {
            stack.pop().ok_or(Error::StackUnderflow)?;
        }
        Exch => {
            let len = stack.len();
            if len < 2 {
                return Err(Error::StackUnderflow);
            }
            stack.swap(len - 1, len - 2);
        }
        Copy => {
            let count = usize::try_from(pop_integer(stack)?).map_err(|_| Error::RangeCheck)?;
            if count > stack.len() {
                return Err(Error::StackUnderflow);
            }
            if stack.len().checked_add(count).ok_or(Error::StackOverflow)? > MAX_TYPE4_STACK {
                return Err(Error::StackOverflow);
            }
            charge(remaining, count)?;
            let start = stack.len() - count;
            for index in 0..count {
                stack.push(stack[start + index]);
            }
        }
        Index => {
            let index = usize::try_from(pop_integer(stack)?).map_err(|_| Error::RangeCheck)?;
            if index >= stack.len() {
                return Err(Error::StackUnderflow);
            }
            stack.push(stack[stack.len() - index - 1]);
        }
        Roll => {
            let shift = pop_integer(stack)?;
            let count = usize::try_from(pop_integer(stack)?).map_err(|_| Error::RangeCheck)?;
            if count > stack.len() {
                return Err(Error::StackUnderflow);
            }
            charge(remaining, count)?;
            if count > 0 {
                let start = stack.len() - count;
                stack[start..].rotate_right(shift.rem_euclid(count as i32) as usize);
            }
        }
        Eq | Ne => {
            let b = stack.pop().ok_or(Error::StackUnderflow)?;
            let a = stack.pop().ok_or(Error::StackUnderflow)?;
            let same = equal(a, b);
            stack.push(Value::Bool(if op == Eq { same } else { !same }));
        }
        Gt | Ge | Lt | Le => {
            let b = pop_number(stack)?.real();
            let a = pop_number(stack)?.real();
            stack.push(Value::Bool(match op {
                Gt => a > b,
                Ge => a >= b,
                Lt => a < b,
                _ => a <= b,
            }));
        }
        And | Or | Xor => {
            let b = stack.pop().ok_or(Error::StackUnderflow)?;
            let a = stack.pop().ok_or(Error::StackUnderflow)?;
            stack.push(logical(op, a, b)?);
        }
        Not => {
            let result = match stack.pop().ok_or(Error::StackUnderflow)? {
                Value::Bool(value) => Value::Bool(!value),
                Value::Number(Number::Integer(value)) => Value::Number(Number::Integer(!value)),
                _ => return Err(Error::TypeCheck),
            };
            stack.push(result);
        }
        Bitshift => {
            let shift = pop_integer(stack)?;
            let bits = pop_integer(stack)? as u32;
            let amount = shift.unsigned_abs();
            let result = if amount >= 32 {
                0
            } else if shift >= 0 {
                bits << amount
            } else {
                bits >> amount
            };
            stack.push(Value::Number(Number::Integer(result as i32)));
        }
        If | IfElse => return Err(Error::TypeCheck),
    }
    Ok(())
}

#[cfg(test)]
#[path = "calculator_function_tests.rs"]
mod tests;
