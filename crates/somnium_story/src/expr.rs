//! The condition and effect language every story asset shares.
//!
//! One line of text, because the people who write it are writers and the place they write it is a field on a graph
//! node or a JSON string an agent produced:
//!
//! ```text
//! met.mireille and mireille.affection >= 3 and not quest('varga_job') == 'failed'
//! lang.patter >= 'conversant' or 'frost_step' in caelan.weaves
//! mireille.affection += 2; met.mireille = true; caelan.weaves add 'frost_step'; start_quest('varga_job')
//! ```
//!
//! Text, rather than nested JSON, for the same reason the engine's rich text is BBCode: the source is what a person
//! reads in a review, and `{"all":[{"fact":"met.mireille","eq":true},...]}` is not something anyone reviews.
//!
//! # Checked against the schema, once
//!
//! [`Condition::compile`] and [`Effects::compile`] resolve every fact against the game's [`Schema`] and type-check
//! the whole expression: comparing a number with a word, testing membership in something that is not a set, or
//! naming a variant an enum does not have is refused at load with a byte offset. What is left at run time cannot
//! fail; it only reads values. An unset fact reads as its declared default, so a condition never has to ask whether
//! a fact "exists".
//!
//! # Vocabulary
//!
//! | Form | Meaning |
//! |---|---|
//! | `true`, `false`, `12`, `'text'` or `"text"` | literals (whole numbers only) |
//! | `fact.id` | a declared fact |
//! | `not a`, `a and b`, `a or b` (also `!`, `&&`, `\|\|`) | logic |
//! | `==`, `!=`, `<`, `<=`, `>`, `>=` | numbers; enums by declared order; `==` and `!=` for anything of one type |
//! | `'x' in set` | set membership |
//! | `a + b`, `a - b`, `-a` | whole-number arithmetic, saturating |
//! | `count(set)`, `min(a, b)`, `max(a, b)`, `abs(a)` | helpers |
//! | `visited('node')`, `visited('dialogue/node')` | times a dialogue node was entered (options: chosen) |
//! | `quest('id')` | `'inactive'`, `'active'`, `'completed'` or `'failed'` |
//! | `stage('id')` | a quest's current stage id, `''` when not active |
//! | `done('quest', 'objective')`, `failed('quest', 'objective')` | objective state |
//!
//! Effects are `;`- or line-separated: `fact = expr`, `fact += expr`, `fact -= expr`, `set add expr`,
//! `set remove expr`, `clear fact` (back to its default), and commands: the story's own (`start_quest('id')`, ...)
//! and the ones a game declares in its manifest (`give_item('lantern', 1)`), which reach the game as events.

use crate::value::{FactDef, FactType, Schema, Value};
use serde::{Deserialize, Serialize};
use std::fmt;
use std::sync::Arc;

/// A parse or type error, with the byte offset it was found at.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExprError {
    pub at: usize,
    pub message: String,
}

impl ExprError {
    fn new(at: usize, message: impl Into<String>) -> Self {
        Self {
            at,
            message: message.into(),
        }
    }
}

impl fmt::Display for ExprError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "at {}: {}", self.at, self.message)
    }
}

impl std::error::Error for ExprError {}

// ---------------------------------------------------------------------------------------------------------------
// Lexer

#[derive(Clone, Debug, PartialEq)]
enum Tok {
    Ident(String),
    Int(i64),
    Str(String),
    LParen,
    RParen,
    Comma,
    Sep,
    EqEq,
    NotEq,
    Lt,
    Le,
    Gt,
    Ge,
    Assign,
    PlusAssign,
    MinusAssign,
    Plus,
    Minus,
    Not,
    And,
    Or,
    True,
    False,
    In,
    Add,
    Remove,
    Clear,
    End,
}

fn describe(tok: &Tok) -> String {
    match tok {
        Tok::Ident(name) => format!("`{name}`"),
        Tok::Int(value) => format!("`{value}`"),
        Tok::Str(text) => format!("'{text}'"),
        Tok::End => "the end".into(),
        Tok::Sep => "`;`".into(),
        other => format!("{other:?}").to_lowercase(),
    }
}

fn lex(src: &str, newline_separates: bool) -> Result<Vec<(Tok, usize)>, ExprError> {
    let bytes = src.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        let c = bytes[i];
        let start = i;
        match c {
            b'\n' if newline_separates => {
                out.push((Tok::Sep, start));
                i += 1;
            }
            b' ' | b'\t' | b'\r' | b'\n' => i += 1,
            b'#' => {
                // A comment runs to the end of the line, so an effect list can say why.
                while i < bytes.len() && bytes[i] != b'\n' {
                    i += 1;
                }
            }
            b'(' => {
                out.push((Tok::LParen, start));
                i += 1;
            }
            b')' => {
                out.push((Tok::RParen, start));
                i += 1;
            }
            b',' => {
                out.push((Tok::Comma, start));
                i += 1;
            }
            b';' => {
                out.push((Tok::Sep, start));
                i += 1;
            }
            b'=' | b'!' | b'<' | b'>' | b'+' | b'-' | b'&' | b'|' => {
                let next = bytes.get(i + 1).copied();
                let (tok, len) = match (c, next) {
                    (b'=', Some(b'=')) => (Tok::EqEq, 2),
                    (b'=', _) => (Tok::Assign, 1),
                    (b'!', Some(b'=')) => (Tok::NotEq, 2),
                    (b'!', _) => (Tok::Not, 1),
                    (b'<', Some(b'=')) => (Tok::Le, 2),
                    (b'<', _) => (Tok::Lt, 1),
                    (b'>', Some(b'=')) => (Tok::Ge, 2),
                    (b'>', _) => (Tok::Gt, 1),
                    (b'+', Some(b'=')) => (Tok::PlusAssign, 2),
                    (b'+', _) => (Tok::Plus, 1),
                    (b'-', Some(b'=')) => (Tok::MinusAssign, 2),
                    (b'-', _) => (Tok::Minus, 1),
                    (b'&', Some(b'&')) => (Tok::And, 2),
                    (b'|', Some(b'|')) => (Tok::Or, 2),
                    _ => {
                        return Err(ExprError::new(
                            start,
                            format!(
                                "`{}` on its own means nothing; use `and` or `or`",
                                c as char
                            ),
                        ));
                    }
                };
                out.push((tok, start));
                i += len;
            }
            b'\'' | b'"' => {
                let quote = c;
                i += 1;
                let mut text = String::new();
                let mut closed = false;
                while i < bytes.len() {
                    let ch = src[i..]
                        .chars()
                        .next()
                        .expect("index is on a char boundary");
                    i += ch.len_utf8();
                    if ch as u32 == u32::from(quote) {
                        closed = true;
                        break;
                    }
                    if ch == '\\' {
                        let Some(escaped) = src[i..].chars().next() else {
                            break;
                        };
                        i += escaped.len_utf8();
                        text.push(match escaped {
                            'n' => '\n',
                            't' => '\t',
                            other => other,
                        });
                    } else {
                        text.push(ch);
                    }
                }
                if !closed {
                    return Err(ExprError::new(start, "text is never closed"));
                }
                out.push((Tok::Str(text), start));
            }
            b'0'..=b'9' => {
                while i < bytes.len() && bytes[i].is_ascii_digit() {
                    i += 1;
                }
                if i < bytes.len() && (bytes[i].is_ascii_alphabetic() || bytes[i] == b'.') {
                    return Err(ExprError::new(
                        start,
                        "numbers are whole and ids start with a letter",
                    ));
                }
                let value = src[start..i]
                    .parse::<i64>()
                    .map_err(|_| ExprError::new(start, "number is too large"))?;
                out.push((Tok::Int(value), start));
            }
            c if c.is_ascii_alphabetic() || c == b'_' => {
                while i < bytes.len()
                    && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'_' || bytes[i] == b'.')
                {
                    i += 1;
                }
                let word = &src[start..i];
                if word.ends_with('.') || word.contains("..") {
                    return Err(ExprError::new(start, format!("`{word}` is not a valid id")));
                }
                out.push((
                    match word {
                        "and" => Tok::And,
                        "or" => Tok::Or,
                        "not" => Tok::Not,
                        "true" => Tok::True,
                        "false" => Tok::False,
                        "in" => Tok::In,
                        "add" => Tok::Add,
                        "remove" => Tok::Remove,
                        "clear" => Tok::Clear,
                        _ => Tok::Ident(word.to_owned()),
                    },
                    start,
                ));
            }
            _ => {
                let ch = src[i..].chars().next().unwrap_or('?');
                return Err(ExprError::new(start, format!("unexpected `{ch}`")));
            }
        }
    }
    out.push((Tok::End, src.len()));
    Ok(out)
}

