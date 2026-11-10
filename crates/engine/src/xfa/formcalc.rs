//! FormCalc (XFA 3.3 chapter 25; PDFium `xfa/fxfa/formcalc` and
//! `fxjs/xfa/cfxjse_formcalc_context.cpp`): lexer, parser and a direct
//! AST interpreter over the script DOM ([`Dom`]).
//!
//! Supported: numbers, strings, `null`, `nan`, `infinity`; arithmetic,
//! logical (`|`/`or`, `&`/`and`, `not`), equality and relational
//! operators with FormCalc null semantics; `var`, assignment, `if`/
//! `elseif`/`else`, `while`, `for ... upto/downto ... step`, `foreach`,
//! `do ... end`, `func`, `break`, `continue`, `return`; SOM accessors
//! (`a.b[2]`, `[*]`, `..`, `#class`, `$`, `$form`, `!`, method calls);
//! the built-in function library (arithmetic, date/time, financial,
//! logical, string, miscellaneous). URL functions (`Get`, `Post`, `Put`)
//! are refused: scripts never reach the network or the file system.
//!
//! Every evaluation step counts against the DOM's operation budget and
//! deadline; recursion depth and string sizes are bounded.

use std::collections::HashMap;
use std::rc::Rc;

use super::picture;
use super::som::{err, Dom, Obj, SResult, ScriptError, Seg, SomIndex, SomName, Val};

// ---------------------------------------------------------------------
// Lexer
// ---------------------------------------------------------------------

#[derive(Clone, Debug, PartialEq)]
enum Tok {
    Num(f64),
    Str(String),
    Ident(String),
    /// `$`, `$form`, ... (the text after `$`).
    Dollar(String),
    Hash(String),
    LParen,
    RParen,
    LBracket,
    RBracket,
    Comma,
    Dot,
    DotDot,
    DotHash,
    DotStar,
    Assign,
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
    Plus,
    Minus,
    Star,
    Slash,
    And,
    Or,
    Not,
    Kw(Kw),
    Eof,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kw {
    Break,
    Continue,
    Do,
    Downto,
    Else,
    Elseif,
    End,
    Endfor,
    Endfunc,
    Endif,
    Endwhile,
    For,
    Foreach,
    Func,
    If,
    In,
    Infinity,
    Nan,
    Null,
    Return,
    Step,
    Then,
    Upto,
    Var,
    While,
}

fn keyword(word: &str) -> Option<Tok> {
    let lower = word.to_ascii_lowercase();
    Some(match lower.as_str() {
        "and" => Tok::And,
        "or" => Tok::Or,
        "not" => Tok::Not,
        "eq" => Tok::Eq,
        "ne" => Tok::Ne,
        "lt" => Tok::Lt,
        "le" => Tok::Le,
        "gt" => Tok::Gt,
        "ge" => Tok::Ge,
        "break" => Tok::Kw(Kw::Break),
        "continue" => Tok::Kw(Kw::Continue),
        "do" => Tok::Kw(Kw::Do),
        "downto" => Tok::Kw(Kw::Downto),
        "else" => Tok::Kw(Kw::Else),
        "elseif" => Tok::Kw(Kw::Elseif),
        "end" => Tok::Kw(Kw::End),
        "endfor" => Tok::Kw(Kw::Endfor),
        "endfunc" => Tok::Kw(Kw::Endfunc),
        "endif" => Tok::Kw(Kw::Endif),
        "endwhile" => Tok::Kw(Kw::Endwhile),
        "for" => Tok::Kw(Kw::For),
        "foreach" => Tok::Kw(Kw::Foreach),
        "func" => Tok::Kw(Kw::Func),
        "if" => Tok::Kw(Kw::If),
        "in" => Tok::Kw(Kw::In),
        "infinity" => Tok::Kw(Kw::Infinity),
        "nan" => Tok::Kw(Kw::Nan),
        "null" => Tok::Kw(Kw::Null),
        "return" => Tok::Kw(Kw::Return),
        "step" => Tok::Kw(Kw::Step),
        "then" => Tok::Kw(Kw::Then),
        "upto" => Tok::Kw(Kw::Upto),
        "var" => Tok::Kw(Kw::Var),
        "while" => Tok::Kw(Kw::While),
        _ => return None,
    })
}

fn ident_char(ch: char) -> bool {
    ch.is_alphanumeric() || ch == '_'
}

fn lex(source: &str) -> SResult<Vec<Tok>> {
    let chars: Vec<char> = source.chars().collect();
    let mut out = Vec::new();
    let mut pos = 0;
    while pos < chars.len() {
        let ch = chars[pos];
        if ch.is_whitespace() {
            pos += 1;
            continue;
        }
        // Comments: `;` or `//` to end of line.
        if ch == ';' || (ch == '/' && chars.get(pos + 1) == Some(&'/')) {
            while pos < chars.len() && chars[pos] != '\n' && chars[pos] != '\r' {
                pos += 1;
            }
            continue;
        }
        if ch.is_ascii_digit()
            || (ch == '.' && chars.get(pos + 1).is_some_and(|c| c.is_ascii_digit()))
        {
            let start = pos;
            while pos < chars.len() && chars[pos].is_ascii_digit() {
                pos += 1;
            }
            if pos < chars.len() && chars[pos] == '.' {
                pos += 1;
                while pos < chars.len() && chars[pos].is_ascii_digit() {
                    pos += 1;
                }
            }
            if pos < chars.len() && matches!(chars[pos], 'e' | 'E') {
                let save = pos;
                pos += 1;
                if pos < chars.len() && matches!(chars[pos], '+' | '-') {
                    pos += 1;
                }
                if pos < chars.len() && chars[pos].is_ascii_digit() {
                    while pos < chars.len() && chars[pos].is_ascii_digit() {
                        pos += 1;
                    }
                } else {
                    pos = save;
                }
            }
            let text: String = chars[start..pos].iter().collect();
            out.push(Tok::Num(text.parse::<f64>().unwrap_or(0.0)));
            continue;
        }
        if ch == '"' {
            pos += 1;
            let mut text = String::new();
            loop {
                let Some(&c) = chars.get(pos) else {
                    return err("unterminated FormCalc string");
                };
                pos += 1;
                if c == '"' {
                    if chars.get(pos) == Some(&'"') {
                        text.push('"');
                        pos += 1;
                        continue;
                    }
                    break;
                }
                if c == '\\' && matches!(chars.get(pos), Some('u' | 'U')) {
                    let hex: String = chars[pos + 1..(pos + 5).min(chars.len())].iter().collect();
                    if hex.len() == 4 {
                        if let Ok(code) = u32::from_str_radix(&hex, 16) {
                            if let Some(decoded) = char::from_u32(code) {
                                text.push(decoded);
                                pos += 5;
                                continue;
                            }
                        }
                    }
                }
                text.push(c);
            }
            out.push(Tok::Str(text));
            continue;
        }
        if ch == '$' {
            pos += 1;
            let start = pos;
            while pos < chars.len() && (chars[pos].is_alphanumeric() || chars[pos] == '_') {
                pos += 1;
            }
            out.push(Tok::Dollar(chars[start..pos].iter().collect()));
            continue;
        }
        if ch.is_alphabetic() || ch == '_' {
            let start = pos;
            while pos < chars.len() && ident_char(chars[pos]) {
                pos += 1;
            }
            let word: String = chars[start..pos].iter().collect();
            out.push(keyword(&word).unwrap_or(Tok::Ident(word)));
            continue;
        }
        let next = chars.get(pos + 1).copied();
        let (tok, len) = match (ch, next) {
            ('(', _) => (Tok::LParen, 1),
            (')', _) => (Tok::RParen, 1),
            ('[', _) => (Tok::LBracket, 1),
            (']', _) => (Tok::RBracket, 1),
            (',', _) => (Tok::Comma, 1),
            ('.', Some('.')) => (Tok::DotDot, 2),
            ('.', Some('#')) => {
                pos += 2;
                let start = pos;
                while pos < chars.len() && (chars[pos].is_alphanumeric() || chars[pos] == '_') {
                    pos += 1;
                }
                out.push(Tok::DotHash);
                out.push(Tok::Hash(chars[start..pos].iter().collect()));
                continue;
            }
            ('.', Some('*')) => (Tok::DotStar, 2),
            ('.', _) => (Tok::Dot, 1),
            ('#', _) => {
                pos += 1;
                let start = pos;
                while pos < chars.len() && (chars[pos].is_alphanumeric() || chars[pos] == '_') {
                    pos += 1;
                }
                out.push(Tok::Hash(chars[start..pos].iter().collect()));
                continue;
            }
            ('=', Some('=')) => (Tok::Eq, 2),
            ('=', _) => (Tok::Assign, 1),
            ('<', Some('>')) => (Tok::Ne, 2),
            ('<', Some('=')) => (Tok::Le, 2),
            ('<', _) => (Tok::Lt, 1),
            ('>', Some('=')) => (Tok::Ge, 2),
            ('>', _) => (Tok::Gt, 1),
            ('+', _) => (Tok::Plus, 1),
            ('-', _) => (Tok::Minus, 1),
            ('*', _) => (Tok::Star, 1),
            ('/', _) => (Tok::Slash, 1),
            ('&', _) => (Tok::And, 1),
            ('|', _) => (Tok::Or, 1),
            ('!', Some('=')) => (Tok::Ne, 2),
            ('!', _) => (Tok::Not, 1),
            _ => return err(format!("unexpected character '{ch}' in FormCalc")),
        };
        out.push(tok);
        pos += len;
    }
    out.push(Tok::Eof);
    Ok(out)
}

// ---------------------------------------------------------------------
// AST and parser
// ---------------------------------------------------------------------

#[derive(Clone, Debug)]
enum Index {
    All,
    Abs(Expr),
    Rel(Expr, bool),
}

#[derive(Clone, Debug)]
enum PName {
    Name(String),
    Class(String),
    Star,
}

#[derive(Clone, Debug)]
struct PSeg {
    desc: bool,
    name: PName,
    index: Option<Box<Index>>,
    call: Option<Vec<Expr>>,
}

#[derive(Clone, Debug)]
enum Head {
    Ident(String, Option<Box<Index>>),
    Dollar(String),
    Call(String, Vec<Expr>),
    Paren(Box<Expr>),
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Op {
    Or,
    And,
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
    Add,
    Sub,
    Mul,
    Div,
}

#[derive(Clone, Debug)]
struct FuncDef {
    params: Vec<String>,
    body: Vec<Expr>,
}

#[derive(Clone, Debug)]
enum Expr {
    Num(f64),
    Str(String),
    Null,
    Path(Head, Vec<PSeg>),
    Neg(Box<Expr>),
    Pos(Box<Expr>),
    Not(Box<Expr>),
    Bin(Op, Box<Expr>, Box<Expr>),
    Assign(Box<Expr>, Box<Expr>),
    Var(String, Option<Box<Expr>>),
    If(Vec<(Expr, Vec<Expr>)>, Option<Vec<Expr>>),
    While(Box<Expr>, Vec<Expr>),
    For {
        var: String,
        start: Box<Expr>,
        end: Box<Expr>,
        down: bool,
        step: Option<Box<Expr>>,
        body: Vec<Expr>,
    },
    Foreach(String, Vec<Expr>, Vec<Expr>),
    Block(Vec<Expr>),
    Func(String, Rc<FuncDef>),
    Break,
    Continue,
    Return(Option<Box<Expr>>),
}

struct Parser {
    toks: Vec<Tok>,
    pos: usize,
    depth: usize,
}

impl Parser {
    fn peek(&self) -> &Tok {
        &self.toks[self.pos.min(self.toks.len() - 1)]
    }

    fn next(&mut self) -> Tok {
        let tok = self.peek().clone();
        if self.pos < self.toks.len() {
            self.pos += 1;
        }
        tok
    }

