//! JUnit-style assertion helpers used by BeanShell test scripts.

use crate::error::{Error, Result};
use crate::value::Value;

pub fn call(name: &str, args: &[Value]) -> Result<Value> {
    match name {
        "assert" | "assertTrue" => {
            let condition = args.first().map(truthy).unwrap_or(false);
            if !condition {
                return fail(&format!("assertion failed{}", detail(args.get(1))));
            }
            Ok(Value::Void)
        }
        "assertFalse" => {
            let condition = args.first().map(truthy).unwrap_or(false);
            if condition {
                return fail(&format!("expected false{}", detail(args.get(1))));
            }
            Ok(Value::Void)
        }
        "assertNull" => {
            let is_null = args.first().map(is_null_like).unwrap_or(true);
            if !is_null {
                return fail(&format!("expected null{}", detail(args.get(1))));
            }
            Ok(Value::Void)
        }
        "assertNotNull" => {
            let is_null = args.first().map(is_null_like).unwrap_or(true);
            if is_null {
                return fail(&format!("expected non-null{}", detail(args.get(1))));
            }
            Ok(Value::Void)
        }
        "assertEquals" => {
            let (left, right) = pair(args)?;
            if !values_equal(left, right) {
                return fail(&format!(
                    "expected {} but was {}{}",
                    left.to_display_string(),
                    right.to_display_string(),
                    detail(args.get(2))
                ));
            }
            Ok(Value::Void)
        }
        "assertNotEquals" => {
            let (left, right) = pair(args)?;
            if values_equal(left, right) {
                return fail(&format!(
                    "values should differ: {}{}",
                    left.to_display_string(),
                    detail(args.get(2))
                ));
            }
            Ok(Value::Void)
        }
        "assertSame" => {
            let (left, right) = pair(args)?;
            if !same_object(left, right) {
                return fail(&format!("expected same object{}", detail(args.get(2))));
            }
            Ok(Value::Void)
        }
        "assertNotSame" => {
            let (left, right) = pair(args)?;
            if same_object(left, right) {
                return fail(&format!(
                    "expected different objects{}",
                    detail(args.get(2))
                ));
            }
            Ok(Value::Void)
        }
        "assertArrayEquals" => {
            let (left, right) = pair(args)?;
            if !arrays_equal(left, right) {
                return fail(&format!(
                    "arrays differ: {} vs {}{}",
                    left.to_display_string(),
                    right.to_display_string(),
                    detail(args.get(2))
                ));
            }
            Ok(Value::Void)
        }
        "fail" => fail(&format!("assertion failed{}", detail(args.first()))),
        _ => Err(Error::eval(format!("Unknown assertion {}", name))),
    }
}

fn truthy(value: &Value) -> bool {
    match value {
        Value::Bool(b) => *b,
        Value::Null | Value::Void => false,
        _ => true,
    }
}

fn is_null_like(value: &Value) -> bool {
    matches!(value, Value::Null | Value::Void)
}

fn pair(args: &[Value]) -> Result<(&Value, &Value)> {
    match args {
        [left, right, ..] => Ok((left, right)),
        _ => Err(Error::eval("assertion requires two arguments")),
    }
}

fn detail(message: Option<&Value>) -> String {
    match message {
        Some(Value::Str(text)) => format!(": {}", text),
        Some(other) => format!(": {}", other.to_display_string()),
        None => String::new(),
    }
}

fn fail(message: &str) -> Result<Value> {
    Err(Error::eval(format!("AssertionError: {}", message)))
}

fn numeric(value: &Value) -> Option<f64> {
    match value {
        Value::Byte(_)
        | Value::Short(_)
        | Value::Int(_)
        | Value::Long(_)
        | Value::Float(_)
        | Value::Double(_)
        | Value::Char(_) => value.as_f64().ok(),
        _ => None,
    }
}

pub fn values_equal(left: &Value, right: &Value) -> bool {
    if let (Some(a), Some(b)) = (numeric(left), numeric(right)) {
        return a == b;
    }
    match (left, right) {
        (Value::Str(a), Value::Str(b)) => a == b,
        (Value::Bool(a), Value::Bool(b)) => a == b,
        (Value::Null | Value::Void, Value::Null | Value::Void) => true,
        (Value::Array(_), Value::Array(_)) => arrays_equal(left, right),
        (Value::Java(a), Value::Java(b)) => std::sync::Arc::ptr_eq(a, b),
        _ => left.to_display_string() == right.to_display_string(),
    }
}

pub fn arrays_equal(left: &Value, right: &Value) -> bool {
    match (left, right) {
        (Value::Array(a), Value::Array(b)) => {
            let a = a.borrow();
            let b = b.borrow();
            a.items.len() == b.items.len()
                && a.items
                    .iter()
                    .zip(b.items.iter())
                    .all(|(x, y)| values_equal(x, y))
        }
        _ => values_equal(left, right),
    }
}

fn same_object(left: &Value, right: &Value) -> bool {
    match (left, right) {
        (Value::Java(a), Value::Java(b)) => std::sync::Arc::ptr_eq(a, b),
        (Value::Namespace(a), Value::Namespace(b)) => std::rc::Rc::ptr_eq(a, b),
        _ => values_equal(left, right),
    }
}