// ---------------------------------------------------------------------------------------------------------------
// Parse tree

#[derive(Clone, Debug)]
enum Raw {
    Lit(Value),
    Ident(String),
    Unary(UnaryOp, Box<(Raw, usize)>),
    Binary(BinaryOp, Box<(Raw, usize)>, Box<(Raw, usize)>),
    Call(String, Vec<(Raw, usize)>),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum UnaryOp {
    Not,
    Neg,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum BinaryOp {
    And,
    Or,
    Cmp(CmpOp),
    In,
    Plus,
    Minus,
}

/// A comparison operator.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CmpOp {
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
}

impl CmpOp {
    fn holds(self, ordering: std::cmp::Ordering) -> bool {
        use std::cmp::Ordering::*;
        match self {
            CmpOp::Eq => ordering == Equal,
            CmpOp::Ne => ordering != Equal,
            CmpOp::Lt => ordering == Less,
            CmpOp::Le => ordering != Greater,
            CmpOp::Gt => ordering == Greater,
            CmpOp::Ge => ordering != Less,
        }
    }
}

struct Parser {
    toks: Vec<(Tok, usize)>,
    pos: usize,
}

impl Parser {
    fn peek(&self) -> &Tok {
        &self.toks[self.pos].0
    }

    fn at(&self) -> usize {
        self.toks[self.pos].1
    }

    fn bump(&mut self) -> (Tok, usize) {
        let tok = self.toks[self.pos].clone();
        if self.pos + 1 < self.toks.len() {
            self.pos += 1;
        }
        tok
    }

    fn expect(&mut self, tok: Tok, what: &str) -> Result<usize, ExprError> {
        if *self.peek() == tok {
            Ok(self.bump().1)
        } else {
            Err(ExprError::new(
                self.at(),
                format!("expected {what}, found {}", describe(self.peek())),
            ))
        }
    }

    fn expr(&mut self) -> Result<(Raw, usize), ExprError> {
        self.or()
    }

    fn or(&mut self) -> Result<(Raw, usize), ExprError> {
        let mut left = self.and()?;
        while *self.peek() == Tok::Or {
            let at = self.bump().1;
            let right = self.and()?;
            left = (
                Raw::Binary(BinaryOp::Or, Box::new(left), Box::new(right)),
                at,
            );
        }
        Ok(left)
    }

    fn and(&mut self) -> Result<(Raw, usize), ExprError> {
        let mut left = self.not()?;
        while *self.peek() == Tok::And {
            let at = self.bump().1;
            let right = self.not()?;
            left = (
                Raw::Binary(BinaryOp::And, Box::new(left), Box::new(right)),
                at,
            );
        }
        Ok(left)
    }

    fn not(&mut self) -> Result<(Raw, usize), ExprError> {
        if *self.peek() == Tok::Not {
            let at = self.bump().1;
            let inner = self.not()?;
            return Ok((Raw::Unary(UnaryOp::Not, Box::new(inner)), at));
        }
        self.compare()
    }

    fn compare(&mut self) -> Result<(Raw, usize), ExprError> {
        let left = self.sum()?;
        let op = match self.peek() {
            Tok::EqEq => BinaryOp::Cmp(CmpOp::Eq),
            Tok::NotEq => BinaryOp::Cmp(CmpOp::Ne),
            Tok::Lt => BinaryOp::Cmp(CmpOp::Lt),
            Tok::Le => BinaryOp::Cmp(CmpOp::Le),
            Tok::Gt => BinaryOp::Cmp(CmpOp::Gt),
            Tok::Ge => BinaryOp::Cmp(CmpOp::Ge),
            Tok::In => BinaryOp::In,
            Tok::Assign => {
                return Err(ExprError::new(
                    self.at(),
                    "`=` sets a fact; compare with `==`",
                ));
            }
            _ => return Ok(left),
        };
        let at = self.bump().1;
        let right = self.sum()?;
        if matches!(
            self.peek(),
            Tok::EqEq | Tok::NotEq | Tok::Lt | Tok::Le | Tok::Gt | Tok::Ge | Tok::In
        ) {
            return Err(ExprError::new(
                self.at(),
                "comparisons do not chain; join them with `and`",
            ));
        }
        Ok((Raw::Binary(op, Box::new(left), Box::new(right)), at))
    }