    fn eat(&mut self, tok: &Tok) -> bool {
        if self.peek() == tok {
            self.pos += 1;
            true
        } else {
            false
        }
    }

    fn expect(&mut self, tok: &Tok) -> SResult<()> {
        if self.eat(tok) {
            Ok(())
        } else {
            err(format!(
                "FormCalc syntax error: expected {tok:?}, found {:?}",
                self.peek()
            ))
        }
    }

    fn ident(&mut self) -> SResult<String> {
        match self.next() {
            Tok::Ident(name) => Ok(name),
            other => err(format!(
                "FormCalc syntax error: expected a name, found {other:?}"
            )),
        }
    }

    fn enter(&mut self) -> SResult<()> {
        self.depth += 1;
        if self.depth > 200 {
            return err("FormCalc expression nesting too deep");
        }
        Ok(())
    }

    /// Expressions until one of `stops` (not consumed).
    fn list(&mut self, stops: &[Tok]) -> SResult<Vec<Expr>> {
        let mut out = Vec::new();
        loop {
            let tok = self.peek();
            if *tok == Tok::Eof || stops.contains(tok) {
                return Ok(out);
            }
            if *tok == Tok::Comma {
                self.pos += 1;
                continue;
            }
            out.push(self.expression()?);
        }
    }

    fn expression(&mut self) -> SResult<Expr> {
        self.enter()?;
        let result = match self.peek().clone() {
            Tok::Kw(Kw::If) => self.if_expr(),
            Tok::Kw(Kw::While) => {
                self.pos += 1;
                self.expect(&Tok::LParen)?;
                let cond = self.simple()?;
                self.expect(&Tok::RParen)?;
                self.expect(&Tok::Kw(Kw::Do))?;
                let body = self.list(&[Tok::Kw(Kw::Endwhile)])?;
                self.expect(&Tok::Kw(Kw::Endwhile))?;
                Ok(Expr::While(Box::new(cond), body))
            }
            Tok::Kw(Kw::For) => {
                self.pos += 1;
                let var = self.ident()?;
                self.expect(&Tok::Assign)?;
                let start = self.simple()?;
                let down = match self.next() {
                    Tok::Kw(Kw::Upto) => false,
                    Tok::Kw(Kw::Downto) => true,
                    other => {
                        return err(format!("FormCalc: expected upto/downto, found {other:?}"))
                    }
                };
                let end = self.simple()?;
                let step = if self.eat(&Tok::Kw(Kw::Step)) {
                    Some(Box::new(self.simple()?))
                } else {
                    None
                };
                self.expect(&Tok::Kw(Kw::Do))?;
                let body = self.list(&[Tok::Kw(Kw::Endfor)])?;
                self.expect(&Tok::Kw(Kw::Endfor))?;
                Ok(Expr::For {
                    var,
                    start: Box::new(start),
                    end: Box::new(end),
                    down,
                    step,
                    body,
                })
            }
            Tok::Kw(Kw::Foreach) => {
                self.pos += 1;
                let var = self.ident()?;
                self.expect(&Tok::Kw(Kw::In))?;
                self.expect(&Tok::LParen)?;
                let items = self.args()?;
                self.expect(&Tok::Kw(Kw::Do))?;
                let body = self.list(&[Tok::Kw(Kw::Endfor)])?;
                self.expect(&Tok::Kw(Kw::Endfor))?;
                Ok(Expr::Foreach(var, items, body))
            }
            Tok::Kw(Kw::Do) => {
                self.pos += 1;
                let body = self.list(&[Tok::Kw(Kw::End)])?;
                self.expect(&Tok::Kw(Kw::End))?;
                Ok(Expr::Block(body))
            }
            Tok::Kw(Kw::Func) => {
                self.pos += 1;
                let name = self.ident()?;
                self.expect(&Tok::LParen)?;
                let mut params = Vec::new();
                while !self.eat(&Tok::RParen) {
                    params.push(self.ident()?);
                    if !self.eat(&Tok::Comma) {
                        self.expect(&Tok::RParen)?;
                        break;
                    }
                }
                self.expect(&Tok::Kw(Kw::Do))?;
                let body = self.list(&[Tok::Kw(Kw::Endfunc)])?;
                self.expect(&Tok::Kw(Kw::Endfunc))?;
                Ok(Expr::Func(name, Rc::new(FuncDef { params, body })))
            }
            Tok::Kw(Kw::Var) => {
                self.pos += 1;
                let name = self.ident()?;
                let init = if self.eat(&Tok::Assign) {
                    Some(Box::new(self.simple()?))
                } else {
                    None
                };
                Ok(Expr::Var(name, init))
            }
            Tok::Kw(Kw::Break) => {
                self.pos += 1;
                Ok(Expr::Break)
            }
            Tok::Kw(Kw::Continue) => {
                self.pos += 1;
                Ok(Expr::Continue)
            }
            Tok::Kw(Kw::Return) => {
                self.pos += 1;
                let value = match self.peek() {
                    Tok::Eof
                    | Tok::Kw(
                        Kw::End
                        | Kw::Endfor
                        | Kw::Endfunc
                        | Kw::Endif
                        | Kw::Endwhile
                        | Kw::Else
                        | Kw::Elseif,
                    ) => None,
                    _ => Some(Box::new(self.simple()?)),
                };
                Ok(Expr::Return(value))
            }
            _ => {
                let lhs = self.simple()?;
                if self.eat(&Tok::Assign) {
                    let rhs = self.simple()?;
                    Ok(Expr::Assign(Box::new(lhs), Box::new(rhs)))
                } else {
                    Ok(lhs)
                }
            }
        };
        self.depth -= 1;
        result
    }

    fn if_expr(&mut self) -> SResult<Expr> {
        self.pos += 1;
        let mut arms = Vec::new();
        self.expect(&Tok::LParen)?;
        let cond = self.simple()?;
        self.expect(&Tok::RParen)?;
        self.expect(&Tok::Kw(Kw::Then))?;
        let stops = [Tok::Kw(Kw::Elseif), Tok::Kw(Kw::Else), Tok::Kw(Kw::Endif)];
        let body = self.list(&stops)?;
        arms.push((cond, body));
        let mut otherwise = None;
        loop {
            match self.next() {
                Tok::Kw(Kw::Elseif) => {
                    self.expect(&Tok::LParen)?;
                    let cond = self.simple()?;
                    self.expect(&Tok::RParen)?;
                    self.expect(&Tok::Kw(Kw::Then))?;
                    let body = self.list(&stops)?;
                    arms.push((cond, body));
                }
                Tok::Kw(Kw::Else) => {
                    otherwise = Some(self.list(&[Tok::Kw(Kw::Endif)])?);
                    self.expect(&Tok::Kw(Kw::Endif))?;
                    break;
                }
                Tok::Kw(Kw::Endif) => break,
                other => return err(format!("FormCalc: expected endif, found {other:?}")),
            }
        }
        Ok(Expr::If(arms, otherwise))
    }

    fn simple(&mut self) -> SResult<Expr> {
        self.enter()?;
        let result = self.binary(0);
        self.depth -= 1;
        result
    }

    fn binary(&mut self, level: usize) -> SResult<Expr> {
        fn op_at(level: usize, tok: &Tok) -> Option<Op> {
            Some(match (level, tok) {
                (0, Tok::Or) => Op::Or,
                (1, Tok::And) => Op::And,
                (2, Tok::Eq) => Op::Eq,
                (2, Tok::Ne) => Op::Ne,
                (3, Tok::Lt) => Op::Lt,
                (3, Tok::Le) => Op::Le,
                (3, Tok::Gt) => Op::Gt,
                (3, Tok::Ge) => Op::Ge,
                (4, Tok::Plus) => Op::Add,
                (4, Tok::Minus) => Op::Sub,
                (5, Tok::Star) => Op::Mul,
                (5, Tok::Slash) => Op::Div,
                _ => return None,
            })
        }
        if level == 6 {
            return self.unary();
        }
        let mut lhs = self.binary(level + 1)?;
        while let Some(op) = op_at(level, self.peek()) {
            self.pos += 1;
            let rhs = self.binary(level + 1)?;
            lhs = Expr::Bin(op, Box::new(lhs), Box::new(rhs));
        }
        Ok(lhs)
    }

    fn unary(&mut self) -> SResult<Expr> {
        self.enter()?;
        let result = match self.peek() {
            Tok::Minus => {
                self.pos += 1;
                Ok(Expr::Neg(Box::new(self.unary()?)))
            }
            Tok::Plus => {
                self.pos += 1;
                Ok(Expr::Pos(Box::new(self.unary()?)))
            }
            Tok::Not => {
                self.pos += 1;
                Ok(Expr::Not(Box::new(self.unary()?)))
            }
            _ => self.primary(),
        };
        self.depth -= 1;
        result
    }

    fn args(&mut self) -> SResult<Vec<Expr>> {
        // After '(' — up to and including ')'.
        let mut args = Vec::new();
        if self.eat(&Tok::RParen) {
            return Ok(args);
        }
        loop {
            args.push(self.simple()?);
            if self.eat(&Tok::Comma) {
                continue;
            }
            self.expect(&Tok::RParen)?;
            return Ok(args);
        }
    }

    fn index(&mut self) -> SResult<Option<Box<Index>>> {
        if !self.eat(&Tok::LBracket) {
            return Ok(None);
        }
        let index = if self.eat(&Tok::Star) {
            Index::All
        } else if self.eat(&Tok::Plus) {
            Index::Rel(self.simple()?, false)
        } else if self.eat(&Tok::Minus) {
            Index::Rel(self.simple()?, true)
        } else {
            Index::Abs(self.simple()?)
        };
        self.expect(&Tok::RBracket)?;
        Ok(Some(Box::new(index)))
    }

