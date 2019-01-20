use super::Primitive::{self, Undefined};
use super::SExp::{self, Atom};
use super::{Env, Error, Result};

mod base;
mod core;
mod math;
pub mod utils;
mod write;

/// Evaluation context for LISP expressions.
///
/// ## Note
/// `Context::default()` only provides *very* basic utilities. To obtain an
/// evaluation context with useful functions available, use
/// [`Context::base()`](#method.base).
///
/// ## Some implementation details
/// `Context` maintains separate environments for "core" (special forms, etc.),
/// "lang" (basic functions, vectors, and more), and "user" definitions. Most of
/// the provided methods operate on the "user" environment, as the intended use
/// case keeps the other environments immutable once they have been initialized.
pub struct Context {
    core: Env,
    /// You can `insert` additional definitions here to make them available
    /// throughout the runtime. These definitions will not go out of scope
    /// automatically, but can be overridden (see [`get`](#method.get) for
    /// semantic details).
    pub lang: Env,
    user: Vec<Env>,
    overlay: Option<Env>,
    out: Option<String>,
}

impl Default for Context {
    fn default() -> Self {
        Self {
            core: Self::core(),
            lang: Env::new(),
            user: vec![Env::new()],
            overlay: None,
            out: None,
        }
    }
}

impl Context {
    /// Add a new, nested scope.
    ///
    /// See [Context::pop](#method.pop) for a usage example.
    pub fn push(&mut self) {
        self.user.push(Env::new());
    }

    /// Remove the most recently added scope.
    ///
    /// If the stack height is 1, all definitions will be cleared, and the
    /// global scope will be replaced with an empty one.
    ///
    /// # Example
    /// ```
    /// use parsley::prelude::*;
    /// let mut ctx = Context::default();
    ///
    /// assert_eq!(ctx.get("x"), None);
    /// ctx.push();
    /// ctx.define("x", SExp::Null);
    /// assert_eq!(ctx.get("x"), Some(SExp::Null));
    /// ctx.pop();
    /// assert_eq!(ctx.get("x"), None);
    /// ```
    pub fn pop(&mut self) {
        self.user.pop();

        if self.user.is_empty() {
            self.push();
        }
    }

    /// Create a new definition in the current scope.
    pub fn define(&mut self, key: &str, value: SExp) {
        let num_frames = self.user.len();
        self.user[num_frames - 1].insert(key.to_string(), value);
    }

    fn get_user(&self, key: &str) -> Option<SExp> {
        self.user
            .iter()
            .rev()
            .find_map(|w| w.get(key))
            .map(Clone::clone)
    }

    /// Get the definition for a symbol in the execution environment.
    ///
    /// Returns `None` if no definition is found.
    ///
    /// # Override semantics
    /// This method searches for a definition in the following order:
    ///
    ///   1. The core language
    ///   2. The current closure overlay (if there is one)
    ///   3. User definitions, starting from the most recent scope and working
    ///      backward to the top-level
    ///   4. [Language-level definitions](#structfield.lang)
    ///
    /// What this means is that definitions populated in the `lang` field can be
    /// overridden inside the runtime (e.g. in a REPL), but special form keywords
    /// cannot. For example, we can `(define null "foo")`, but we cannot
    /// `(set! and or)`.
    ///
    /// # Examples
    /// ```
    /// let ctx = parsley::Context::default(); // only core definitions included
    /// assert!(ctx.get("potato").is_none());
    /// ```
    /// ```
    /// use parsley::prelude::*;
    /// let mut ctx = Context::default();
    ///
    /// ctx.define("x", SExp::from(3));
    /// assert_eq!(ctx.get("x"), Some(SExp::from(3)));
    /// ```
    pub fn get(&self, key: &str) -> Option<SExp> {
        // first check core (reserved keywords)
        if let Some(exp) = self.core.get(key) {
            return Some(exp.clone());
        }

        // then check the overlay
        if let Some(env) = &self.overlay {
            if let Some(exp) = env.get(key) {
                return Some(exp.clone());
            }
        }

        // then check user definitions (could have overridden library definitions)
        if let Some(exp) = self.get_user(key) {
            return Some(exp);
        }

        // then check the stdlib
        if let Some(exp) = self.lang.get(key) {
            return Some(exp.clone());
        }

        // otherwise fail
        None
    }

