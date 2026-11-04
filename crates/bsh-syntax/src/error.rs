//! Syntax errors with source positions.

use crate::span::Span;
use std::fmt;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SyntaxError {
    pub message: String,
    pub span: Span,
}

impl SyntaxError {
    pub fn new(message: impl Into<String>, span: Span) -> Self {
        SyntaxError {
            message: message.into(),
            span,
        }
    }
}

impl fmt::Display for SyntaxError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} at line {}", self.message, self.span.line)
    }
}

impl std::error::Error for SyntaxError {}