    fn sum(&mut self) -> Result<(Raw, usize), ExprError> {
        let mut left = self.unary()?;
        loop {
            let op = match self.peek() {
                Tok::Plus => BinaryOp::Plus,
                Tok::Minus => BinaryOp::Minus,
                _ => return Ok(left),
            };
            let at = self.bump().1;
            let right = self.unary()?;
            left = (Raw::Binary(op, Box::new(left), Box::new(right)), at);
        }
    }

    fn unary(&mut self) -> Result<(Raw, usize), ExprError> {
        if *self.peek() == Tok::Minus {
            let at = self.bump().1;
            let inner = self.unary()?;
            return Ok((Raw::Unary(UnaryOp::Neg, Box::new(inner)), at));
        }
        self.atom()
    }

    fn atom(&mut self) -> Result<(Raw, usize), ExprError> {
        let (tok, at) = self.bump();
        Ok(match tok {
            Tok::Int(value) => (Raw::Lit(Value::Int(value)), at),
            Tok::Str(text) => (Raw::Lit(Value::Text(text)), at),
            Tok::True => (Raw::Lit(Value::Bool(true)), at),
            Tok::False => (Raw::Lit(Value::Bool(false)), at),
            Tok::LParen => {
                let inner = self.expr()?;
                self.expect(Tok::RParen, "`)`")?;
                inner
            }
            Tok::Ident(name) => {
                if *self.peek() == Tok::LParen {
                    self.bump();
                    let args = self.args()?;
                    (Raw::Call(name, args), at)
                } else {
                    (Raw::Ident(name), at)
                }
            }
            other => {
                return Err(ExprError::new(
                    at,
                    format!("expected a value, found {}", describe(&other)),
                ));
            }
        })
    }

    /// Arguments after an opening `(`, through the closing `)`.
    fn args(&mut self) -> Result<Vec<(Raw, usize)>, ExprError> {
        let mut args = Vec::new();
        if *self.peek() == Tok::RParen {
            self.bump();
            return Ok(args);
        }
        loop {
            args.push(self.expr()?);
            match self.peek() {
                Tok::Comma => {
                    self.bump();
                }
                Tok::RParen => {
                    self.bump();
                    return Ok(args);
                }
                other => {
                    return Err(ExprError::new(
                        self.at(),
                        format!("expected `,` or `)`, found {}", describe(other)),
                    ));
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------------------------------------------
// Types and the checked tree

/// What an expression produces.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Ty {
    Bool,
    Int,
    Text,
    /// An enum fact's variants, in order.
    Enum(Arc<[String]>),
    /// A set's allowed members; empty is "any name".
    Set(Arc<[String]>),
}

impl Ty {
    fn name(&self) -> &'static str {
        match self {
            Ty::Bool => "a yes/no value",
            Ty::Int => "a number",
            Ty::Text => "text",
            Ty::Enum(_) => "an enum",
            Ty::Set(_) => "a set",
        }
    }

    fn of(def: &FactDef) -> Self {
        match def.ty {
            FactType::Bool => Ty::Bool,
            FactType::Int => Ty::Int,
            FactType::Text => Ty::Text,
            FactType::Enum => Ty::Enum(def.values.clone().into()),
            FactType::Set => Ty::Set(def.values.clone().into()),
        }
    }
}

/// The story functions a condition may call, answered by the [`Env`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Func {
    Visited,
    Quest,
    Stage,
    Done,
    Failed,
}

impl Func {
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Func::Visited => "visited",
            Func::Quest => "quest",
            Func::Stage => "stage",
            Func::Done => "done",
            Func::Failed => "failed",
        }
    }
}

/// The values `quest('id')` returns, in order.
pub const QUEST_STATES: [&str; 4] = ["inactive", "active", "completed", "failed"];

/// A checked expression. Building one is only possible through compilation, so evaluating one cannot fail.
#[derive(Clone, Debug, PartialEq)]
pub enum Expr {
    Lit(Value),
    Fact(String),
    Not(Box<Expr>),
    And(Box<Expr>, Box<Expr>),
    Or(Box<Expr>, Box<Expr>),
    Compare {
        op: CmpOp,
        left: Box<Expr>,
        right: Box<Expr>,
        /// Enum variants when this compares by declared order.
        order: Option<Arc<[String]>>,
    },
    In {
        member: Box<Expr>,
        set: Box<Expr>,
    },
    Add(Box<Expr>, Box<Expr>),
    Sub(Box<Expr>, Box<Expr>),
    Neg(Box<Expr>),
    Count(Box<Expr>),
    Min(Box<Expr>, Box<Expr>),
    Max(Box<Expr>, Box<Expr>),
    Abs(Box<Expr>),
    Call(Func, Vec<Expr>),
}

/// What evaluation reads: facts, and the answers to story functions.
pub trait Env {
    /// The fact's value, or its declared default when unset.
    fn fact(&self, id: &str) -> Value;
    /// A story function's answer. Unknown quests and nodes answer with the defaults: `0`, `'inactive'`, `''`,
    /// `false`.
    fn call(&self, func: Func, args: &[Value]) -> Value {
        let _ = args;
        match func {
            Func::Visited => Value::Int(0),
            Func::Quest => Value::Text(QUEST_STATES[0].into()),
            Func::Stage => Value::Text(String::new()),
            Func::Done | Func::Failed => Value::Bool(false),
        }
    }
}

impl Expr {
    /// Evaluate against `env`.
    #[must_use]
    pub fn eval(&self, env: &dyn Env) -> Value {
        match self {
            Expr::Lit(value) => value.clone(),
            Expr::Fact(id) => env.fact(id),
            Expr::Not(inner) => Value::Bool(!inner.eval(env).as_bool()),
            Expr::And(a, b) => Value::Bool(a.eval(env).as_bool() && b.eval(env).as_bool()),
            Expr::Or(a, b) => Value::Bool(a.eval(env).as_bool() || b.eval(env).as_bool()),
            Expr::Compare {
                op,
                left,
                right,
                order,
            } => {
                let (left, right) = (left.eval(env), right.eval(env));
                let ordering = match (order, &left, &right) {
                    (Some(order), Value::Text(a), Value::Text(b)) => {
                        let rank = |text: &str| order.iter().position(|v| v == text);
                        rank(a).cmp(&rank(b))
                    }
                    (_, Value::Int(a), Value::Int(b)) => a.cmp(b),
                    _ => {
                        if left == right {
                            std::cmp::Ordering::Equal
                        } else {
                            std::cmp::Ordering::Less
                        }
                    }
                };
                Value::Bool(op.holds(ordering))
            }
            Expr::In { member, set } => {
                let member = member.eval(env);
                Value::Bool(match set.eval(env) {
                    Value::Set(set) => set.contains(member.as_text()),
                    _ => false,
                })
            }
            Expr::Add(a, b) => {
                Value::Int(a.eval(env).as_int().saturating_add(b.eval(env).as_int()))
            }
            Expr::Sub(a, b) => {
                Value::Int(a.eval(env).as_int().saturating_sub(b.eval(env).as_int()))
            }
            Expr::Neg(inner) => Value::Int(inner.eval(env).as_int().saturating_neg()),
            Expr::Count(inner) => Value::Int(match inner.eval(env) {
                Value::Set(set) => i64::try_from(set.len()).unwrap_or(i64::MAX),
                _ => 0,
            }),
            Expr::Min(a, b) => Value::Int(a.eval(env).as_int().min(b.eval(env).as_int())),
            Expr::Max(a, b) => Value::Int(a.eval(env).as_int().max(b.eval(env).as_int())),
            Expr::Abs(inner) => Value::Int(inner.eval(env).as_int().saturating_abs()),
            Expr::Call(func, args) => {
                let args: Vec<Value> = args.iter().map(|arg| arg.eval(env)).collect();
                env.call(*func, &args)
            }
        }
    }

