use std::fmt;
use std::rc::Rc;
use std::str::FromStr;

use quicli::prelude::*;

mod as_atom;
mod context;
mod errors;
mod primitives;
mod tests;
mod utils;

use self::as_atom::AsAtom;
pub use self::context::Context;
use self::primitives::Primitive;

const NULL: SExp = SExp::List(Vec::new());

type LispResult = Result<SExp, errors::LispError>;

#[derive(Debug, PartialEq, Clone)]
pub enum SExp {
    Atom(Primitive),
    List(Vec<SExp>),
}

impl fmt::Display for SExp {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            SExp::Atom(a) => write!(f, "{}", a),
            SExp::List(v) => write!(
                f,
                "'({})",
                v.iter()
                    .map(|e| format!("{}", e))
                    .collect::<Vec<String>>()
                    .join(" ")
            ),
        }
    }
}

impl FromStr for SExp {
    type Err = errors::LispError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let code = s.trim().to_owned();

        if code.starts_with('\'')
            && code.len() > 1
            && code.chars().skip(1).all(utils::is_symbol_char)
        {
            debug!("Matched quoted symbol: {}", code);
            Ok(SExp::List(vec![
                SExp::make_symbol("quote"),
                SExp::Atom(code[1..].parse::<Primitive>()?),
            ]))
        } else if code.chars().all(utils::is_atom_char) {
            debug!("Matched atom: {}", code);
            Ok(SExp::Atom(code.parse::<Primitive>()?))
        } else if code.starts_with("'(") && code.ends_with(')') {
            Ok(SExp::List(vec![
                SExp::make_symbol("quote"),
                code.get(1..).unwrap().parse::<SExp>()?,
            ]))
        } else if code.starts_with('(') && code.ends_with(')') {
            match utils::find_closing_delim(&code, '(', ')') {
                Some(idx) => {
                    debug!("Matched list with length {} chars", idx + 1);
                    let mut list_str = code.clone();
                    let mut list_out: Vec<SExp> = Vec::new();

                    if idx + 1 == code.len() {
                        list_str = code.get(1..idx).unwrap().to_string();
                    }

                    while !list_str.is_empty() {
                        debug!(
                            "Processing list string with length {} chars",
                            list_str.len()
                        );

                        if list_str.starts_with('(') {
                            match utils::find_closing_delim(&list_str, '(', ')') {
                                Some(idx2) => {
                                    if idx2 + 1 == list_str.len() {
                                        debug!("Whole string is a single list");
                                        list_out.push(list_str.parse::<SExp>()?);
                                        break;
                                    } else {
                                        debug!("Matched sub-list with length {} chars", idx2 + 1);
                                        let (before, after) = list_str.split_at(idx2 + 1);
                                        list_out.push(before.parse::<SExp>()?);
                                        list_str = after.trim().to_string();
                                    }
                                }
                                None => {
                                    return Err(errors::LispError::SyntaxError {
                                        exp: list_str.to_string(),
                                    });
                                }
                            }
                        } else {
                            if let Ok(prim_val) = list_str.parse::<Primitive>() {
                                list_out.push(SExp::Atom(prim_val));
                                break;
                            }

                            match list_str.find(|c| !utils::is_atom_char(c)) {
                                Some(idx3) if &list_str[..idx3] != "'" => {
                                    debug!(
                                        "Matched atom in first position with length {} chars",
                                        idx3
                                    );
                                    let (first, rest) = list_str.split_at(idx3);
                                    list_out.push(first.parse::<SExp>()?);
                                    list_str = rest.trim().to_string();
                                }
                                _ => {
                                    list_out.push(list_str.parse::<SExp>()?);
                                    break;
                                }
                            }
                        }
                    }

                    Ok(SExp::List(list_out))
                }
                None => Err(errors::LispError::SyntaxError { exp: code }),
            }
        } else {
            let prim = code.parse::<Primitive>()?;
            Ok(SExp::Atom(prim))
        }
    }
}