    fn primary(&mut self) -> SResult<Expr> {
        let head = match self.next() {
            Tok::Num(n) => return Ok(Expr::Num(n)),
            Tok::Str(s) => return Ok(Expr::Str(s)),
            Tok::Kw(Kw::Null) => return Ok(Expr::Null),
            Tok::Kw(Kw::Nan) => return Ok(Expr::Num(f64::NAN)),
            Tok::Kw(Kw::Infinity) => return Ok(Expr::Num(f64::INFINITY)),
            Tok::LParen => {
                let inner = self.simple()?;
                self.expect(&Tok::RParen)?;
                Head::Paren(Box::new(inner))
            }
            Tok::Ident(name) => {
                if self.eat(&Tok::LParen) {
                    let args = self.args()?;
                    Head::Call(name, args)
                } else {
                    let index = self.index()?;
                    Head::Ident(name, index)
                }
            }
            Tok::Dollar(name) => Head::Dollar(name),
            other => return err(format!("FormCalc syntax error near {other:?}")),
        };
        let mut segs = Vec::new();
        loop {
            let desc = match self.peek() {
                Tok::Dot => false,
                Tok::DotDot => true,
                Tok::DotHash => {
                    self.pos += 1;
                    let Tok::Hash(class) = self.next() else {
                        return err("FormCalc: expected a class name after .#");
                    };
                    let index = self.index()?;
                    segs.push(PSeg {
                        desc: false,
                        name: PName::Class(class),
                        index,
                        call: None,
                    });
                    continue;
                }
                Tok::DotStar => {
                    self.pos += 1;
                    let index = self.index()?;
                    segs.push(PSeg {
                        desc: false,
                        name: PName::Star,
                        index,
                        call: None,
                    });
                    continue;
                }
                _ => break,
            };
            self.pos += 1;
            let name = match self.next() {
                Tok::Ident(name) => PName::Name(name),
                Tok::Hash(class) => PName::Class(class),
                Tok::Star => PName::Star,
                // Keywords used as property names (`.value`, `.name`...).
                Tok::Kw(_)
                | Tok::And
                | Tok::Or
                | Tok::Not
                | Tok::Eq
                | Tok::Ne
                | Tok::Lt
                | Tok::Le
                | Tok::Gt
                | Tok::Ge => {
                    let text = self.toks[self.pos - 1].clone();
                    PName::Name(keyword_text(&text))
                }
                other => {
                    return err(format!(
                        "FormCalc: expected a name after '.', found {other:?}"
                    ))
                }
            };
            let call = if self.eat(&Tok::LParen) {
                Some(self.args()?)
            } else {
                None
            };
            let index = self.index()?;
            segs.push(PSeg {
                desc,
                name,
                index,
                call,
            });
        }
        Ok(Expr::Path(head, segs))
    }
}

fn keyword_text(tok: &Tok) -> String {
    match tok {
        Tok::And => "and",
        Tok::Or => "or",
        Tok::Not => "not",
        Tok::Eq => "eq",
        Tok::Ne => "ne",
        Tok::Lt => "lt",
        Tok::Le => "le",
        Tok::Gt => "gt",
        Tok::Ge => "ge",
        Tok::Kw(kw) => match kw {
            Kw::Break => "break",
            Kw::Continue => "continue",
            Kw::Do => "do",
            Kw::Downto => "downto",
            Kw::Else => "else",
            Kw::Elseif => "elseif",
            Kw::End => "end",
            Kw::Endfor => "endfor",
            Kw::Endfunc => "endfunc",
            Kw::Endif => "endif",
            Kw::Endwhile => "endwhile",
            Kw::For => "for",
            Kw::Foreach => "foreach",
            Kw::Func => "func",
            Kw::If => "if",
            Kw::In => "in",
            Kw::Infinity => "infinity",
            Kw::Nan => "nan",
            Kw::Null => "null",
            Kw::Return => "return",
            Kw::Step => "step",
            Kw::Then => "then",
            Kw::Upto => "upto",
            Kw::Var => "var",
            Kw::While => "while",
        },
        _ => "",
    }
    .to_string()
}

fn parse(source: &str) -> SResult<Vec<Expr>> {
    let toks = lex(source)?;
    let mut parser = Parser {
        toks,
        pos: 0,
        depth: 0,
    };
    let list = parser.list(&[])?;
    if *parser.peek() != Tok::Eof {
        return err(format!("FormCalc syntax error near {:?}", parser.peek()));
    }
    Ok(list)
}

// ---------------------------------------------------------------------
// Interpreter
// ---------------------------------------------------------------------

enum Flow {
    Normal(Val),
    Break,
    Continue,
    Return(Val),
}

struct Interp<'d> {
    dom: &'d mut Dom,
    this: usize,
    scopes: Vec<HashMap<String, Val>>,
    funcs: Vec<HashMap<String, Rc<FuncDef>>>,
    depth: usize,
}

/// Runs a FormCalc script with `this` = `node`; returns the value of its
/// last expression.
pub(crate) fn run(dom: &mut Dom, node: usize, source: &str) -> SResult<Val> {
    if source.len() > dom.limits.max_source_bytes {
        return err("FormCalc script exceeds the source size limit");
    }
    let program = parse(source)?;
    let mut interp = Interp {
        dom,
        this: node,
        scopes: vec![HashMap::new()],
        funcs: vec![HashMap::new()],
        depth: 0,
    };
    match interp.block(&program)? {
        Flow::Normal(value) | Flow::Return(value) => Ok(value),
        Flow::Break | Flow::Continue => Ok(Val::Null),
    }
}

/// SOM `.[expr]` predicate: truthiness of `expr` with `$` = `node`.
pub(crate) fn predicate(dom: &mut Dom, node: usize, expr: &str) -> SResult<bool> {
    let value = run(dom, node, expr)?;
    let mut interp = Interp {
        dom,
        this: node,
        scopes: vec![HashMap::new()],
        funcs: vec![HashMap::new()],
        depth: 0,
    };
    let value = interp.scalar(value)?;
    Ok(truthy(&value))
}

fn truthy(value: &Val) -> bool {
    match value {
        Val::Null => false,
        Val::Bool(b) => *b,
        Val::Num(n) => *n != 0.0 && !n.is_nan(),
        Val::Str(s) => {
            let n = super::som::parse_leading_number(s);
            n != 0.0
        }
        Val::Obj(_) => true,
    }
}

fn bool_val(value: bool) -> Val {
    Val::Num(if value { 1.0 } else { 0.0 })
}

impl Interp<'_> {
    fn block(&mut self, list: &[Expr]) -> SResult<Flow> {
        let mut last = Val::Null;
        // Hoist function declarations.
        for expr in list {
            if let Expr::Func(name, def) = expr {
                if let Some(scope) = self.funcs.last_mut() {
                    scope.insert(name.to_ascii_lowercase(), def.clone());
                }
            }
        }
        for expr in list {
            match self.eval(expr)? {
                Flow::Normal(value) => last = value,
                other => return Ok(other),
            }
        }
        Ok(Flow::Normal(last))
    }

    fn scoped_block(&mut self, list: &[Expr]) -> SResult<Flow> {
        self.scopes.push(HashMap::new());
        self.funcs.push(HashMap::new());
        let result = self.block(list);
        self.scopes.pop();
        self.funcs.pop();
        result
    }

    fn lookup_var(&self, name: &str) -> Option<&Val> {
        self.scopes.iter().rev().find_map(|scope| scope.get(name))
    }

    fn set_var(&mut self, name: &str, value: Val) -> bool {
        for scope in self.scopes.iter_mut().rev() {
            if let Some(slot) = scope.get_mut(name) {
                *slot = value;
                return true;
            }
        }
        false
    }

    fn value(&mut self, expr: &Expr) -> SResult<Val> {
        match self.eval(expr)? {
            Flow::Normal(value) | Flow::Return(value) => Ok(value),
            _ => Ok(Val::Null),
        }
    }

    /// The value of an object in scalar context.
    fn scalar(&mut self, value: Val) -> SResult<Val> {
        match value {
            Val::Obj(obj) => {
                let inner = self.dom.value_of(&obj)?;
                if let Val::Obj(_) = inner {
                    Ok(Val::Null)
                } else {
                    Ok(inner)
                }
            }
            Val::Bool(b) => Ok(bool_val(b)),
            other => Ok(other),
        }
    }

    fn scalar_of(&mut self, expr: &Expr) -> SResult<Val> {
        let value = self.value(expr)?;
        self.scalar(value)
    }

    fn eval(&mut self, expr: &Expr) -> SResult<Flow> {
        self.dom.tick(1)?;
        let value = match expr {
            Expr::Num(n) => Val::Num(*n),
            Expr::Str(s) => Val::Str(s.clone()),
            Expr::Null => Val::Null,
            Expr::Path(head, segs) => self.path(head, segs)?,
            Expr::Neg(inner) => match self.scalar_of(inner)? {
                Val::Null => Val::Null,
                other => Val::Num(-other.to_num()),
            },
            Expr::Pos(inner) => match self.scalar_of(inner)? {
                Val::Null => Val::Null,
                other => Val::Num(other.to_num()),
            },
            Expr::Not(inner) => match self.scalar_of(inner)? {
                Val::Null => Val::Null,
                other => bool_val(!truthy(&other)),
            },
            Expr::Bin(op, lhs, rhs) => self.binary(*op, lhs, rhs)?,
            Expr::Assign(lhs, rhs) => {
                let value = self.value(rhs)?;
                // Assigning a node reference to a declared variable keeps
                // the reference; everything else assigns values.
                if let Expr::Path(Head::Ident(name, None), segs) = &**lhs {
                    if segs.is_empty() && self.lookup_var(name).is_some() && keeps_object(rhs) {
                        self.set_var(name, value.clone());
                        return Ok(Flow::Normal(value));
                    }
                }
                let value = self.scalar(value)?;
                self.assign(lhs, value.clone())?;
                value
            }
            Expr::Var(name, init) => {
                let value = match init {
                    Some(init) => {
                        let value = self.value(init)?;
                        // Variables hold values, except node references
                        // (`var x = xfa.resolveNode(...)`, `Ref(x)`).
                        if keeps_object(init) {
                            value
                        } else {
                            self.scalar(value)?
                        }
                    }
                    None => Val::Str(String::new()),
                };
                if let Some(scope) = self.scopes.last_mut() {
                    scope.insert(name.clone(), value.clone());
                }
                value
            }
            Expr::If(arms, otherwise) => {
                for (cond, body) in arms {
                    let cond = self.scalar_of(cond)?;
                    if truthy(&cond) {
                        return self.scoped_block(body);
                    }
                }
                match otherwise {
                    Some(body) => return self.scoped_block(body),
                    None => Val::Null,
                }
            }
            Expr::While(cond, body) => {
                let mut last = Val::Null;
                loop {
                    let c = self.scalar_of(cond)?;
                    if !truthy(&c) {
                        break;
                    }
                    match self.scoped_block(body)? {
                        Flow::Normal(value) => last = value,
                        Flow::Break => break,
                        Flow::Continue => continue,
                        Flow::Return(value) => return Ok(Flow::Return(value)),
                    }
                }
                last
            }
            Expr::For {
                var,
                start,
                end,
                down,
                step,
                body,
            } => {
                let start = self.scalar_of(start)?.to_num();
                let end = self.scalar_of(end)?.to_num();
                let step = match step {
                    Some(step) => self.scalar_of(step)?.to_num(),
                    None => 1.0,
                };
                if step <= 0.0 || !step.is_finite() {
                    return err("FormCalc for-loop step must be positive");
                }
                self.scopes.push(HashMap::new());
                let mut last = Val::Null;
                let mut counter = start;
                let mut result = None;
                while if *down {
                    counter >= end
                } else {
                    counter <= end
                } {
                    if let Some(scope) = self.scopes.last_mut() {
                        scope.insert(var.clone(), Val::Num(counter));
                    }
                    match self.scoped_block(body) {
                        Ok(Flow::Normal(value)) => last = value,
                        Ok(Flow::Break) => break,
                        Ok(Flow::Continue) => {}
                        Ok(Flow::Return(value)) => {
                            result = Some(Flow::Return(value));
                            break;
                        }
                        Err(error) => {
                            self.scopes.pop();
                            return Err(error);
                        }
                    }
                    // The body may assign the loop variable.
                    if let Some(Val::Num(current)) = self.lookup_var(var).cloned() {
                        counter = current;
                    }
                    counter = if *down {
                        counter - step
                    } else {
                        counter + step
                    };
                }
                self.scopes.pop();
                if let Some(flow) = result {
                    return Ok(flow);
                }
                last
            }
            Expr::Foreach(var, items, body) => {
                let mut values = Vec::new();
                for item in items {
                    let value = self.value(item)?;
                    self.expand(value, &mut values)?;
                }
                self.scopes.push(HashMap::new());
                let mut last = Val::Null;
                let mut result = None;
                for value in values {
                    if let Some(scope) = self.scopes.last_mut() {
                        scope.insert(var.clone(), value);
                    }
                    match self.scoped_block(body) {
                        Ok(Flow::Normal(value)) => last = value,
                        Ok(Flow::Break) => break,
                        Ok(Flow::Continue) => {}
                        Ok(Flow::Return(value)) => {
                            result = Some(Flow::Return(value));
                            break;
                        }
                        Err(error) => {
                            self.scopes.pop();
                            return Err(error);
                        }
                    }
                }
                self.scopes.pop();
                if let Some(flow) = result {
                    return Ok(flow);
                }
                last
            }
            Expr::Block(body) => return self.scoped_block(body),
            Expr::Func(_, _) => Val::Null,
            Expr::Break => return Ok(Flow::Break),
            Expr::Continue => return Ok(Flow::Continue),
            Expr::Return(value) => {
                let value = match value {
                    Some(value) => self.value(value)?,
                    None => Val::Null,
                };
                return Ok(Flow::Return(value));
            }
        };
        Ok(Flow::Normal(value))
    }