    /// Visit this expression and everything inside it, depth first.
    pub fn walk(&self, visit: &mut impl FnMut(&Expr)) {
        visit(self);
        match self {
            Expr::Lit(_) | Expr::Fact(_) => {}
            Expr::Not(a) | Expr::Neg(a) | Expr::Count(a) | Expr::Abs(a) => a.walk(visit),
            Expr::And(a, b)
            | Expr::Or(a, b)
            | Expr::Add(a, b)
            | Expr::Sub(a, b)
            | Expr::Min(a, b)
            | Expr::Max(a, b) => {
                a.walk(visit);
                b.walk(visit);
            }
            Expr::Compare { left, right, .. } => {
                left.walk(visit);
                right.walk(visit);
            }
            Expr::In { member, set } => {
                member.walk(visit);
                set.walk(visit);
            }
            Expr::Call(_, args) => {
                for arg in args {
                    arg.walk(visit);
                }
            }
        }
    }
}

/// A parameter type for a declared command.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ParamType {
    Bool,
    Int,
    Text,
    /// Anything: the game checks it.
    Any,
}

/// A command a game accepts from story effects (`give_item('lantern', 1)`). It reaches the game as an event.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CommandDef {
    pub name: String,
    #[serde(default)]
    pub params: Vec<ParamType>,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub doc: String,
}

/// The story's own commands. Their arguments are ids, checked by the content lint.
pub const BUILTIN_COMMANDS: &[(&str, &[ParamType])] = &[
    ("start_quest", &[ParamType::Text]),
    ("set_stage", &[ParamType::Text, ParamType::Text]),
    ("complete_objective", &[ParamType::Text, ParamType::Text]),
    ("fail_objective", &[ParamType::Text, ParamType::Text]),
    ("complete_quest", &[ParamType::Text]),
    ("fail_quest", &[ParamType::Text]),
    ("reset_quest", &[ParamType::Text]),
    ("journal", &[ParamType::Text]),
    ("journal_obscure", &[ParamType::Text]),
    ("journal_restore", &[ParamType::Text]),
];

/// What compilation checks names against.
#[derive(Clone, Copy, Debug)]
pub struct Context<'a> {
    pub schema: &'a Schema,
    pub commands: &'a [CommandDef],
}

impl<'a> Context<'a> {
    #[must_use]
    pub fn new(schema: &'a Schema, commands: &'a [CommandDef]) -> Self {
        Self { schema, commands }
    }

    fn check(&self, raw: &(Raw, usize)) -> Result<(Expr, Ty, Option<String>), ExprError> {
        let (raw, at) = raw;
        let at = *at;
        Ok(match raw {
            Raw::Lit(value) => {
                let literal = match value {
                    Value::Text(text) => Some(text.clone()),
                    _ => None,
                };
                let ty = match value {
                    Value::Bool(_) => Ty::Bool,
                    Value::Int(_) => Ty::Int,
                    Value::Text(_) => Ty::Text,
                    Value::Set(_) => Ty::Set(Arc::from(Vec::new())),
                };
                (Expr::Lit(value.clone()), ty, literal)
            }
            Raw::Ident(name) => {
                let def = self.schema.get(name).ok_or_else(|| {
                    ExprError::new(at, format!("`{name}` is not a declared fact"))
                })?;
                (Expr::Fact(name.clone()), Ty::of(def), None)
            }
            Raw::Unary(UnaryOp::Not, inner) => {
                let (expr, ty, _) = self.check(inner)?;
                expect(&ty, &Ty::Bool, inner.1, "`not`")?;
                (Expr::Not(Box::new(expr)), Ty::Bool, None)
            }
            Raw::Unary(UnaryOp::Neg, inner) => {
                let (expr, ty, _) = self.check(inner)?;
                expect(&ty, &Ty::Int, inner.1, "`-`")?;
                (Expr::Neg(Box::new(expr)), Ty::Int, None)
            }
            Raw::Binary(op, left, right) => {
                let (a, at_ty, a_lit) = self.check(left)?;
                let (b, bt_ty, b_lit) = self.check(right)?;
                match op {
                    BinaryOp::And | BinaryOp::Or => {
                        let word = if *op == BinaryOp::And {
                            "`and`"
                        } else {
                            "`or`"
                        };
                        expect(&at_ty, &Ty::Bool, left.1, word)?;
                        expect(&bt_ty, &Ty::Bool, right.1, word)?;
                        let (a, b) = (Box::new(a), Box::new(b));
                        (
                            if *op == BinaryOp::And {
                                Expr::And(a, b)
                            } else {
                                Expr::Or(a, b)
                            },
                            Ty::Bool,
                            None,
                        )
                    }
                    BinaryOp::Plus | BinaryOp::Minus => {
                        expect(&at_ty, &Ty::Int, left.1, "arithmetic")?;
                        expect(&bt_ty, &Ty::Int, right.1, "arithmetic")?;
                        let (a, b) = (Box::new(a), Box::new(b));
                        (
                            if *op == BinaryOp::Plus {
                                Expr::Add(a, b)
                            } else {
                                Expr::Sub(a, b)
                            },
                            Ty::Int,
                            None,
                        )
                    }
                    BinaryOp::In => {
                        let Ty::Set(allowed) = &bt_ty else {
                            return Err(ExprError::new(
                                right.1,
                                format!("`in` needs a set on the right, not {}", bt_ty.name()),
                            ));
                        };
                        if !matches!(at_ty, Ty::Text | Ty::Enum(_)) {
                            return Err(ExprError::new(
                                left.1,
                                format!("a set holds names; {} cannot be in one", at_ty.name()),
                            ));
                        }
                        if let Some(member) = &a_lit
                            && !allowed.is_empty()
                            && !allowed.iter().any(|v| v == member)
                        {
                            return Err(ExprError::new(
                                left.1,
                                format!(
                                    "'{member}' can never be in this set ({})",
                                    allowed.join(", ")
                                ),
                            ));
                        }
                        (
                            Expr::In {
                                member: Box::new(a),
                                set: Box::new(b),
                            },
                            Ty::Bool,
                            None,
                        )
                    }
                    BinaryOp::Cmp(cmp) => {
                        let order = compare_types(
                            *cmp,
                            (&at_ty, a_lit.as_deref(), left.1),
                            (&bt_ty, b_lit.as_deref(), right.1),
                            at,
                        )?;
                        (
                            Expr::Compare {
                                op: *cmp,
                                left: Box::new(a),
                                right: Box::new(b),
                                order,
                            },
                            Ty::Bool,
                            None,
                        )
                    }
                }
            }
            Raw::Call(name, args) => self.call(name, args, at)?,
        })
    }