impl SExp {
    pub fn eval(self, ctx: &mut Context) -> LispResult {
        match self {
            SExp::Atom(Primitive::Symbol(sym)) => match ctx.get(&sym) {
                None => Err(errors::LispError::UndefinedSymbol { sym }),
                Some(exp) => Ok(exp),
            },
            SExp::Atom(_) => Ok(self),
            SExp::List(_) if self.is_null() => Err(errors::LispError::NullList),
            SExp::List(contents) => {
                // handle special functions
                if let Some(result) = SExp::List(contents.clone()).eval_special_form(ctx) {
                    debug!("Special form finished evaluating.");
                    result
                } else {
                    // handle everything else
                    debug!("Evaluating normal list.");
                    match contents.into_iter().map(|e| e.eval(ctx)).collect() {
                        Ok(list) => SExp::List(list).apply(ctx),
                        Err(err) => Err(err),
                    }
                }
            }
        }
    }

    fn eval_special_form(self, ctx: &mut Context) -> Option<LispResult> {
        match self {
            SExp::Atom(_) => None,
            SExp::List(_) if self.is_null() => None,
            SExp::List(contents) => match &contents[0] {
                SExp::Atom(Primitive::Symbol(sym)) => match sym.as_ref() {
                    "lambda" => match contents.len() {
                        1 => Some(Err(errors::LispError::NoArgumentsProvided {
                            symbol: "lambda".to_string(),
                        })),
                        2 => Some(Err(errors::LispError::TooManyArguments {
                            n_args: 1,
                            right_num: 2,
                        })),
                        _ => match contents[1].clone() {
                            SExp::List(params) => {
                                debug!("Creating procedure with {} parameters.", params.len());
                                let params_ = params.to_owned();
                                Some(Ok(SExp::Atom(Primitive::Procedure(Rc::new(move |args| {
                                    let mut elems = vec![SExp::make_symbol("let")];
                                    let bound_params = params_
                                        .iter()
                                        .zip(args.iter())
                                        .map(|p| SExp::List(vec![p.0.clone(), p.1.clone()]))
                                        .collect::<Vec<_>>();
                                    elems.push(SExp::List(bound_params));
                                    for expr in contents.clone().into_iter().skip(2) {
                                        elems.push(expr);
                                    }
                                    Ok(SExp::List(elems))
                                })))))
                            }
                            expr => Some(Err(errors::LispError::SyntaxError {
                                exp: expr.to_string(),
                            })),
                        },
                    },
                    "define" => match contents.len() {
                        1 => Some(Err(errors::LispError::NoArgumentsProvided {
                            symbol: "define".to_string(),
                        })),
                        2 => Some(Err(errors::LispError::TooManyArguments {
                            n_args: 1,
                            right_num: 2,
                        })),
                        n_args => match &contents[1] {
                            SExp::Atom(Primitive::Symbol(sym)) => {
                                if n_args == 3 {
                                    debug!("Defining a quanitity with symbol {}", &sym);
                                    ctx.define(&sym, contents[2].clone());
                                    Some(Ok(contents[2].clone()))
                                } else {
                                    Some(Err(errors::LispError::TooManyArguments {
                                        n_args: n_args - 1,
                                        right_num: 2,
                                    }))
                                }
                            }
                            SExp::List(signature) if !signature.is_empty() => match &signature[0] {
                                SExp::Atom(Primitive::Symbol(sym)) => {
                                    debug!("Defining a function with \"define\" syntax.");
                                    let mut exprs = vec![
                                        SExp::make_symbol("lambda"),
                                        SExp::List(signature[1..].to_vec()),
                                    ];
                                    for expr in contents[2..].iter() {
                                        exprs.push(expr.to_owned());
                                    }
                                    ctx.define(&sym, SExp::List(exprs));
                                    Some(Ok(SExp::Atom(Primitive::Undefined)))
                                }
                                exp => Some(Err(errors::LispError::SyntaxError {
                                    exp: exp.to_string(),
                                })),
                            },
                            exp => Some(Err(errors::LispError::SyntaxError {
                                exp: exp.to_string(),
                            })),
                        },
                    },
                    "set!" => match contents.len() {
                        1 => Some(Err(errors::LispError::NoArgumentsProvided {
                            symbol: "set!".to_string(),
                        })),
                        3 => match &contents[1] {
                            SExp::Atom(Primitive::Symbol(sym)) => {
                                ctx.set(&sym, contents[2].to_owned());
                                Some(Ok(SExp::Atom(Primitive::Undefined)))
                            }
                            other => Some(Err(errors::LispError::SyntaxError {
                                exp: other.to_string(),
                            })),
                        },
                        n => Some(Err(errors::LispError::TooManyArguments {
                            n_args: n - 1,
                            right_num: 2,
                        })),
                    },
                    "let" => match contents.len() {
                        1 => Some(Err(errors::LispError::NoArgumentsProvided {
                            symbol: "let".to_string(),
                        })),
                        2 => Some(Err(errors::LispError::TooManyArguments {
                            n_args: 1,
                            right_num: 2,
                        })),
                        _ => match &contents[1] {
                            SExp::List(vals) => {
                                debug!("Creating a local binding.");
                                let mut new_ctx = ctx.push();
                                match vals.iter().find_map(|e| match e {
                                    SExp::List(kv) if kv.len() == 2 => match &kv[0] {
                                        SExp::Atom(Primitive::Symbol(key)) => {
                                            match kv[1].clone().eval(ctx) {
                                                Ok(val) => {
                                                    new_ctx.define(key, val);
                                                    None
                                                }
                                                Err(stuff) => {
                                                    Some(Err(errors::LispError::SyntaxError {
                                                        exp: stuff.to_string(),
                                                    }))
                                                }
                                            }
                                        }
                                        stuff => Some(Err(errors::LispError::SyntaxError {
                                            exp: stuff.to_string(),
                                        })),
                                    },
                                    stuff => Some(Err(errors::LispError::SyntaxError {
                                        exp: stuff.to_string(),
                                    })),
                                }) {
                                    None => contents
                                        .into_iter()
                                        .skip(2)
                                        .map(|e| e.eval(&mut new_ctx))
                                        .last(),
                                    stuff => stuff,
                                }
                            }
                            stuff => Some(Err(errors::LispError::SyntaxError {
                                exp: stuff.to_string(),
                            })),
                        },
                    },
                    "cond" => match contents.len() {
                        1 => Some(Ok(SExp::Atom(Primitive::Void))),
                        _ => {
                            debug!("Evaluating conditional form.");
                            let false_ = false.as_atom();
                            let else_ = SExp::make_symbol("else");

                            match contents
                                .into_iter()
                                .skip(1)
                                .map(|expr| match expr {
                                    SExp::List(pair) => {
                                        if pair.len() == 2 {
                                            if pair[0] == else_ {
                                                Some(pair[1].clone().eval(ctx))
                                            } else {
                                                match pair[0].clone().eval(ctx) {
                                                    Ok(condition) => {
                                                        if condition == false_ {
                                                            None
                                                        } else {
                                                            Some(pair[1].clone().eval(ctx))
                                                        }
                                                    }
                                                    err => Some(err),
                                                }
                                            }
                                        } else {
                                            Some(Err(errors::LispError::SyntaxError {
                                                exp: format!("{:?}", pair),
                                            }))
                                        }
                                    }
                                    other => Some(Err(errors::LispError::SyntaxError {
                                        exp: other.to_string(),
                                    })),
                                })
                                .skip_while(Option::is_none)
                                .next()
                            {
                                Some(stuff) => stuff,
                                None => Some(Err(errors::LispError::SyntaxError {
                                    exp: "malformed 'cond' statement".to_string(),
                                })),
                            }
                        }
                    },
                    "and" => Some(SExp::eval_and(&contents[1..], ctx)),
                    "begin" => Some(SExp::eval_begin(&contents[1..], ctx)),
                    "if" => Some(SExp::eval_if(&contents[1..], ctx)),
                    "or" => Some(SExp::eval_or(&contents[1..], ctx)),
                    "quote" => Some(SExp::eval_quote(&contents[1..])),
                    _ => None,
                },
                _ => None,
            },
        }
    }