    /// Re-bind an existing definition to a new value.
    ///
    /// Returns `Ok` if an existing definition was found and updated. Returns
    /// `Err` if no definition exists.
    ///
    /// # Example
    /// ```
    /// use parsley::prelude::*;
    /// let mut ctx = Context::default();
    ///
    /// assert!(ctx.set("x", SExp::from(false)).is_err());    // Err, because x is not yet defined
    /// ctx.define("x", SExp::from(3));                       // define x
    /// assert_eq!(ctx.get("x"), Some(SExp::from(3)));        // check that its value is 3
    /// assert!(ctx.set("x", SExp::from("potato")).is_ok());  // Ok because x is now defined
    /// assert_eq!(ctx.get("x"), Some(SExp::from("potato"))); // check that its value is now "potato"
    /// ```
    pub fn set(&mut self, key: &str, value: SExp) -> Result {
        for frame in self.user.iter_mut().rev() {
            if frame.contains_key(key) {
                frame.insert(key.to_string(), value);
                return Ok(Atom(Undefined));
            }
        }
        Err(Error::UndefinedSymbol {
            sym: key.to_string(),
        })
    }

    /// Get a snapshot of environment state for a list of symbols.
    pub fn close(&self, vars: Vec<&str>) -> Env {
        let mut out = Env::new();

        for var in vars {
            if let Some(exp) = self.get_user(var) {
                out.insert(var.to_string(), exp);
            }
        }

        out
    }

    /// Use definitions from a closure (or purge the existing ones)
    pub fn overlay_env(&mut self, env: Option<Env>) {
        self.overlay = env;
    }

    /// Run a code snippet in an existing `Context`.
    ///
    /// # Example
    /// ```
    /// use parsley::prelude::*;
    /// let mut ctx = Context::base();
    ///
    /// assert!(ctx.run("x").is_err());
    /// assert!(ctx.run("(define x 6)").is_ok());
    /// assert_eq!(ctx.run("x").unwrap(), SExp::from(6));
    /// ```
    pub fn run(&mut self, expr: &str) -> Result {
        self.eval(expr.parse::<SExp>()?)
    }

    /// Evaluate an S-Expression in a context.
    ///
    /// The context will retain any definitions bound during evaluation
    /// (e.g. `define`, `set!`).
    ///
    /// # Examples
    /// ```
    /// use parsley::prelude::*;
    /// let result = sexp![SExp::sym("eq?"), 0, 1].eval(&mut Context::base());
    /// assert_eq!(result.unwrap(), SExp::from(false));
    /// ```
    /// ```
    /// use parsley::prelude::*;
    /// let exp1 = sexp![SExp::sym("define"), SExp::sym("x"), 10];
    /// let exp2 = SExp::sym("x");
    ///
    /// let mut ctx = Context::base();
    ///
    /// exp1.eval(&mut ctx);
    /// assert_eq!(exp2.eval(&mut ctx).unwrap(), SExp::from(10));
    /// ```
    pub fn eval(&mut self, expr: SExp) -> Result {
        use super::primitives::proc::Procedure::Ctx;
        use SExp::{Atom, Null, Pair, Vector};

        match expr {
            Null => Err(Error::NullList),
            Atom(Primitive::Symbol(sym)) => match self.get(&sym) {
                None | Some(Atom(Primitive::Undefined)) => Err(Error::UndefinedSymbol { sym }),
                Some(exp) => Ok(exp),
            },
            Atom(_) | Vector(_) => Ok(expr),
            Pair { head, tail } => {
                let proc = self.eval(*head)?;
                let applic = if let Atom(Primitive::Procedure { f: Ctx(_), .. }) = proc {
                    *tail
                } else {
                    tail.into_iter().map(|e| self.eval(e)).collect::<Result>()?
                }
                .cons(proc);
                self.apply(applic)
            }
        }
    }

    fn apply(&mut self, expr: SExp) -> Result {
        use super::primitives::proc::Procedure::{Basic, Ctx};
        use SExp::{Atom, Null, Pair, Vector};

        match expr {
            Null | Atom(_) | Vector(_) => Ok(expr),
            Pair { head, tail } => match *head {
                Atom(Primitive::Procedure { f, env, .. }) => {
                    self.overlay_env(env);
                    let result = match f {
                        Basic(p) => p(*tail),
                        Ctx(p) => p(self, *tail),
                    };
                    self.overlay_env(None);
                    result
                }
                Atom(Primitive::Symbol(sym)) => Err(Error::NotAProcedure {
                    exp: sym.to_string(),
                }),
                Pair {
                    head: proc,
                    tail: tail2,
                } => {
                    let the_proc = self.eval(*proc)?;
                    self.eval(tail2.cons(the_proc))
                },
                _ => Ok(tail.cons(*head)),
            },
        }
    }
}