    fn binary(&mut self, op: Op, lhs: &Expr, rhs: &Expr) -> SResult<Val> {
        let a = self.scalar_of(lhs)?;
        let b = self.scalar_of(rhs)?;
        let both_null = a == Val::Null && b == Val::Null;
        let any_null = a == Val::Null || b == Val::Null;
        Ok(match op {
            Op::Or => {
                if both_null {
                    Val::Null
                } else {
                    bool_val(truthy(&a) || truthy(&b))
                }
            }
            Op::And => {
                if both_null {
                    Val::Null
                } else {
                    bool_val(truthy(&a) && truthy(&b))
                }
            }
            Op::Eq | Op::Ne => {
                let equal = if any_null {
                    both_null
                } else if let (Val::Str(x), Val::Str(y)) = (&a, &b) {
                    x == y
                } else {
                    a.to_num() == b.to_num()
                };
                bool_val(if op == Op::Eq { equal } else { !equal })
            }
            Op::Lt | Op::Le | Op::Gt | Op::Ge => {
                if any_null {
                    bool_val(both_null && matches!(op, Op::Le | Op::Ge))
                } else {
                    let ordering = if let (Val::Str(x), Val::Str(y)) = (&a, &b) {
                        x.cmp(y)
                    } else {
                        a.to_num()
                            .partial_cmp(&b.to_num())
                            .unwrap_or(std::cmp::Ordering::Equal)
                    };
                    bool_val(match op {
                        Op::Lt => ordering.is_lt(),
                        Op::Le => ordering.is_le(),
                        Op::Gt => ordering.is_gt(),
                        _ => ordering.is_ge(),
                    })
                }
            }
            Op::Add | Op::Sub | Op::Mul | Op::Div => {
                if both_null {
                    return Ok(Val::Null);
                }
                let (x, y) = (a.to_num(), b.to_num());
                match op {
                    Op::Add => Val::Num(x + y),
                    Op::Sub => Val::Num(x - y),
                    Op::Mul => Val::Num(x * y),
                    _ => {
                        if y == 0.0 {
                            return err("Divide by zero.");
                        }
                        Val::Num(x / y)
                    }
                }
            }
        })
    }

    fn seg_index(&mut self, index: &Option<Box<Index>>) -> SResult<SomIndex> {
        Ok(match index.as_deref() {
            None => SomIndex::Default,
            Some(Index::All) => SomIndex::All,
            Some(Index::Abs(expr)) => SomIndex::At(self.scalar_of(expr)?.to_num() as i64),
            Some(Index::Rel(expr, negative)) => {
                let value = self.scalar_of(expr)?.to_num() as i64;
                SomIndex::Rel(if *negative { -value } else { value })
            }
        })
    }

    fn som_seg(&mut self, seg: &PSeg) -> SResult<Seg> {
        let index = self.seg_index(&seg.index)?;
        Ok(Seg {
            desc: seg.desc,
            name: match &seg.name {
                PName::Name(name) => SomName::Name(name.clone()),
                PName::Class(class) => SomName::Class(class.clone()),
                PName::Star => SomName::Star,
            },
            index,
        })
    }

    /// Evaluates an accessor chain to a value (objects stay objects).
    fn path(&mut self, head: &Head, segs: &[PSeg]) -> SResult<Val> {
        let this = self.this;
        let mut current: Val = match head {
            Head::Paren(inner) => self.value(inner)?,
            Head::Call(name, args) => self.call_function(name, args)?,
            Head::Ident(name, index) => {
                if index.is_none() {
                    if let Some(value) = self.lookup_var(name) {
                        let value = value.clone();
                        if segs.is_empty() {
                            return Ok(value);
                        }
                        // Variables holding objects continue the chain.
                        return self.continue_path(value, segs);
                    }
                }
                let index = self.seg_index(index)?;
                let seg = Seg {
                    desc: false,
                    name: SomName::Name(name.clone()),
                    index,
                };
                let objs = self.dom.start(Some(this), &seg)?;
                if objs.is_empty() {
                    return err(format!("Accessor '{name}' is unknown."));
                }
                objs_val(objs)
            }
            Head::Dollar(name) => {
                let seg = Seg {
                    desc: false,
                    name: SomName::Dollar(name.clone()),
                    index: SomIndex::Default,
                };
                objs_val(self.dom.start(Some(this), &seg)?)
            }
        };
        if segs.is_empty() {
            return Ok(current);
        }
        current = self.continue_path(current, segs)?;
        Ok(current)
    }

    fn continue_path(&mut self, mut current: Val, segs: &[PSeg]) -> SResult<Val> {
        for seg in segs {
            let Val::Obj(obj) = current.clone() else {
                // Properties of scalars (rare): nothing.
                return Ok(Val::Null);
            };
            if let Some(args) = &seg.call {
                let PName::Name(method) = &seg.name else {
                    return err("FormCalc: bad method call");
                };
                current = self.method(&obj, method, args)?;
                continue;
            }
            let som = self.som_seg(seg)?;
            let found = self
                .dom
                .step(std::slice::from_ref(&obj), &som, Some(self.this))?;
            if !found.is_empty() {
                current = objs_val(found);
                continue;
            }
            match &seg.name {
                // `.#text`, `.#value`: properties named like classes.
                PName::Name(name) | PName::Class(name) => {
                    current = self.dom.get(&obj, name)?;
                }
                PName::Star => return Ok(Val::Null),
            }
        }
        Ok(current)
    }

    fn method(&mut self, obj: &Obj, method: &str, args: &[Expr]) -> SResult<Val> {
        // Script objects: FormCalc functions declared in `<variables>`.
        if let Obj::Node(id) = obj {
            if self.dom.nodes[*id].class == super::som::Class::ScriptObject {
                return self.call_script_object(*id, method, args);
            }
        }
        let mut values = Vec::with_capacity(args.len());
        for arg in args {
            let value = self.value(arg)?;
            // Node arguments stay objects (`xfa.layout.page(this)`).
            values.push(match value {
                Val::Obj(Obj::Node(_)) => value,
                other => self.scalar(other)?,
            });
        }
        self.dom.call(obj, Some(self.this), method, &values)
    }

    fn call_script_object(&mut self, id: usize, method: &str, args: &[Expr]) -> SResult<Val> {
        let source = self.dom.nodes[id]
            .scripts
            .first()
            .map(|script| script.source.clone())
            .unwrap_or_default();
        let program = parse(&source)?;
        let def = program.iter().find_map(|expr| match expr {
            Expr::Func(name, def) if name.eq_ignore_ascii_case(method) => Some(def.clone()),
            _ => None,
        });
        let Some(def) = def else {
            return err(format!("function '{method}' not found in script object"));
        };
        let mut values = Vec::new();
        for arg in args {
            values.push(self.value(arg)?);
        }
        self.invoke(&def, values)
    }

    fn assign(&mut self, target: &Expr, value: Val) -> SResult<()> {
        let Expr::Path(head, segs) = target else {
            return err("FormCalc: invalid assignment target");
        };
        if segs.is_empty() {
            match head {
                Head::Ident(name, None) => {
                    if self.set_var(name, value.clone()) {
                        return Ok(());
                    }
                    let seg = Seg {
                        desc: false,
                        name: SomName::Name(name.clone()),
                        index: SomIndex::Default,
                    };
                    let objs = self.dom.start(Some(self.this), &seg)?;
                    return self.assign_objs(objs, value, name);
                }
                Head::Ident(name, Some(index)) => {
                    let index = self.seg_index(&Some(index.clone()))?;
                    let seg = Seg {
                        desc: false,
                        name: SomName::Name(name.clone()),
                        index,
                    };
                    let objs = self.dom.start(Some(self.this), &seg)?;
                    return self.assign_objs(objs, value, name);
                }
                Head::Dollar(name) => {
                    let seg = Seg {
                        desc: false,
                        name: SomName::Dollar(name.clone()),
                        index: SomIndex::Default,
                    };
                    let objs = self.dom.start(Some(self.this), &seg)?;
                    return self.assign_objs(objs, value, "$");
                }
                _ => return err("FormCalc: invalid assignment target"),
            }
        }
        let (last, init) = segs.split_last().expect("non-empty");
        let base = self.path(head, init)?;
        let Val::Obj(base) = base else {
            return err("FormCalc: assignment to a non-object");
        };
        if last.call.is_some() {
            return err("FormCalc: cannot assign to a method call");
        }
        let som = self.som_seg(last)?;
        let found = self
            .dom
            .step(std::slice::from_ref(&base), &som, Some(self.this))?;
        if !found.is_empty() {
            return self.assign_objs(found, value, "");
        }
        match &last.name {
            PName::Name(name) | PName::Class(name) => self.dom.set(&base, name, &value),
            PName::Star => err("FormCalc: invalid assignment target"),
        }
    }

    fn assign_objs(&mut self, objs: Vec<Obj>, value: Val, name: &str) -> SResult<()> {
        let Some(first) = objs.into_iter().next() else {
            return err(format!("Accessor '{name}' is unknown."));
        };
        match first {
            Obj::Node(id) => self.dom.set_value(id, &value),
            Obj::Nodes(ids) => match ids.first() {
                Some(&id) => self.dom.set_value(id, &value),
                None => Ok(()),
            },
            Obj::Prop { node, path } => {
                let mut path = path;
                let last = path.pop().unwrap_or_default();
                self.dom.set(&Obj::Prop { node, path }, &last, &value)
            }
            _ => Ok(()),
        }
    }

    /// Flattens accessor results (`a[*]`) into scalar values.
    fn expand(&mut self, value: Val, out: &mut Vec<Val>) -> SResult<()> {
        match value {
            Val::Obj(Obj::Nodes(ids)) => {
                for id in ids {
                    out.push(self.dom.raw_value(id));
                }
            }
            Val::Obj(obj) => out.push(self.dom.value_of(&obj)?),
            other => out.push(other),
        }
        Ok(())
    }

    fn arg_values(&mut self, args: &[Expr]) -> SResult<Vec<Val>> {
        let mut out = Vec::with_capacity(args.len());
        for arg in args {
            let value = self.value(arg)?;
            out.push(self.scalar(value)?);
        }
        Ok(out)
    }

    fn expanded(&mut self, args: &[Expr]) -> SResult<Vec<Val>> {
        let mut out = Vec::new();
        for arg in args {
            let value = self.value(arg)?;
            self.expand(value, &mut out)?;
        }
        Ok(out)
    }

    fn find_func(&self, name: &str) -> Option<Rc<FuncDef>> {
        let lower = name.to_ascii_lowercase();
        self.funcs
            .iter()
            .rev()
            .find_map(|scope| scope.get(&lower).cloned())
    }

    fn invoke(&mut self, def: &FuncDef, args: Vec<Val>) -> SResult<Val> {
        self.depth += 1;
        if self.depth > self.dom.limits.max_call_depth {
            self.depth -= 1;
            return err("FormCalc call depth limit exceeded");
        }
        let mut frame = HashMap::new();
        for (index, param) in def.params.iter().enumerate() {
            frame.insert(param.clone(), args.get(index).cloned().unwrap_or(Val::Null));
        }
        self.scopes.push(frame);
        self.funcs.push(HashMap::new());
        let result = self.block(&def.body);
        self.scopes.pop();
        self.funcs.pop();
        self.depth -= 1;
        match result? {
            Flow::Normal(value) | Flow::Return(value) => Ok(value),
            _ => Ok(Val::Null),
        }
    }