    fn eval_begin(c: &[Self], ctx: &mut Context) -> LispResult {
        if c.is_empty() {
            Err(errors::LispError::NoArgumentsProvided {
                symbol: "begin".to_string(),
            })
        } else {
            debug!("Evaluating \"begin\" sequence.");
            match c.to_owned().into_iter().map(|e| e.eval(ctx)).last() {
                Some(stuff) => stuff,
                None => Err(errors::LispError::SyntaxError {
                    exp: "something bad happened, idk".to_string(),
                }),
            }
        }
    }

    fn eval_and(c: &[Self], ctx: &mut Context) -> LispResult {
        match c.len() {
            0 => Ok(true.as_atom()),
            n => {
                debug!("Evaluating 'and' expression.");
                let false_ = false.as_atom();
                let mut good_stuff = c
                    .to_owned()
                    .into_iter()
                    .map(|expr| expr.eval(ctx))
                    .take_while(|element| match element {
                        Err(_) => false,
                        Ok(ref atom) if *atom == false_ => false,
                        _ => true,
                    })
                    .collect::<Vec<_>>();

                if good_stuff.len() == n {
                    good_stuff.pop().expect("oops")
                } else {
                    Ok(false_)
                }
            }
        }
    }

    fn eval_or(c: &[Self], ctx: &mut Context) -> LispResult {
        let false_ = false.as_atom();
        if c.is_empty() {
            Ok(false_)
        } else {
            debug!("Evaluating 'or' expression.");
            match c
                .to_owned()
                .into_iter()
                .find_map(|expr| match expr.eval(ctx) {
                    Ok(ref atom) if *atom == false_ => None,
                    thing => Some(thing),
                }) {
                Some(thing) => thing,
                _ => Ok(false_),
            }
        }
    }