    fn call(
        &self,
        name: &str,
        args: &[(Raw, usize)],
        at: usize,
    ) -> Result<(Expr, Ty, Option<String>), ExprError> {
        let (params, ret): (&[Ty], Ty) = match name {
            "count" => (&[], Ty::Int),
            "min" | "max" => (&[Ty::Int, Ty::Int], Ty::Int),
            "abs" => (&[Ty::Int], Ty::Int),
            "visited" => (&[Ty::Text], Ty::Int),
            "quest" => (
                &[Ty::Text],
                Ty::Enum(QUEST_STATES.iter().map(|s| (*s).to_owned()).collect()),
            ),
            "stage" => (&[Ty::Text], Ty::Text),
            "done" | "failed" => (&[Ty::Text, Ty::Text], Ty::Bool),
            _ => {
                let hint = if BUILTIN_COMMANDS.iter().any(|(c, _)| *c == name)
                    || self.commands.iter().any(|c| c.name == name)
                {
                    " (it is a command: commands go in effects, not conditions)"
                } else {
                    ""
                };
                return Err(ExprError::new(
                    at,
                    format!("`{name}` is not a function{hint}"),
                ));
            }
        };
        if name == "count" {
            let [arg] = args else {
                return Err(ExprError::new(at, "`count` takes one set"));
            };
            let (expr, ty, _) = self.check(arg)?;
            if !matches!(ty, Ty::Set(_)) {
                return Err(ExprError::new(arg.1, "`count` takes a set"));
            }
            return Ok((Expr::Count(Box::new(expr)), Ty::Int, None));
        }
        if args.len() != params.len() {
            return Err(ExprError::new(
                at,
                format!(
                    "`{name}` takes {} argument(s), got {}",
                    params.len(),
                    args.len()
                ),
            ));
        }
        let mut checked = Vec::with_capacity(args.len());
        for (arg, param) in args.iter().zip(params) {
            let (expr, ty, _) = self.check(arg)?;
            let ok = match param {
                Ty::Text => matches!(ty, Ty::Text | Ty::Enum(_)),
                other => ty == *other,
            };
            if !ok {
                return Err(ExprError::new(
                    arg.1,
                    format!("`{name}` wants {} here, not {}", param.name(), ty.name()),
                ));
            }
            checked.push(expr);
        }
        let expr = match (name, checked.as_slice()) {
            ("min", [a, b]) => Expr::Min(Box::new(a.clone()), Box::new(b.clone())),
            ("max", [a, b]) => Expr::Max(Box::new(a.clone()), Box::new(b.clone())),
            ("abs", [a]) => Expr::Abs(Box::new(a.clone())),
            _ => Expr::Call(
                match name {
                    "visited" => Func::Visited,
                    "quest" => Func::Quest,
                    "stage" => Func::Stage,
                    "done" => Func::Done,
                    _ => Func::Failed,
                },
                checked,
            ),
        };
        Ok((expr, ret, None))
    }
}

fn expect(found: &Ty, wanted: &Ty, at: usize, what: &str) -> Result<(), ExprError> {
    if found == wanted {
        Ok(())
    } else {
        Err(ExprError::new(
            at,
            format!("{what} needs {}, not {}", wanted.name(), found.name()),
        ))
    }
}

/// Check one comparison and say whether it compares by enum order.
fn compare_types(
    op: CmpOp,
    (a, a_lit, a_at): (&Ty, Option<&str>, usize),
    (b, b_lit, b_at): (&Ty, Option<&str>, usize),
    at: usize,
) -> Result<Option<Arc<[String]>>, ExprError> {
    let ordered = !matches!(op, CmpOp::Eq | CmpOp::Ne);
    let variant = |variants: &Arc<[String]>, literal: Option<&str>, where_at: usize| {
        if let Some(text) = literal
            && !variants.iter().any(|v| v == text)
        {
            return Err(ExprError::new(
                where_at,
                format!("'{text}' is not one of {}", variants.join(", ")),
            ));
        }
        Ok(())
    };
    match (a, b) {
        (Ty::Int, Ty::Int) => Ok(None),
        (Ty::Enum(x), Ty::Enum(y)) => {
            if x != y {
                return Err(ExprError::new(at, "these two enums have different values"));
            }
            Ok(ordered.then(|| x.clone()))
        }
        (Ty::Enum(variants), Ty::Text) => {
            variant(variants, b_lit, b_at)?;
            if ordered && b_lit.is_none() {
                return Err(ExprError::new(
                    b_at,
                    "order an enum against one of its values",
                ));
            }
            Ok(ordered.then(|| variants.clone()))
        }
        (Ty::Text, Ty::Enum(variants)) => {
            variant(variants, a_lit, a_at)?;
            if ordered && a_lit.is_none() {
                return Err(ExprError::new(
                    a_at,
                    "order an enum against one of its values",
                ));
            }
            Ok(ordered.then(|| variants.clone()))
        }
        (Ty::Bool, Ty::Bool) | (Ty::Text, Ty::Text) | (Ty::Set(_), Ty::Set(_)) if !ordered => {
            Ok(None)
        }
        (x, y) if x == y || (matches!(x, Ty::Set(_)) && matches!(y, Ty::Set(_))) => Err(
            ExprError::new(at, format!("{} cannot be ordered", x.name())),
        ),
        (x, y) => Err(ExprError::new(
            at,
            format!("cannot compare {} with {}", x.name(), y.name()),
        )),
    }
}