    fn call_function(&mut self, name: &str, args: &[Expr]) -> SResult<Val> {
        self.dom.tick(2)?;
        if let Some(def) = self.find_func(name) {
            let mut values = Vec::with_capacity(args.len());
            for arg in args {
                values.push(self.value(arg)?);
            }
            return self.invoke(&def, values);
        }
        let lower = name.to_ascii_lowercase();
        match lower.as_str() {
            // Arithmetic.
            "sum" | "avg" | "count" | "max" | "min" => {
                let values = self.expanded(args)?;
                let present: Vec<f64> = values
                    .iter()
                    .filter(|v| !matches!(v, Val::Null))
                    .map(Val::to_num)
                    .collect();
                if lower == "count" {
                    return Ok(Val::Num(present.len() as f64));
                }
                if present.is_empty() {
                    return Ok(Val::Null);
                }
                Ok(Val::Num(match lower.as_str() {
                    "sum" => present.iter().sum(),
                    "avg" => present.iter().sum::<f64>() / present.len() as f64,
                    "max" => present.iter().copied().fold(f64::NEG_INFINITY, f64::max),
                    _ => present.iter().copied().fold(f64::INFINITY, f64::min),
                }))
            }
            "abs" | "ceil" | "floor" => {
                let args = self.arg_values(args)?;
                let Some(value) = args.first() else {
                    return param_error(name);
                };
                if *value == Val::Null {
                    return Ok(Val::Null);
                }
                let n = value.to_num();
                Ok(Val::Num(match lower.as_str() {
                    "abs" => n.abs(),
                    "ceil" => n.ceil(),
                    _ => n.floor(),
                }))
            }
            "mod" => {
                let args = self.arg_values(args)?;
                if args.len() != 2 {
                    return param_error(name);
                }
                if args.contains(&Val::Null) {
                    return Ok(Val::Null);
                }
                let (a, b) = (args[0].to_num(), args[1].to_num());
                if b == 0.0 {
                    return err("Divide by zero.");
                }
                Ok(Val::Num(a % b))
            }
            "round" => {
                let args = self.arg_values(args)?;
                if args.is_empty() || args.len() > 2 {
                    return param_error(name);
                }
                if args.contains(&Val::Null) {
                    return Ok(Val::Null);
                }
                let places = args.get(1).map_or(0.0, Val::to_num).clamp(0.0, 12.0) as i32;
                Ok(Val::Num(round_half_away(args[0].to_num(), places)))
            }
            // Logical.
            "choose" => {
                let values = self.expanded(args)?;
                let Some(index) = values.first() else {
                    return param_error(name);
                };
                if *index == Val::Null {
                    return Ok(Val::Null);
                }
                let index = index.to_num() as i64;
                Ok(usize::try_from(index)
                    .ok()
                    .filter(|i| *i >= 1)
                    .and_then(|i| values.get(i).cloned())
                    .unwrap_or(Val::Str(String::new())))
            }
            "exists" => {
                let Some(arg) = args.first() else {
                    return param_error(name);
                };
                Ok(bool_val(match self.value(arg) {
                    Ok(Val::Obj(_)) => true,
                    Ok(Val::Null) | Err(_) => false,
                    Ok(_) => false,
                }))
            }
            "hasvalue" => {
                let args = self.arg_values(args)?;
                Ok(bool_val(match args.first() {
                    Some(Val::Null) | None => false,
                    Some(Val::Str(s)) => !s.trim().is_empty(),
                    Some(_) => true,
                }))
            }
            "oneof" => {
                let values = self.expanded(args)?;
                let Some((first, rest)) = values.split_first() else {
                    return param_error(name);
                };
                Ok(bool_val(rest.iter().any(|v| loose_equal(first, v))))
            }
            "within" => {
                let args = self.arg_values(args)?;
                if args.len() != 3 {
                    return param_error(name);
                }
                if args[0] == Val::Null {
                    return Ok(Val::Null);
                }
                let within = if let (Val::Str(v), Val::Str(lo), Val::Str(hi)) =
                    (&args[0], &args[1], &args[2])
                {
                    lo <= v && v <= hi
                } else {
                    let v = args[0].to_num();
                    args[1].to_num() <= v && v <= args[2].to_num()
                };
                Ok(bool_val(within))
            }
            // Miscellaneous.
            "null" => Ok(Val::Null),
            "eval" => {
                let args = self.arg_values(args)?;
                let source = args.first().map(Val::to_text).unwrap_or_default();
                self.depth += 1;
                if self.depth > self.dom.limits.max_call_depth {
                    self.depth -= 1;
                    return err("FormCalc call depth limit exceeded");
                }
                let program = parse(&source);
                let result = program.and_then(|program| self.block(&program));
                self.depth -= 1;
                match result? {
                    Flow::Normal(v) | Flow::Return(v) => Ok(v),
                    _ => Ok(Val::Null),
                }
            }
            "ref" => {
                let Some(arg) = args.first() else {
                    return param_error(name);
                };
                self.value(arg)
            }
            "unittype" => {
                let args = self.arg_values(args)?;
                let text = args.first().map(Val::to_text).unwrap_or_default();
                Ok(Val::Str(unit_of(&text).to_string()))
            }
            "unitvalue" => {
                let args = self.arg_values(args)?;
                let Some(first) = args.first() else {
                    return param_error(name);
                };
                if *first == Val::Null {
                    return Ok(Val::Null);
                }
                let text = first.to_text();
                let points = measurement_points(&text);
                let unit = args
                    .get(1)
                    .map(Val::to_text)
                    .unwrap_or_else(|| unit_of(&text).to_string());
                Ok(Val::Num(points_to(points, &unit)))
            }
            // Strings.
            "at" => {
                let args = self.arg_values(args)?;
                if args.len() != 2 {
                    return param_error(name);
                }
                let (hay, needle) = (args[0].to_text(), args[1].to_text());
                if needle.is_empty() {
                    return Ok(Val::Num(1.0));
                }
                Ok(Val::Num(hay.find(&needle).map_or(0.0, |byte| {
                    (hay[..byte].chars().count() + 1) as f64
                })))
            }
            "concat" => {
                let values = self.expanded(args)?;
                if values.iter().all(|v| *v == Val::Null) && !values.is_empty() {
                    return Ok(Val::Null);
                }
                let mut out = String::new();
                for value in values {
                    out.push_str(&value.to_text());
                    self.dom.check_string(&out)?;
                }
                Ok(Val::Str(out))
            }
            "decode" | "encode" => {
                let args = self.arg_values(args)?;
                let Some(first) = args.first() else {
                    return param_error(name);
                };
                if *first == Val::Null {
                    return Ok(Val::Null);
                }
                let text = first.to_text();
                let kind = args
                    .get(1)
                    .map(|k| k.to_text().to_ascii_lowercase())
                    .unwrap_or_else(|| "url".into());
                Ok(Val::Str(if lower == "decode" {
                    decode(&text, &kind)
                } else {
                    encode(&text, &kind)
                }))
            }
            "format" => {
                let args = self.arg_values(args)?;
                if args.len() < 2 {
                    return param_error(name);
                }
                let picture = args[0].to_text();
                let value = args[1].to_text();
                let kind = if picture.trim_start().starts_with("date") {
                    "date"
                } else if picture.trim_start().starts_with("time") {
                    "time"
                } else if picture.trim_start().starts_with("text") {
                    "text"
                } else {
                    "decimal"
                };
                let locale = self.node_locale();
                Ok(Val::Str(picture::format_value_in(
                    &value,
                    Some(&picture),
                    kind,
                    &self.dom.locales,
                    locale.as_deref(),
                )))
            }
            "parse" => {
                let args = self.arg_values(args)?;
                if args.len() != 2 {
                    return param_error(name);
                }
                let picture = args[0].to_text();
                let text = args[1].to_text();
                let locale = self.node_locale();
                let kind = if picture.trim_start().starts_with("date") {
                    "date"
                } else if picture.trim_start().starts_with("time") {
                    "time"
                } else {
                    "decimal"
                };
                Ok(picture::parse_value_in(
                    &text,
                    &picture,
                    kind,
                    &self.dom.locales,
                    locale.as_deref(),
                )
                .map_or(Val::Str(String::new()), |parsed| {
                    if kind == "decimal" {
                        Val::Num(super::som::parse_leading_number(&parsed))
                    } else {
                        Val::Str(parsed)
                    }
                }))
            }
            "left" | "right" => {
                let args = self.arg_values(args)?;
                if args.len() != 2 {
                    return param_error(name);
                }
                if args[0] == Val::Null {
                    return Ok(Val::Null);
                }
                let chars: Vec<char> = args[0].to_text().chars().collect();
                let count = (args[1].to_num().max(0.0) as usize).min(chars.len());
                Ok(Val::Str(if lower == "left" {
                    chars[..count].iter().collect()
                } else {
                    chars[chars.len() - count..].iter().collect()
                }))
            }
            "len" => {
                let args = self.arg_values(args)?;
                Ok(Val::Num(
                    args.first().map_or(0, |v| v.to_text().chars().count()) as f64,
                ))
            }
            "lower" | "upper" => {
                let args = self.arg_values(args)?;
                let Some(first) = args.first() else {
                    return param_error(name);
                };
                if *first == Val::Null {
                    return Ok(Val::Null);
                }
                let text = first.to_text();
                Ok(Val::Str(if lower == "lower" {
                    text.to_lowercase()
                } else {
                    text.to_uppercase()
                }))
            }
            "ltrim" | "rtrim" => {
                let args = self.arg_values(args)?;
                let Some(first) = args.first() else {
                    return param_error(name);
                };
                if *first == Val::Null {
                    return Ok(Val::Null);
                }
                let text = first.to_text();
                Ok(Val::Str(if lower == "ltrim" {
                    text.trim_start().to_string()
                } else {
                    text.trim_end().to_string()
                }))
            }
            "replace" => {
                let args = self.arg_values(args)?;
                if args.len() < 2 {
                    return param_error(name);
                }
                let text = args[0].to_text();
                let find = args[1].to_text();
                let with = args.get(2).map(Val::to_text).unwrap_or_default();
                if find.is_empty() {
                    return Ok(Val::Str(text));
                }
                let out = text.replace(&find, &with);
                self.dom.check_string(&out)?;
                Ok(Val::Str(out))
            }
            "space" => {
                let args = self.arg_values(args)?;
                let count = args.first().map_or(0.0, Val::to_num).max(0.0) as usize;
                if count > self.dom.limits.max_string_bytes {
                    return err("String too long.");
                }
                Ok(Val::Str(" ".repeat(count)))
            }
            "str" => {
                let args = self.arg_values(args)?;
                if args.is_empty() || args.len() > 3 {
                    return param_error(name);
                }
                if args[0] == Val::Null {
                    return Ok(Val::Null);
                }
                let width = args.get(1).map_or(10.0, Val::to_num) as i64;
                let precision = args.get(2).map_or(0.0, Val::to_num).clamp(0.0, 15.0) as usize;
                if width > 10_000 {
                    return err("String too long.");
                }
                Ok(Val::Str(str_function(args[0].to_num(), width, precision)))
            }
            "stuff" => {
                let args = self.arg_values(args)?;
                if args.len() < 3 {
                    return param_error(name);
                }
                if args[0] == Val::Null {
                    return Ok(Val::Null);
                }
                let chars: Vec<char> = args[0].to_text().chars().collect();
                let start = ((args[1].to_num() as i64).max(1) as usize - 1).min(chars.len());
                let count = (args[2].to_num().max(0.0) as usize).min(chars.len() - start);
                let insert = args.get(3).map(Val::to_text).unwrap_or_default();
                let mut out: String = chars[..start].iter().collect();
                out.push_str(&insert);
                out.extend(chars[start + count..].iter());
                self.dom.check_string(&out)?;
                Ok(Val::Str(out))
            }
            "substr" => {
                let args = self.arg_values(args)?;
                if args.len() != 3 {
                    return param_error(name);
                }
                if args[0] == Val::Null {
                    return Ok(Val::Null);
                }
                let chars: Vec<char> = args[0].to_text().chars().collect();
                let start = ((args[1].to_num() as i64).max(1) as usize - 1).min(chars.len());
                let count = (args[2].to_num().max(0.0) as usize).min(chars.len() - start);
                Ok(Val::Str(chars[start..start + count].iter().collect()))
            }
            "uuid" => {
                let args = self.arg_values(args)?;
                let dashed = args.first().is_some_and(|v| v.to_num() != 0.0);
                Ok(Val::Str(pseudo_uuid(self.dom.ops, dashed)))
            }
            "wordnum" => {
                let args = self.arg_values(args)?;
                let Some(first) = args.first() else {
                    return param_error(name);
                };
                if *first == Val::Null {
                    return Ok(Val::Null);
                }
                let style = args.get(1).map_or(0.0, Val::to_num) as i32;
                Ok(Val::Str(word_num(first.to_num(), style)))
            }
            // Date and time.
            "date" => Ok(Val::Num(days_from_date(now_utc().0) as f64)),
            "time" => {
                let (_, (h, m, s, ms)) = now_utc();
                Ok(Val::Num(f64::from(((h * 60 + m) * 60 + s) * 1000 + ms)))
            }
            "date2num" | "isodate2num" => {
                let args = self.arg_values(args)?;
                let Some(first) = args.first() else {
                    return param_error(name);
                };
                if *first == Val::Null {
                    return Ok(Val::Null);
                }
                let text = first.to_text();
                let date = if lower == "isodate2num" {
                    picture::parse_iso_date(&text)
                } else {
                    let (locale, pattern) = self.date_pattern_args(&args, 1)?;
                    let locale = self.dom.locales.get(locale.as_deref());
                    picture::parse_date(&text, &pattern, locale)
                };
                Ok(Val::Num(date.map_or(0, days_from_date) as f64))
            }
            "num2date" => {
                let args = self.arg_values(args)?;
                let Some(first) = args.first() else {
                    return param_error(name);
                };
                if *first == Val::Null {
                    return Ok(Val::Null);
                }
                let days = first.to_num() as i64;
                if days < 1 {
                    return Ok(Val::Null);
                }
                let (y, m, d) = date_from_days(days);
                let (locale, pattern) = self.date_pattern_args(&args, 1)?;
                let iso = format!("{y:04}-{m:02}-{d:02}");
                Ok(Val::Str(picture::format_value_in(
                    &iso,
                    Some(&format!("date{{{pattern}}}")),
                    "date",
                    &self.dom.locales,
                    locale.as_deref(),
                )))
            }
            "datefmt" | "localdatefmt" | "timefmt" | "localtimefmt" => {
                let args = self.arg_values(args)?;
                let style = args.first().map_or(0.0, Val::to_num) as i32;
                let locale = match args.get(1) {
                    Some(v) => Some(v.to_text()),
                    None => self.node_locale(),
                };
                let locale = self.dom.locales.get(locale.as_deref());
                let length = style_length(style);
                Ok(Val::Str(if lower.contains("date") {
                    locale.date_pattern(length).to_string()
                } else {
                    locale.time_pattern(length).to_string()
                }))
            }
            "isotime2num" | "time2num" => {
                let args = self.arg_values(args)?;
                let Some(first) = args.first() else {
                    return param_error(name);
                };
                if *first == Val::Null {
                    return Ok(Val::Null);
                }
                let text = first.to_text();
                let time = if lower == "isotime2num" {
                    picture::parse_iso_time(&text).map(|(h, m, s)| (h, m, s, 0))
                } else {
                    let (locale, pattern) = self.time_pattern_args(&args, 1)?;
                    let locale = self.dom.locales.get(locale.as_deref());
                    picture::parse_time(&text, &pattern, locale)
                };
                Ok(Val::Num(time.map_or(0.0, |(h, m, s, ms)| {
                    f64::from(((h * 60 + m) * 60 + s) * 1000 + ms + 1)
                })))
            }
            "num2time" | "num2gmtime" => {
                let args = self.arg_values(args)?;
                let Some(first) = args.first() else {
                    return param_error(name);
                };
                if *first == Val::Null {
                    return Ok(Val::Null);
                }
                let millis = first.to_num();
                if millis < 1.0 {
                    return Ok(Val::Null);
                }
                let total = (millis as i64 - 1).rem_euclid(86_400_000);
                let (h, m, s) = (
                    total / 3_600_000,
                    (total / 60_000) % 60,
                    (total / 1000) % 60,
                );
                let (locale, pattern) = self.time_pattern_args(&args, 1)?;
                let iso = format!("{h:02}:{m:02}:{s:02}");
                Ok(Val::Str(picture::format_value_in(
                    &iso,
                    Some(&format!("time{{{pattern}}}")),
                    "time",
                    &self.dom.locales,
                    locale.as_deref(),
                )))
            }
            // Financial.
            "apr" | "cterm" | "fv" | "ipmt" | "npv" | "pmt" | "ppmt" | "pv" | "rate" | "term" => {
                let args = self.expanded(args)?;
                if args.contains(&Val::Null) {
                    return Ok(Val::Null);
                }
                let n: Vec<f64> = args.iter().map(Val::to_num).collect();
                financial(&lower, &n).map(Val::Num)
            }
            // URL functions are never allowed to reach the network.
            "get" | "post" | "put" => err("Server does not permit operation."),
            _ => err(format!("Function '{name}' is unknown.")),
        }
    }