    fn eval_if(c: &[Self], ctx: &mut Context) -> LispResult {
        match c.len() {
            3 => {
                debug!("Evaluating 'if' expression.");
                if c[0] == true.as_atom() {
                    c[1].to_owned().eval(ctx)
                } else {
                    c[2].to_owned().eval(ctx)
                }
            }
            n_args => Err(errors::LispError::TooManyArguments {
                n_args,
                right_num: 1,
            }),
        }
    }

    fn eval_quote(c: &[Self]) -> LispResult {
        match c.len() {
            1 => Ok(c[0].to_owned()),
            n_args => Err(errors::LispError::TooManyArguments {
                n_args,
                right_num: 1,
            }),
        }
    }

    fn apply(self, ctx: &mut Context) -> LispResult {
        match self {
            SExp::Atom(_) => Ok(self),
            SExp::List(_) if self.is_null() => Ok(NULL),
            SExp::List(contents) => match &contents[0] {
                SExp::Atom(Primitive::Procedure(proc)) => {
                    debug!("Applying a procedure.");
                    proc(&contents[1..])?.eval(ctx)
                }
                l @ SExp::List(_) => {
                    let mut new_contents = vec![l.to_owned().eval(ctx)?];
                    for element in &contents[1..] {
                        new_contents.push(element.to_owned());
                    }
                    SExp::List(new_contents).eval(ctx)
                }
                _ => Ok(SExp::List(contents.clone())),
            },
        }
    }

    fn is_null(&self) -> bool {
        match self {
            SExp::List(ref contents) if contents.is_empty() => true,
            _ => false,
        }
    }

    fn car(&self) -> LispResult {
        match self {
            atom @ SExp::Atom(_) => Err(errors::LispError::NotAList {
                atom: atom.to_string(),
            }),
            SExp::List(_) if self.is_null() => Err(errors::LispError::NullList),
            SExp::List(contents) => Ok(contents[0].clone()),
        }
    }

    fn cdr(&self) -> LispResult {
        match self {
            atom @ SExp::Atom(_) => Err(errors::LispError::NotAList {
                atom: atom.to_string(),
            }),
            SExp::List(_) if self.is_null() => Err(errors::LispError::NullList),
            SExp::List(contents) => Ok(SExp::List(contents[1..].to_vec())),
        }
    }

    fn cons(exp1: Self, exp2: Self) -> Self {
        match exp2 {
            SExp::Atom(_) => SExp::List(vec![exp1, exp2]),
            SExp::List(mut contents) => {
                let mut new_contents = vec![exp1];
                new_contents.append(&mut contents);
                SExp::List(new_contents)
            }
        }
    }

    fn make_symbol(sym: &str) -> Self {
        SExp::Atom(Primitive::Symbol(sym.to_string()))
    }
}
