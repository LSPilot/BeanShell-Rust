//! Runtime errors.

use crate::value::Value;
use std::fmt;

#[derive(Debug)]
pub enum Error {
    /// A BeanShell evaluation error (parse/eval/reflection level).
    Eval(String),
    /// A value thrown by the script with "throw".
    Thrown(Value),
    /// An explicit "return" escaping a context where it is not allowed.
    Internal(String),
}

pub type Result<T> = std::result::Result<T, Error>;

impl Error {
    pub fn eval(msg: impl Into<String>) -> Error {
        Error::Eval(msg.into())
    }

    pub fn message(&self) -> String {
        match self {
            Error::Eval(m) => m.clone(),
            Error::Thrown(v) => format!("Uncaught exception: {}", v.to_display_string()),
            Error::Internal(m) => m.clone(),
        }
    }

    pub fn is_thrown(&self) -> bool {
        matches!(self, Error::Thrown(_))
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Eval(m) => write!(f, "EvalError: {}", m),
            Error::Thrown(v) => write!(f, "TargetError: {}", v.to_display_string()),
            Error::Internal(m) => write!(f, "InterpreterError: {}", m),
        }
    }
}

impl std::error::Error for Error {}