// ---------------------------------------------------------------------------------------------------------------
// Conditions

/// A checked condition and the text it came from. An empty source is always true.
#[derive(Clone, Debug, PartialEq)]
pub struct Condition {
    source: String,
    expr: Option<Expr>,
}

impl Default for Condition {
    fn default() -> Self {
        Self::always()
    }
}

impl Condition {
    /// The condition that always holds.
    #[must_use]
    pub fn always() -> Self {
        Self {
            source: String::new(),
            expr: None,
        }
    }

    /// Parse and type-check `source` against `ctx`. Blank text is [`Condition::always`].
    pub fn compile(source: &str, ctx: Context<'_>) -> Result<Self, ExprError> {
        let Some(raw) = parse_condition(source)? else {
            return Ok(Self::always());
        };
        let (expr, ty, _) = ctx.check(&raw)?;
        if ty != Ty::Bool {
            return Err(ExprError::new(
                raw.1,
                format!("a condition must be yes or no; this is {}", ty.name()),
            ));
        }
        Ok(Self {
            source: source.trim().to_owned(),
            expr: Some(expr),
        })
    }

    #[must_use]
    pub fn eval(&self, env: &dyn Env) -> bool {
        self.expr
            .as_ref()
            .is_none_or(|expr| expr.eval(env).as_bool())
    }

    #[must_use]
    pub fn source(&self) -> &str {
        &self.source
    }

    #[must_use]
    pub fn is_always(&self) -> bool {
        self.expr.is_none()
    }

    #[must_use]
    pub fn expr(&self) -> Option<&Expr> {
        self.expr.as_ref()
    }
}

impl fmt::Display for Condition {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(if self.source.is_empty() {
            "always"
        } else {
            &self.source
        })
    }
}

// ---------------------------------------------------------------------------------------------------------------
// Effects

/// One checked effect.
#[derive(Clone, Debug, PartialEq)]
pub enum Effect {
    Set {
        fact: String,
        value: Expr,
    },
    Add {
        fact: String,
        delta: Expr,
    },
    Insert {
        fact: String,
        member: Expr,
    },
    Remove {
        fact: String,
        member: Expr,
    },
    Clear {
        fact: String,
    },
    /// A story command (`start_quest`) or one the game declared.
    Command {
        name: String,
        args: Vec<Expr>,
    },
}

impl Effect {
    /// The fact this writes, if any.
    #[must_use]
    pub fn fact(&self) -> Option<&str> {
        match self {
            Effect::Set { fact, .. }
            | Effect::Add { fact, .. }
            | Effect::Insert { fact, .. }
            | Effect::Remove { fact, .. }
            | Effect::Clear { fact } => Some(fact),
            Effect::Command { .. } => None,
        }
    }

    /// Every expression this reads.
    pub fn exprs(&self) -> Vec<&Expr> {
        match self {
            Effect::Set { value: e, .. }
            | Effect::Add { delta: e, .. }
            | Effect::Insert { member: e, .. }
            | Effect::Remove { member: e, .. } => vec![e],
            Effect::Clear { .. } => vec![],
            Effect::Command { args, .. } => args.iter().collect(),
        }
    }
}

/// A checked list of effects and the text it came from.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Effects {
    source: String,
    list: Vec<Effect>,
}

impl Effects {
    /// Parse and type-check `source` against `ctx`. Blank text is no effects.
    pub fn compile(source: &str, ctx: Context<'_>) -> Result<Self, ExprError> {
        let list = parse_effects(source)?
            .into_iter()
            .map(|raw| check_effect(raw, ctx))
            .collect::<Result<_, _>>()?;
        Ok(Self {
            source: source.trim().to_owned(),
            list,
        })
    }

    #[must_use]
    pub fn source(&self) -> &str {
        &self.source
    }

    #[must_use]
    pub fn list(&self) -> &[Effect] {
        &self.list
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.list.is_empty()
    }
}

/// Check only that `source` parses, as a condition or an effect list. For validating one file without the game's
/// manifest; unknown facts and types are the full compile's business.
pub fn syntax(source: &str, effects: bool) -> Result<(), ExprError> {
    if effects {
        parse_effects(source).map(|_| ())
    } else {
        parse_condition(source).map(|_| ())
    }
}

fn parse_condition(source: &str) -> Result<Option<(Raw, usize)>, ExprError> {
    if source.trim().is_empty() {
        return Ok(None);
    }
    let mut parser = Parser {
        toks: lex(source, false)?,
        pos: 0,
    };
    let raw = parser.expr()?;
    if *parser.peek() != Tok::End {
        return Err(ExprError::new(
            parser.at(),
            format!("unexpected {} after the condition", describe(parser.peek())),
        ));
    }
    Ok(Some(raw))
}

/// An effect as written, before its names are checked.
enum RawEffect {
    Clear {
        fact: String,
        at: usize,
    },
    Command {
        name: String,
        at: usize,
        args: Vec<(Raw, usize)>,
    },
    Write {
        fact: String,
        at: usize,
        op: Tok,
        op_at: usize,
        value: (Raw, usize),
    },
}

fn parse_effects(source: &str) -> Result<Vec<RawEffect>, ExprError> {
    let mut parser = Parser {
        toks: lex(source, true)?,
        pos: 0,
    };
    let mut list = Vec::new();
    loop {
        while *parser.peek() == Tok::Sep {
            parser.bump();
        }
        if *parser.peek() == Tok::End {
            return Ok(list);
        }
        list.push(parse_effect(&mut parser)?);
        match parser.peek() {
            Tok::Sep | Tok::End => {}
            other => {
                return Err(ExprError::new(
                    parser.at(),
                    format!("expected `;` between effects, found {}", describe(other)),
                ));
            }
        }
    }
}