    fn node_locale(&self) -> Option<String> {
        self.dom.nodes.get(self.this).and_then(|n| n.locale.clone())
    }

    /// (locale, date pattern) from optional `(format, locale)` arguments.
    fn date_pattern_args(&self, args: &[Val], at: usize) -> SResult<(Option<String>, String)> {
        let locale = args
            .get(at + 1)
            .map(Val::to_text)
            .or_else(|| self.node_locale());
        let pattern = match args.get(at).map(Val::to_text).filter(|p| !p.is_empty()) {
            Some(pattern) => strip_category(&pattern),
            None => self
                .dom
                .locales
                .get(locale.as_deref())
                .date_pattern(super::locale::PatternLength::Med)
                .to_string(),
        };
        Ok((locale, pattern))
    }

    fn time_pattern_args(&self, args: &[Val], at: usize) -> SResult<(Option<String>, String)> {
        let locale = args
            .get(at + 1)
            .map(Val::to_text)
            .or_else(|| self.node_locale());
        let pattern = match args.get(at).map(Val::to_text).filter(|p| !p.is_empty()) {
            Some(pattern) => strip_category(&pattern),
            None => self
                .dom
                .locales
                .get(locale.as_deref())
                .time_pattern(super::locale::PatternLength::Med)
                .to_string(),
        };
        Ok((locale, pattern))
    }
}

/// Whether an initializer yields a node reference a variable should keep:
/// a method call (`resolveNode`, `addInstance`...) or `Ref(...)`.
fn keeps_object(expr: &Expr) -> bool {
    match expr {
        Expr::Path(Head::Call(name, _), segs) if segs.is_empty() => {
            name.eq_ignore_ascii_case("ref")
        }
        Expr::Path(_, segs) => segs.last().is_some_and(|seg| seg.call.is_some()),
        _ => false,
    }
}

fn objs_val(objs: Vec<Obj>) -> Val {
    if objs.len() == 1 {
        return Val::Obj(objs.into_iter().next().expect("one"));
    }
    if objs.is_empty() {
        return Val::Null;
    }
    let mut ids = Vec::new();
    for obj in &objs {
        match obj {
            Obj::Node(id) => ids.push(*id),
            Obj::Nodes(more) => ids.extend(more),
            _ => return Val::Obj(objs.into_iter().next().expect("one")),
        }
    }
    Val::Obj(Obj::Nodes(ids))
}

fn param_error<T>(name: &str) -> SResult<T> {
    Err(ScriptError(format!(
        "Incorrect number of parameters calling method '{name}'."
    )))
}

fn loose_equal(a: &Val, b: &Val) -> bool {
    match (a, b) {
        (Val::Null, Val::Null) => true,
        (Val::Null, _) | (_, Val::Null) => false,
        (Val::Str(x), Val::Str(y)) => x == y,
        _ => a.to_num() == b.to_num(),
    }
}

fn strip_category(pattern: &str) -> String {
    let trimmed = pattern.trim();
    for prefix in ["datetime", "date", "time"] {
        if let Some(rest) = trimmed.strip_prefix(prefix) {
            if let (Some(open), true) = (rest.find('{'), rest.ends_with('}')) {
                return rest[open + 1..rest.len() - 1].to_string();
            }
        }
    }
    trimmed.to_string()
}

fn style_length(style: i32) -> super::locale::PatternLength {
    use super::locale::PatternLength;
    match style {
        1 => PatternLength::Short,
        3 => PatternLength::Long,
        4 => PatternLength::Full,
        _ => PatternLength::Med,
    }
}

/// Half-away-from-zero rounding at `places` decimals, with a small
/// correction for binary representation error (PDFium rounds through
/// `CFGAS_Decimal`).
fn round_half_away(value: f64, places: i32) -> f64 {
    let factor = 10f64.powi(places);
    let scaled = value * factor;
    let corrected = scaled + scaled.signum() * 1e-9 * scaled.abs().max(1.0);
    let rounded = corrected.abs().round() * corrected.signum();
    rounded / factor
}

/// PDFium `Str`: right-justified in `width` with `precision` decimals;
/// stars when it does not fit.
fn str_function(value: f64, width: i64, precision: usize) -> String {
    let width = width.max(0);
    let stars = || "*".repeat(width as usize);
    let text = format!("{:.*}", precision, value);
    let int_len = text.find('.').unwrap_or(text.len()) as i64;
    if int_len > width || precision as i64 + int_len >= width {
        return stars();
    }
    if text.len() as i64 > width {
        return stars();
    }
    format!("{:>width$}", text, width = width as usize)
}

fn unit_of(text: &str) -> &'static str {
    let lower = text.trim().to_ascii_lowercase();
    for (suffix, unit) in [
        ("in", "in"),
        ("inches", "in"),
        ("mm", "mm"),
        ("millimeters", "mm"),
        ("cm", "cm"),
        ("centimeters", "cm"),
        ("pt", "pt"),
        ("points", "pt"),
        ("picas", "pt"),
        ("mp", "mp"),
        ("millipoints", "mp"),
    ] {
        if lower.ends_with(suffix) {
            return unit;
        }
    }
    "in"
}

fn measurement_points(text: &str) -> f64 {
    let value = super::som::parse_leading_number(text);
    match unit_of(text) {
        "mm" => value * 72.0 / 25.4,
        "cm" => value * 72.0 / 2.54,
        "pt" => value,
        "mp" => value / 1000.0,
        _ => value * 72.0,
    }
}

fn points_to(points: f64, unit: &str) -> f64 {
    match unit_of(unit) {
        "mm" => points / 72.0 * 25.4,
        "cm" => points / 72.0 * 2.54,
        "pt" => points,
        "mp" => points * 1000.0,
        _ => points / 72.0,
    }
}

