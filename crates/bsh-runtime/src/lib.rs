//! Rust implementation of the BeanShell language runtime.

pub mod builtins;
pub mod class;
pub mod error;
pub mod eval;
pub mod fxhash;
pub mod interp;
pub mod java;
pub mod namespace;
pub mod primitive;
pub mod proxy;
pub mod value;

pub use error::{Error, Result};
pub use interp::Interpreter;
pub use value::Value;