fn parse_effect(parser: &mut Parser) -> Result<RawEffect, ExprError> {
    let (tok, at) = parser.bump();
    if tok == Tok::Clear {
        let (tok, at) = parser.bump();
        let Tok::Ident(fact) = tok else {
            return Err(ExprError::new(at, "`clear` names a fact"));
        };
        return Ok(RawEffect::Clear { fact, at });
    }
    let Tok::Ident(name) = tok else {
        return Err(ExprError::new(
            at,
            format!(
                "an effect starts with a fact or a command, not {}",
                describe(&tok)
            ),
        ));
    };
    if *parser.peek() == Tok::LParen {
        parser.bump();
        let args = parser.args()?;
        return Ok(RawEffect::Command { name, at, args });
    }
    let (op, op_at) = parser.bump();
    if !matches!(
        op,
        Tok::Assign | Tok::PlusAssign | Tok::MinusAssign | Tok::Add | Tok::Remove
    ) {
        return Err(ExprError::new(
            op_at,
            format!(
                "after {name} comes `=`, `+=`, `-=`, `add` or `remove`, not {}",
                describe(&op)
            ),
        ));
    }
    let value = parser.expr()?;
    Ok(RawEffect::Write {
        fact: name,
        at,
        op,
        op_at,
        value,
    })
}

fn check_effect(raw: RawEffect, ctx: Context<'_>) -> Result<Effect, ExprError> {
    let (name, at, op, op_at, value) = match raw {
        RawEffect::Clear { fact, at } => {
            ctx.schema
                .get(&fact)
                .ok_or_else(|| ExprError::new(at, format!("`{fact}` is not a declared fact")))?;
            return Ok(Effect::Clear { fact });
        }
        RawEffect::Command { name, at, args } => return check_command(name, at, &args, ctx),
        RawEffect::Write {
            fact,
            at,
            op,
            op_at,
            value,
        } => (fact, at, op, op_at, value),
    };
    let def = ctx
        .schema
        .get(&name)
        .ok_or_else(|| ExprError::new(at, format!("`{name}` is not a declared fact")))?;
    let fact_ty = Ty::of(def);
    let (expr, ty, literal) = ctx.check(&value)?;
    let mismatch = |wanted: &str| {
        ExprError::new(
            value.1,
            format!("{name} is {wanted}; this is {}", ty.name()),
        )
    };
    match op {
        Tok::Assign => {
            match (&fact_ty, &ty) {
                (Ty::Enum(variants), Ty::Text) => {
                    if let Some(text) = &literal
                        && !variants.iter().any(|v| v == text)
                    {
                        return Err(ExprError::new(
                            value.1,
                            format!("'{text}' is not one of {}", variants.join(", ")),
                        ));
                    }
                }
                (Ty::Enum(a), Ty::Enum(b)) if a == b => {}
                (Ty::Set(_), Ty::Set(_)) => {}
                (a, b) if a == b => {}
                _ => return Err(mismatch(fact_ty.name())),
            }
            Ok(Effect::Set {
                fact: name,
                value: expr,
            })
        }
        Tok::PlusAssign | Tok::MinusAssign => {
            if fact_ty != Ty::Int {
                return Err(ExprError::new(
                    op_at,
                    format!(
                        "only a number can be raised or lowered; {name} is {}",
                        fact_ty.name()
                    ),
                ));
            }
            if ty != Ty::Int {
                return Err(mismatch("a number"));
            }
            Ok(Effect::Add {
                fact: name,
                delta: if op == Tok::MinusAssign {
                    Expr::Neg(Box::new(expr))
                } else {
                    expr
                },
            })
        }
        _ => {
            let Ty::Set(allowed) = &fact_ty else {
                return Err(ExprError::new(
                    op_at,
                    format!(
                        "only a set takes add and remove; {name} is {}",
                        fact_ty.name()
                    ),
                ));
            };
            if !matches!(ty, Ty::Text | Ty::Enum(_)) {
                return Err(mismatch("a set of names"));
            }
            if let Some(text) = &literal
                && !allowed.is_empty()
                && !allowed.iter().any(|v| v == text)
            {
                return Err(ExprError::new(
                    value.1,
                    format!("'{text}' is not an allowed member of {name}"),
                ));
            }
            Ok(if op == Tok::Add {
                Effect::Insert {
                    fact: name,
                    member: expr,
                }
            } else {
                Effect::Remove {
                    fact: name,
                    member: expr,
                }
            })
        }
    }
}