fn decode(text: &str, kind: &str) -> String {
    match kind {
        "url" => {
            let bytes = text.as_bytes();
            let mut out = Vec::new();
            let mut i = 0;
            while i < bytes.len() {
                if bytes[i] == b'%' {
                    if let Some(hex) = text.get(i + 1..i + 3) {
                        if let Ok(byte) = u8::from_str_radix(hex, 16) {
                            out.push(byte);
                            i += 3;
                            continue;
                        }
                    }
                }
                out.push(bytes[i]);
                i += 1;
            }
            String::from_utf8_lossy(&out).into_owned()
        }
        _ => {
            let mut out = String::new();
            let mut rest = text;
            while let Some(amp) = rest.find('&') {
                out.push_str(&rest[..amp]);
                let tail = &rest[amp..];
                let Some(semi) = tail.find(';') else {
                    out.push_str(tail);
                    rest = "";
                    break;
                };
                let entity = &tail[1..semi];
                let decoded = match entity {
                    "amp" => Some('&'),
                    "lt" => Some('<'),
                    "gt" => Some('>'),
                    "quot" => Some('"'),
                    "apos" => Some('\''),
                    "nbsp" => Some('\u{a0}'),
                    e if e.starts_with("#x") || e.starts_with("#X") => {
                        u32::from_str_radix(&e[2..], 16)
                            .ok()
                            .and_then(char::from_u32)
                    }
                    e if e.starts_with('#') => e[1..].parse::<u32>().ok().and_then(char::from_u32),
                    _ => None,
                };
                match decoded {
                    Some(ch) => out.push(ch),
                    None => out.push_str(&tail[..=semi]),
                }
                rest = &tail[semi + 1..];
            }
            out.push_str(rest);
            out
        }
    }
}

fn encode(text: &str, kind: &str) -> String {
    let mut out = String::new();
    match kind {
        "url" => {
            for byte in text.bytes() {
                if byte.is_ascii_alphanumeric() || b"-_.!~*'()".contains(&byte) {
                    out.push(byte as char);
                } else {
                    out.push_str(&format!("%{byte:02x}"));
                }
            }
        }
        _ => {
            for ch in text.chars() {
                match ch {
                    '&' => out.push_str("&amp;"),
                    '<' => out.push_str("&lt;"),
                    '>' => out.push_str("&gt;"),
                    '"' => out.push_str("&quot;"),
                    '\'' if kind == "xml" => out.push_str("&apos;"),
                    c if (c as u32) > 127 => out.push_str(&format!("&#x{:x};", c as u32)),
                    c => out.push(c),
                }
            }
        }
    }
    out
}

fn pseudo_uuid(seed: u64, dashed: bool) -> String {
    // Deterministic per run (no randomness source is exposed to scripts).
    let mut state = seed ^ 0x9E37_79B9_7F4A_7C15;
    let mut hex = String::new();
    for _ in 0..4 {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        hex.push_str(&format!("{:08x}", state as u32));
    }
    if dashed {
        format!(
            "{}-{}-{}-{}-{}",
            &hex[..8],
            &hex[8..12],
            &hex[12..16],
            &hex[16..20],
            &hex[20..32]
        )
    } else {
        hex
    }
}

/// PDFium `TrillionUS`: words for up to 12 digits (quirks included).
fn trillion_us(digits: &[u8]) -> String {
    const UNITS: [&str; 10] = [
        "zero", "one", "two", "three", "four", "five", "six", "seven", "eight", "nine",
    ];
    const CAP_UNITS: [&str; 10] = [
        "Zero", "One", "Two", "Three", "Four", "Five", "Six", "Seven", "Eight", "Nine",
    ];
    const TENS: [&str; 10] = [
        "Ten",
        "Eleven",
        "Twelve",
        "Thirteen",
        "Fourteen",
        "Fifteen",
        "Sixteen",
        "Seventeen",
        "Eighteen",
        "Nineteen",
    ];
    const LAST_TENS: [&str; 8] = [
        "Twenty", "Thirty", "Forty", "Fifty", "Sixty", "Seventy", "Eighty", "Ninety",
    ];
    const COMM: [&str; 5] = [
        " Hundred ",
        " Thousand ",
        " Million ",
        " Billion ",
        "Trillion",
    ];
    let d =
        |i: usize| usize::from(digits.get(i).copied().unwrap_or(b'0').saturating_sub(b'0')).min(9);
    let len = digits.len();
    let mut comm: i32 = if len > 12 {
        4
    } else if len > 9 {
        3
    } else if len > 6 {
        2
    } else if len > 3 {
        1
    } else {
        0
    };
    let mut first = len % 3;
    if first == 0 {
        first = 3;
    }
    let mut out = String::new();
    let tens_units = |out: &mut String, tens: usize, units: usize| {
        if tens == 0 {
            out.push_str(CAP_UNITS[units]);
        } else if tens > 1 {
            out.push_str(LAST_TENS[tens - 2]);
            out.push('-');
            out.push_str(UNITS[units]);
        } else {
            out.push_str(TENS[units]);
        }
    };
    let mut index = 0;
    match first {
        3 => {
            if d(0) != 0 {
                out.push_str(CAP_UNITS[d(0)]);
                out.push_str(COMM[0]);
            }
            tens_units(&mut out, d(1), d(2));
            index += 3;
        }
        2 => {
            tens_units(&mut out, d(0), d(1));
            index += 2;
        }
        _ => {
            out.push_str(CAP_UNITS[d(0)]);
            index += 1;
        }
    }
    if len > 3 {
        out.push_str(COMM[comm.clamp(0, 4) as usize]);
        comm -= 1;
    }
    while index < len {
        if d(index) != 0 {
            out.push_str(CAP_UNITS[d(index)]);
            out.push_str(COMM[0]);
        }
        tens_units(&mut out, d(index + 1), d(index + 2));
        if index + 3 < len {
            out.push_str(COMM[comm.clamp(0, 4) as usize]);
            comm -= 1;
        }
        index += 3;
    }
    out
}

/// PDFium `WordUS` over the `%.2f` text of the number.
fn word_num(value: f64, style: i32) -> String {
    if value.is_nan() || !(0.0..=922_337_203_685_477_550.0).contains(&value) {
        return "*".to_string();
    }
    if !(0..=2).contains(&style) {
        return String::new();
    }
    let text = format!("{:.2}", value as f32);
    let (integer, fraction) = text.split_once('.').unwrap_or((&text, ""));
    let words = |digits: &str| {
        let bytes = digits.as_bytes();
        let mut out = String::new();
        let mut index = 0;
        while index < bytes.len() {
            let mut count = (bytes.len() - index) % 12;
            if count == 0 {
                count = 12;
            }
            out.push_str(&trillion_us(&bytes[index..index + count]));
            index += count;
            if index < bytes.len() {
                out.push_str(" Trillion ");
            }
        }
        out
    };
    let mut out = words(integer);
    if style > 0 {
        out.push_str(" Dollars");
    }
    if style > 1 && !fraction.is_empty() {
        out.push_str(" And ");
        out.push_str(&words(fraction));
        out.push_str(" Cents");
    }
    out
}

fn financial(name: &str, n: &[f64]) -> SResult<f64> {
    let mismatch = || err::<f64>("Argument mismatch in property or function argument.");
    let periods = |v: f64| -> i64 { v as i64 };
    match name {
        "apr" => {
            if n.len() != 3 {
                return param_error(name);
            }
            let (principal, payment, periods) = (n[0], n[1], periods(n[2]));
            if principal <= 0.0 || payment <= 0.0 || periods <= 0 || periods > 100_000 {
                return mismatch();
            }
            let p = periods as f64;
            let mut r = 2.0 * (p * payment - principal) / (p * principal);
            let power = |r: f64| (1.0 + r).powf(p);
            let mut temp = power(r);
            let mut ret = r * temp / (temp - 1.0) - payment / principal;
            let mut guard = 0;
            while ret.abs() > 0.000_000_1 && guard < 1000 {
                guard += 1;
                let derivative = ((temp + r * p * (temp / (1.0 + r))) * (temp - 1.0)
                    - (r * temp * p * (temp / (1.0 + r))))
                    / ((temp - 1.0) * (temp - 1.0));
                if derivative == 0.0 || !derivative.is_finite() {
                    return mismatch();
                }
                r -= ret / derivative;
                temp = power(r);
                ret = r * temp / (temp - 1.0) - payment / principal;
            }
            Ok(r * 12.0)
        }
        "cterm" => {
            if n.len() != 3 {
                return param_error(name);
            }
            let (rate, future, initial) = (n[0], n[1], n[2]);
            if rate <= 0.0 || future <= 0.0 || initial <= 0.0 {
                return mismatch();
            }
            Ok((future / initial).ln() / (1.0 + rate).ln())
        }
        "fv" => {
            if n.len() != 3 {
                return param_error(name);
            }
            let (amount, rate, periods) = (n[0], n[1], periods(n[2]));
            if amount <= 0.0 || rate < 0.0 || periods <= 0 {
                return mismatch();
            }
            Ok(if rate != 0.0 {
                amount * ((1.0 + rate).powf(periods as f64) - 1.0) / rate
            } else {
                amount * periods as f64
            })
        }
        "ipmt" | "ppmt" => {
            if n.len() != 5 {
                return param_error(name);
            }
            let (mut principal, rate, payment, first, months) = (n[0], n[1], n[2], n[3], n[4]);
            if principal <= 0.0 || rate <= 0.0 || payment <= 0.0 || first < 0.0 || months < 0.0 {
                return mismatch();
            }
            let monthly = rate / 12.0;
            if payment < principal * monthly {
                return if name == "ipmt" { Ok(0.0) } else { mismatch() };
            }
            let nums = (((payment / principal).log10() - (payment / principal - monthly).log10())
                / (1.0 + monthly).log10()) as i64;
            let end = ((first + months - 1.0) as i64).min(nums).min(100_000);
            let mut i = 0i64;
            while (i as f64) < first - 1.0 && i < 100_000 {
                principal -= payment - principal * monthly;
                i += 1;
            }
            let mut sum = 0.0;
            while i < end {
                if name == "ipmt" {
                    sum += principal * monthly;
                    principal -= payment - principal * monthly;
                } else {
                    let part = payment - principal * monthly;
                    sum += part;
                    principal -= part;
                }
                i += 1;
            }
            Ok(sum)
        }
        "npv" => {
            if n.len() < 2 {
                return param_error(name);
            }
            let rate = n[0];
            if rate <= 0.0 {
                return mismatch();
            }
            let mut sum = 0.0;
            for value in n[1..].iter().rev() {
                sum += value;
                sum /= 1.0 + rate;
            }
            Ok(sum)
        }
        "pmt" => {
            if n.len() != 3 {
                return param_error(name);
            }
            let (principal, rate, periods) = (n[0], n[1], periods(n[2]));
            if principal <= 0.0 || rate <= 0.0 || periods <= 0 {
                return mismatch();
            }
            let sum = (1.0 + rate).powf(periods as f64);
            Ok(principal * rate * sum / (sum - 1.0))
        }
        "pv" => {
            if n.len() != 3 {
                return param_error(name);
            }
            let (amount, rate, periods) = (n[0], n[1], periods(n[2]));
            if amount <= 0.0 || rate < 0.0 || periods <= 0 {
                return mismatch();
            }
            if rate == 0.0 {
                return Ok(amount * periods as f64);
            }
            let temp = 1.0 / (1.0 + rate).powf(periods as f64);
            Ok(amount * ((1.0 - temp) / rate))
        }
        "rate" => {
            if n.len() != 3 {
                return param_error(name);
            }
            let (future, present, periods) = (n[0], n[1], periods(n[2]));
            if future <= 0.0 || present <= 0.0 || periods <= 0 {
                return mismatch();
            }
            Ok((future / present).powf(1.0 / periods as f64) - 1.0)
        }
        _ => {
            if n.len() != 3 {
                return param_error(name);
            }
            let (payment, rate, future) = (n[0], n[1], n[2]);
            if payment <= 0.0 || rate <= 0.0 || future <= 0.0 {
                return mismatch();
            }
            Ok((future / payment * rate + 1.0).ln() / (1.0 + rate).ln())
        }
    }
}