fn check_command(
    name: String,
    at: usize,
    raw_args: &[(Raw, usize)],
    ctx: Context<'_>,
) -> Result<Effect, ExprError> {
    let params: Vec<ParamType> =
        if let Some((_, params)) = BUILTIN_COMMANDS.iter().find(|(c, _)| *c == name) {
            params.to_vec()
        } else if let Some(def) = ctx.commands.iter().find(|c| c.name == name) {
            def.params.clone()
        } else {
            return Err(ExprError::new(
                at,
                format!("`{name}` is not a story command or one this game declares"),
            ));
        };
    if raw_args.len() != params.len() {
        return Err(ExprError::new(
            at,
            format!(
                "`{name}` takes {} argument(s), got {}",
                params.len(),
                raw_args.len()
            ),
        ));
    }
    let mut args = Vec::with_capacity(raw_args.len());
    for (raw, param) in raw_args.iter().zip(params) {
        let (expr, ty, _) = ctx.check(raw)?;
        let ok = match param {
            ParamType::Any => !matches!(ty, Ty::Set(_)),
            ParamType::Bool => ty == Ty::Bool,
            ParamType::Int => ty == Ty::Int,
            ParamType::Text => matches!(ty, Ty::Text | Ty::Enum(_)),
        };
        if !ok {
            return Err(ExprError::new(
                raw.1,
                format!("`{name}` does not take {} here", ty.name()),
            ));
        }
        args.push(expr);
    }
    Ok(Effect::Command { name, args })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::value::{FactDef, FactType};
    use std::collections::BTreeMap;

    fn schema() -> Schema {
        Schema::new([
            FactDef::new("met.mireille", FactType::Bool),
            FactDef::new("mireille.affection", FactType::Int).with_range(-10, 10),
            FactDef::new("gold", FactType::Int),
            FactDef::new("name", FactType::Text),
            FactDef::new("lang.patter", FactType::Enum).with_values(&[
                "none",
                "phrases",
                "conversant",
                "fluent",
                "native",
            ]),
            FactDef::new("lang.tyffiran", FactType::Enum).with_values(&[
                "none",
                "phrases",
                "conversant",
                "fluent",
                "native",
            ]),
            FactDef::new("caelan.weaves", FactType::Set),
            FactDef::new("seasons", FactType::Set).with_values(&["frost", "thaw"]),
            FactDef::new("rep.*", FactType::Int),
        ])
        .unwrap()
    }

    struct Facts(BTreeMap<&'static str, Value>, Schema);

    impl Env for Facts {
        fn fact(&self, id: &str) -> Value {
            self.0
                .get(id)
                .cloned()
                .unwrap_or_else(|| self.1.get(id).unwrap().default_value())
        }
        fn call(&self, func: Func, args: &[Value]) -> Value {
            match (func, args) {
                (Func::Quest, [Value::Text(q)]) if q == "varga_job" => "active".into(),
                (Func::Visited, [Value::Text(n)]) if n == "greet" => Value::Int(2),
                _ => match func {
                    Func::Visited => Value::Int(0),
                    Func::Quest => "inactive".into(),
                    Func::Stage => "".into(),
                    Func::Done | Func::Failed => false.into(),
                },
            }
        }
    }

    fn env() -> Facts {
        Facts(
            [
                ("met.mireille", Value::Bool(true)),
                ("mireille.affection", Value::Int(4)),
                ("lang.patter", "conversant".into()),
                (
                    "caelan.weaves",
                    Value::Set(["frost_step".to_owned()].into()),
                ),
            ]
            .into(),
            schema(),
        )
    }

    fn holds(source: &str) -> bool {
        let schema = schema();
        Condition::compile(source, Context::new(&schema, &[]))
            .unwrap_or_else(|e| panic!("{source}: {e}"))
            .eval(&env())
    }

    fn refused(source: &str) -> ExprError {
        let schema = schema();
        Condition::compile(source, Context::new(&schema, &[])).unwrap_err()
    }

    #[test]
    fn conditions_read_facts_and_defaults() {
        assert!(holds("met.mireille"));
        assert!(holds("met.mireille and mireille.affection >= 3"));
        assert!(!holds("mireille.affection > 4"));
        assert!(
            holds("not rep.lance > 0 and rep.lance == 0"),
            "an unset family member reads its default"
        );
        assert!(holds(""), "blank is always");
        assert!(holds("name == ''"));
        assert!(holds(
            "gold - 5 < 0 and -gold == 0 and abs(-3) == 3 and max(gold, 2) == 2"
        ));
        assert!(holds("(met.mireille || false) && !(gold != 0)"));
    }

    #[test]
    fn enums_compare_by_declared_order_and_check_their_variants() {
        assert!(holds("lang.patter >= 'conversant'"));
        assert!(holds("lang.patter < \"fluent\""));
        assert!(!holds("lang.patter == 'native'"));
        assert!(
            holds("lang.tyffiran < lang.patter"),
            "two enums with one variant list order together"
        );
        assert_eq!(refused("lang.patter >= 'conversent'").at, 15);
        assert!(
            refused("lang.patter >= name")
                .message
                .contains("one of its values")
        );
    }

    #[test]
    fn sets_and_story_functions() {
        assert!(holds(
            "'frost_step' in caelan.weaves and count(caelan.weaves) == 1"
        ));
        assert!(!holds("'kindled_body' in caelan.weaves"));
        assert!(holds(
            "quest('varga_job') == 'active' and visited('greet') >= 2"
        ));
        assert!(holds(
            "quest('other') == 'inactive' and stage('other') == '' and not done('a', 'b')"
        ));
        assert!(
            refused("'summer' in seasons")
                .message
                .contains("never be in")
        );
        assert!(
            refused("quest('x') == 'won'")
                .message
                .contains("not one of")
        );
    }

    #[test]
    fn mistakes_are_refused_where_they_are() {
        let e = refused("met.mirielle");
        assert_eq!((e.at, e.message.contains("not a declared fact")), (0, true));
        assert!(refused("mireille.affection").message.contains("yes or no"));
        assert!(
            refused("met.mireille = true")
                .message
                .contains("compare with `==`")
        );
        assert!(refused("gold == 'lots'").message.contains("cannot compare"));
        assert!(refused("1 < gold < 3").message.contains("do not chain"));
        assert!(
            refused("gold >= 3 and")
                .message
                .contains("expected a value")
        );
        assert!(refused("'unclosed").message.contains("never closed"));
        assert!(
            refused("start_quest('x')")
                .message
                .contains("it is a command")
        );
        assert!(refused("met.mireille & gold").message.contains("use `and`"));
        assert!(refused("3x").message.contains("whole"));
        assert!(refused("met..x").message.contains("not a valid id"));
    }

    #[test]
    fn effects_check_their_targets() {
        let schema = schema();
        let commands = [CommandDef {
            name: "give_item".into(),
            params: vec![ParamType::Text, ParamType::Int],
            doc: String::new(),
        }];
        let ctx = Context::new(&schema, &commands);
        let effects = Effects::compile(
            "mireille.affection += 2; met.mireille = true\n caelan.weaves add 'frost_step' # a comment\n\
             lang.patter = 'fluent'; clear gold; give_item('lantern', 1); start_quest('varga_job')\n\
             rep.lance -= 3",
            ctx,
        )
        .unwrap();
        assert_eq!(effects.list().len(), 8);
        assert!(matches!(
            &effects.list()[7],
            Effect::Add {
                delta: Expr::Neg(_),
                ..
            }
        ));
        for (source, needle) in [
            ("met.mireille += 1", "only a number"),
            ("gold = 'x'", "gold is a number"),
            ("lang.patter = 'klingon'", "not one of"),
            ("seasons add 'summer'", "not an allowed member"),
            ("gold add 'x'", "only a set"),
            ("give_item('lantern')", "takes 2"),
            ("give_item(1, 1)", "does not take"),
            ("teleport('x')", "not a story command"),
            ("gold = 1 gold = 2", "expected `;`"),
            ("clear 3", "names a fact"),
            ("unknown = 1", "not a declared fact"),
        ] {
            let error = Effects::compile(source, ctx).unwrap_err();
            assert!(error.message.contains(needle), "{source}: {error}");
        }
        assert!(Effects::compile("  ", ctx).unwrap().is_empty());
    }

    #[test]
    fn syntax_needs_no_schema() {
        assert!(syntax("anything.at_all >= 'x' and f(1, 2)", false).is_ok());
        assert!(syntax("x += 1; teleport('y')\n clear z", true).is_ok());
        assert!(syntax("x >=", false).is_err());
        assert!(
            syntax("x 1", true)
                .unwrap_err()
                .message
                .contains("comes `=`")
        );
    }

    #[test]
    fn walk_finds_every_story_call() {
        let schema = schema();
        let condition = Condition::compile(
            "quest('a') == 'active' or visited('n1') > 0 and done('a', 'o')",
            Context::new(&schema, &[]),
        )
        .unwrap();
        let mut calls = Vec::new();
        condition.expr().unwrap().walk(&mut |e| {
            if let Expr::Call(func, _) = e {
                calls.push(func.name());
            }
        });
        assert_eq!(calls, ["quest", "visited", "done"]);
    }
}