/// Days since 1899-12-31 (1900-01-01 is day 1), FormCalc's date number.
pub(crate) fn days_from_date((year, month, day): (i32, u32, u32)) -> i64 {
    days_from_civil(year, month, day) - days_from_civil(1899, 12, 31)
}

fn date_from_days(days: i64) -> (i32, u32, u32) {
    civil_from_days(days + days_from_civil(1899, 12, 31))
}

/// Howard Hinnant's days-from-civil (days since 1970-01-01).
fn days_from_civil(year: i32, month: u32, day: u32) -> i64 {
    let y = i64::from(year) - i64::from(month <= 2);
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let m = i64::from(month);
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + i64::from(day) - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

fn civil_from_days(days: i64) -> (i32, u32, u32) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    ((y + i64::from(m <= 2)) as i32, m, d)
}

/// Current UTC date and time.
fn now_utc() -> ((i32, u32, u32), (u32, u32, u32, u32)) {
    #[cfg(not(target_arch = "wasm32"))]
    let millis = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as i64);
    #[cfg(target_arch = "wasm32")]
    let millis = js_sys::Date::now() as i64;
    let days = millis.div_euclid(86_400_000);
    let rest = millis.rem_euclid(86_400_000);
    let date = civil_from_days(days);
    let h = (rest / 3_600_000) as u32;
    let m = ((rest / 60_000) % 60) as u32;
    let s = ((rest / 1000) % 60) as u32;
    let ms = (rest % 1000) as u32;
    (date, (h, m, s, ms))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::xfa::locale::LocaleSet;
    use crate::xfa::som::{Class, Limits, SNode};

    fn dom() -> Dom {
        let mut dom = Dom::new(Limits::default(), LocaleSet::default());
        let root = dom.add(SNode::new(Class::Subform, "form1", 0), None);
        dom.root = root;
        for (name, value) in [("a", "2"), ("b", "3.5"), ("label", "")] {
            let mut field = SNode::new(Class::Field, name, 1);
            field.value_kind = if name == "label" { "text" } else { "decimal" }.into();
            field.value = Some(value.into()).filter(|v: &String| !v.is_empty());
            field.has_caption = true;
            field.caption = Some("Label".into());
            dom.add(field, Some(root));
        }
        for value in ["1", "2", "3"] {
            let row = dom.add(SNode::new(Class::Subform, "row", 2), Some(root));
            let mut amt = SNode::new(Class::Field, "amt", 3);
            amt.value_kind = "decimal".into();
            amt.value = Some(value.into());
            dom.add(amt, Some(row));
        }
        dom
    }

    fn eval(dom: &mut Dom, source: &str) -> Val {
        let this = dom.nodes.iter().position(|n| n.name == "label").unwrap();
        let value = run(dom, this, source).unwrap();
        match value {
            Val::Obj(obj) => dom.value_of(&obj).unwrap(),
            other => other,
        }
    }

    #[test]
    fn arithmetic_logic_and_null_semantics() {
        let mut d = dom();
        assert_eq!(eval(&mut d, "1 + 2 * 3"), Val::Num(7.0));
        assert_eq!(eval(&mut d, "(1 + 2) * 3 ; comment"), Val::Num(9.0));
        assert_eq!(eval(&mut d, "a * b"), Val::Num(7.0));
        assert_eq!(eval(&mut d, "null + null"), Val::Null);
        assert_eq!(eval(&mut d, "null + 5"), Val::Num(5.0));
        assert_eq!(eval(&mut d, "null == null"), Val::Num(1.0));
        assert_eq!(eval(&mut d, "\"abc\" < \"abd\""), Val::Num(1.0));
        assert_eq!(
            eval(&mut d, "3 > 2 and 2 > 1 & not (1 == 2)"),
            Val::Num(1.0)
        );
        assert_eq!(eval(&mut d, "1 eq 1 or 0"), Val::Num(1.0));
        assert_eq!(eval(&mut d, "a <> 2"), Val::Num(0.0));
        assert!(run(&mut d, 0, "1 / 0").is_err());
    }

    #[test]
    fn control_flow_and_functions() {
        let mut d = dom();
        let source = r#"
            func fact(n) do
                if (n <= 1) then 1 else n * fact(n - 1) endif
            endfunc
            var total = 0
            for i = 1 upto 5 step 2 do total = total + i endfor
            var j = 10
            while (j > 7) do j = j - 1 endwhile
            foreach v in (1, 2, row[*].amt) do total = total + v endfor
            if (total > 100) then "big" elseif (total > 10) then Concat("mid:", total, ":", fact(5), ":", j) else "small" endif
        "#;
        assert_eq!(eval(&mut d, source), Val::Str("mid:18:120:7".into()));
        assert_eq!(
            eval(
                &mut d,
                "var x = 0 for i = 10 downto 1 do if (i == 3) then break endif x = x + 1 endfor x"
            ),
            Val::Num(7.0)
        );
    }

    #[test]
    fn som_accessors_assignments_and_aggregates() {
        let mut d = dom();
        assert_eq!(eval(&mut d, "Sum(row[*].amt)"), Val::Num(6.0));
        assert_eq!(eval(&mut d, "Avg(row[*].amt, a)"), Val::Num(2.0));
        assert_eq!(eval(&mut d, "Count(row[*].amt)"), Val::Num(3.0));
        assert_eq!(eval(&mut d, "Max(row[*].amt, 2.5)"), Val::Num(3.0));
        assert_eq!(eval(&mut d, "Min(form1..amt[*])"), Val::Num(1.0));
        assert_eq!(eval(&mut d, "row[1].amt"), Val::Num(2.0));
        assert_eq!(
            eval(&mut d, "$form.form1.row[2].amt.rawValue"),
            Val::Num(3.0)
        );
        eval(&mut d, "label = Concat(\"Total \", Sum(row[*].amt))");
        let label = d.nodes.iter().position(|n| n.name == "label").unwrap();
        assert_eq!(d.nodes[label].value.as_deref(), Some("Total 6"));
        eval(&mut d, "$.presence = \"hidden\"");
        assert_eq!(d.nodes[label].presence, "hidden");
        eval(&mut d, "label.caption.value.#text = \"New caption\"");
        assert_eq!(d.nodes[label].caption.as_deref(), Some("New caption"));
        eval(&mut d, "a.rawValue = 10");
        assert_eq!(eval(&mut d, "a + b"), Val::Num(13.5));
        eval(&mut d, "_row.setInstances(1)");
        assert_eq!(eval(&mut d, "_row.count"), Val::Num(1.0));
        assert_eq!(
            eval(&mut d, "xfa.resolveNode(\"form1.a\").rawValue"),
            Val::Num(10.0)
        );
        assert!(run(&mut d, 0, "Get(\"http://example.com\")").is_err());
    }

    #[test]
    fn string_date_and_misc_functions() {
        let mut d = dom();
        assert_eq!(eval(&mut d, "At(\"abcd\", \"cd\")"), Val::Num(3.0));
        assert_eq!(eval(&mut d, "Left(\"abcd\", 2)"), Val::Str("ab".into()));
        assert_eq!(eval(&mut d, "Right(\"abcd\", 3)"), Val::Str("bcd".into()));
        assert_eq!(
            eval(&mut d, "Substr(\"abcdef\", 2, 3)"),
            Val::Str("bcd".into())
        );
        assert_eq!(
            eval(&mut d, "Stuff(\"abcdef\", 2, 3, \"X\")"),
            Val::Str("aXef".into())
        );
        assert_eq!(
            eval(&mut d, "Replace(\"a-b-c\", \"-\", \"+\")"),
            Val::Str("a+b+c".into())
        );
        assert_eq!(eval(&mut d, "Upper(\"abc\") + 0"), Val::Num(0.0));
        assert_eq!(eval(&mut d, "Len(\"h\u{e9}llo\")"), Val::Num(5.0));
        assert_eq!(
            eval(&mut d, "Str(3.456, 8, 2)"),
            Val::Str("    3.46".into())
        );
        assert_eq!(eval(&mut d, "Round(2.345, 2)"), Val::Num(2.35));
        assert_eq!(eval(&mut d, "Mod(-7, 3)"), Val::Num(-1.0));
        assert_eq!(
            eval(&mut d, "Abs(-2) + Ceil(1.2) + Floor(1.8)"),
            Val::Num(5.0)
        );
        assert_eq!(
            eval(&mut d, "Date2Num(\"Mar 15, 1996\")"),
            Val::Num(35138.0)
        );
        assert_eq!(eval(&mut d, "IsoDate2Num(\"1900-01-01\")"), Val::Num(1.0));
        assert_eq!(
            eval(&mut d, "Num2Date(35138, \"DD/MM/YYYY\")"),
            Val::Str("15/03/1996".into())
        );
        assert_eq!(
            eval(&mut d, "Num2Date(1, \"DD. MMMM YYYY\", \"de_DE\")"),
            Val::Str("01. Januar 1900".into())
        );
        assert_eq!(
            eval(&mut d, "Time2Num(\"00:00:01\", \"HH:MM:SS\")"),
            Val::Num(1001.0)
        );
        assert_eq!(
            eval(&mut d, "Num2Time(1001, \"HH:MM:SS\")"),
            Val::Str("00:00:01".into())
        );
        assert_eq!(
            eval(&mut d, "LocalDateFmt(1, \"de_DE\")"),
            Val::Str("DD.MM.YY".into())
        );
        assert_eq!(
            eval(&mut d, "Format(\"num{$z,zz9.99}\", 1234.5)"),
            Val::Str("$1,234.50".into())
        );
        assert_eq!(
            eval(&mut d, "Parse(\"num{$z,zz9.99}\", \"$1,234.50\")"),
            Val::Num(1234.5)
        );
        assert_eq!(
            eval(&mut d, "Choose(2, \"x\", \"y\", \"z\")"),
            Val::Str("y".into())
        );
        assert_eq!(eval(&mut d, "Oneof(3, 1, 2, 3)"), Val::Num(1.0));
        assert_eq!(eval(&mut d, "Within(\"c\", \"a\", \"d\")"), Val::Num(1.0));
        assert_eq!(eval(&mut d, "HasValue(\"  \")"), Val::Num(0.0));
        assert_eq!(eval(&mut d, "Exists(nothere)"), Val::Num(0.0));
        assert_eq!(eval(&mut d, "Exists(a)"), Val::Num(1.0));
        assert_eq!(eval(&mut d, "Eval(\"1 + 1\")"), Val::Num(2.0));
        assert_eq!(eval(&mut d, "UnitValue(\"1in\", \"pt\")"), Val::Num(72.0));
        assert_eq!(eval(&mut d, "UnitType(\"10mm\")"), Val::Str("mm".into()));
        assert_eq!(
            eval(&mut d, "WordNum(123.45, 2)"),
            Val::Str("One Hundred Twenty-three Dollars And Forty-five Cents".into())
        );
        assert_eq!(
            eval(&mut d, "Encode(\"a b&\", \"url\")"),
            Val::Str("a%20b%26".into())
        );
        assert_eq!(
            eval(&mut d, "Decode(\"&lt;a&gt;\", \"html\")"),
            Val::Str("<a>".into())
        );
        match eval(&mut d, "Pmt(30000, 0.085 / 12, 12)") {
            Val::Num(n) => assert!((n - 2616.6).abs() < 0.1, "{n}"),
            other => panic!("{other:?}"),
        }
        match eval(&mut d, "FV(100, 0.075 / 12, 10 * 12)") {
            Val::Num(n) => assert!((n - 17793.03).abs() < 0.1, "{n}"),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn budget_stops_runaway_scripts() {
        let mut d = dom();
        d.limits.max_ops = 10_000;
        assert!(run(&mut d, 0, "while (1) do endwhile").is_err());
        let mut d = dom();
        assert!(run(&mut d, 0, "func f(n) do f(n + 1) endfunc f(1)").is_err());
    }
}
