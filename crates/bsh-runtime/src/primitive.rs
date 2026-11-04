//! Java-like primitive semantics: promotion, arithmetic and formatting.

use crate::error::{Error, Result};
use crate::value::Value;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum PrimKind {
    Byte,
    Short,
    Char,
    Int,
    Long,
    Float,
    Double,
    Boolean,
}

impl PrimKind {
    pub fn from_ast(kind: bsh_syntax::ast::PrimKind) -> PrimKind {
        match kind {
            bsh_syntax::ast::PrimKind::Boolean => PrimKind::Boolean,
            bsh_syntax::ast::PrimKind::Byte => PrimKind::Byte,
            bsh_syntax::ast::PrimKind::Char => PrimKind::Char,
            bsh_syntax::ast::PrimKind::Short => PrimKind::Short,
            bsh_syntax::ast::PrimKind::Int => PrimKind::Int,
            bsh_syntax::ast::PrimKind::Long => PrimKind::Long,
            bsh_syntax::ast::PrimKind::Float => PrimKind::Float,
            bsh_syntax::ast::PrimKind::Double => PrimKind::Double,
            bsh_syntax::ast::PrimKind::Void => PrimKind::Double,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            PrimKind::Byte => "byte",
            PrimKind::Short => "short",
            PrimKind::Char => "char",
            PrimKind::Int => "int",
            PrimKind::Long => "long",
            PrimKind::Float => "float",
            PrimKind::Double => "double",
            PrimKind::Boolean => "boolean",
        }
    }

    pub fn rank(self) -> u8 {
        match self {
            PrimKind::Byte => 0,
            PrimKind::Short => 1,
            PrimKind::Char => 2,
            PrimKind::Int => 3,
            PrimKind::Long => 4,
            PrimKind::Float => 5,
            PrimKind::Double => 6,
            PrimKind::Boolean => 7,
        }
    }

    /// Binary numeric promotion (JLS 5.6.2), simplified for BeanShell.
    pub fn promote(a: PrimKind, b: PrimKind) -> PrimKind {
        if a == PrimKind::Double || b == PrimKind::Double {
            PrimKind::Double
        } else if a == PrimKind::Float || b == PrimKind::Float {
            PrimKind::Float
        } else if a == PrimKind::Long || b == PrimKind::Long {
            PrimKind::Long
        } else {
            PrimKind::Int
        }
    }
}

pub fn wrap_int(v: i64) -> Value {
    Value::Int(v as i32)
}

pub fn negate(v: &Value) -> Result<Value> {
    Ok(match v {
        Value::Byte(x) => Value::Int(-(*x as i32)),
        Value::Short(x) => Value::Int(-(*x as i32)),
        Value::Int(x) => Value::Int(x.wrapping_neg()),
        Value::Long(x) => Value::Long(x.wrapping_neg()),
        Value::Float(x) => Value::Float(-x),
        Value::Double(x) => Value::Double(-x),
        Value::Char(c) => Value::Int(-(*c as i32)),
        other => return Err(Error::eval(format!("Unary minus on {}", other.type_name()))),
    })
}

pub fn bit_not(v: &Value) -> Result<Value> {
    Ok(match v {
        Value::Byte(x) => Value::Int(!(*x as i32)),
        Value::Short(x) => Value::Int(!(*x as i32)),
        Value::Int(x) => Value::Int(!x),
        Value::Long(x) => Value::Long(!x),
        Value::Char(c) => Value::Int(!(*c as i32)),
        other => return Err(Error::eval(format!("Unary ~ on {}", other.type_name()))),
    })
}

pub fn add(a: &Value, b: &Value) -> Result<Value> {
    arithmetic(a, b, '+')
}

pub fn sub(a: &Value, b: &Value) -> Result<Value> {
    arithmetic(a, b, '-')
}

pub fn mul(a: &Value, b: &Value) -> Result<Value> {
    arithmetic(a, b, '*')
}

fn arithmetic(a: &Value, b: &Value, op: char) -> Result<Value> {
    let ka = a.prim_kind();
    let kb = b.prim_kind();
    let (ka, kb) = match (ka, kb) {
        (Some(x), Some(y)) => (x, y),
        _ => {
            return Err(Error::eval(format!(
                "Non numeric operands for '{}': {} and {}",
                op,
                a.type_name(),
                b.type_name()
            )))
        }
    };
    let kind = PrimKind::promote(ka, kb);
    Ok(match kind {
        PrimKind::Double => {
            let (x, y) = (a.as_f64()?, b.as_f64()?);
            Value::Double(match op {
                '+' => x + y,
                '-' => x - y,
                _ => x * y,
            })
        }
        PrimKind::Float => {
            let (x, y) = (a.as_f64()? as f32, b.as_f64()? as f32);
            Value::Float(match op {
                '+' => x + y,
                '-' => x - y,
                _ => x * y,
            })
        }
        PrimKind::Long => {
            let (x, y) = (a.as_i64()?, b.as_i64()?);
            Value::Long(match op {
                '+' => x.wrapping_add(y),
                '-' => x.wrapping_sub(y),
                _ => x.wrapping_mul(y),
            })
        }
        _ => {
            let (x, y) = (a.as_i64()? as i32, b.as_i64()? as i32);
            Value::Int(match op {
                '+' => x.wrapping_add(y),
                '-' => x.wrapping_sub(y),
                _ => x.wrapping_mul(y),
            })
        }
    })
}

pub fn div(a: &Value, b: &Value) -> Result<Value> {
    let (ka, kb) = numeric_kinds(a, b, "/")?;
    let kind = PrimKind::promote(ka, kb);
    Ok(match kind {
        PrimKind::Double => {
            let (x, y) = (a.as_f64()?, b.as_f64()?);
            Value::Double(x / y)
        }
        PrimKind::Float => {
            let (x, y) = (a.as_f64()? as f32, b.as_f64()? as f32);
            Value::Float(x / y)
        }
        PrimKind::Long => {
            let (x, y) = (a.as_i64()?, b.as_i64()?);
            if y == 0 {
                return Err(Error::eval("Division by zero"));
            }
            Value::Long(x.wrapping_div(y))
        }
        _ => {
            let (x, y) = (a.as_i64()? as i32, b.as_i64()? as i32);
            if y == 0 {
                return Err(Error::eval("Division by zero"));
            }
            Value::Int(x.wrapping_div(y))
        }
    })
}

pub fn rem(a: &Value, b: &Value) -> Result<Value> {
    let (ka, kb) = numeric_kinds(a, b, "%")?;
    let kind = PrimKind::promote(ka, kb);
    Ok(match kind {
        PrimKind::Double => {
            let (x, y) = (a.as_f64()?, b.as_f64()?);
            Value::Double(x % y)
        }
        PrimKind::Float => {
            let (x, y) = (a.as_f64()? as f32, b.as_f64()? as f32);
            Value::Float(x % y)
        }
        PrimKind::Long => {
            let (x, y) = (a.as_i64()?, b.as_i64()?);
            if y == 0 {
                return Err(Error::eval("Division by zero"));
            }
            Value::Long(x.wrapping_rem(y))
        }
        _ => {
            let (x, y) = (a.as_i64()? as i32, b.as_i64()? as i32);
            if y == 0 {
                return Err(Error::eval("Division by zero"));
            }
            Value::Int(x.wrapping_rem(y))
        }
    })
}

pub fn pow(a: &Value, b: &Value) -> Result<Value> {
    let (ka, kb) = numeric_kinds(a, b, "**")?;
    if matches!(ka, PrimKind::Double | PrimKind::Float)
        || matches!(kb, PrimKind::Double | PrimKind::Float)
    {
        let v = a.as_f64()?.powf(b.as_f64()?);
        return Ok(Value::Double(v));
    }
    let base = a.as_i64()?;
    let exp = b.as_i64()?;
    if exp < 0 {
        return Ok(Value::Double((base as f64).powf(exp as f64)));
    }
    let mut acc: i64 = 1;
    let mut e = exp as u32;
    let mut b2 = base;
    while e > 0 {
        if e & 1 == 1 {
            acc = acc.wrapping_mul(b2);
        }
        b2 = b2.wrapping_mul(b2);
        e >>= 1;
    }
    Ok(Value::Long(acc))
}

fn numeric_kinds(a: &Value, b: &Value, op: &str) -> Result<(PrimKind, PrimKind)> {
    match (a.prim_kind(), b.prim_kind()) {
        (Some(x), Some(y)) => Ok((x, y)),
        _ => Err(Error::eval(format!(
            "Non numeric operands for '{}': {} and {}",
            op,
            a.type_name(),
            b.type_name()
        ))),
    }
}

pub fn compare(a: &Value, b: &Value) -> Result<i32> {
    match (a, b) {
        (Value::Str(x), Value::Str(y)) => Ok(match x.as_ref().cmp(y.as_ref()) {
            std::cmp::Ordering::Less => -1,
            std::cmp::Ordering::Equal => 0,
            std::cmp::Ordering::Greater => 1,
        }),
        (Value::Bool(x), Value::Bool(y)) => Ok((*x as i32) - (*y as i32)),
        _ => {
            if a.is_numeric() && b.is_numeric() {
                let (ka, kb) = (a.prim_kind().unwrap(), b.prim_kind().unwrap());
                let kind = PrimKind::promote(ka, kb);
                if kind == PrimKind::Double || kind == PrimKind::Float {
                    let (x, y) = (a.as_f64()?, b.as_f64()?);
                    Ok(if x < y {
                        -1
                    } else if x > y {
                        1
                    } else {
                        0
                    })
                } else {
                    let (x, y) = (a.as_i64()?, b.as_i64()?);
                    Ok(if x < y {
                        -1
                    } else if x > y {
                        1
                    } else {
                        0
                    })
                }
            } else {
                Err(Error::eval(format!(
                    "Cannot compare {} with {}",
                    a.type_name(),
                    b.type_name()
                )))
            }
        }
    }
}

pub fn bit_op(a: &Value, b: &Value, op: char) -> Result<Value> {
    let (ka, kb) = numeric_kinds(a, b, "bitwise")?;
    let long = ka == PrimKind::Long || kb == PrimKind::Long;
    if long {
        let (x, y) = (a.as_i64()?, b.as_i64()?);
        Ok(Value::Long(match op {
            '&' => x & y,
            '|' => x | y,
            _ => x ^ y,
        }))
    } else {
        let (x, y) = (a.as_i64()? as i32, b.as_i64()? as i32);
        Ok(Value::Int(match op {
            '&' => x & y,
            '|' => x | y,
            _ => x ^ y,
        }))
    }
}

pub fn shift(a: &Value, b: &Value, op: &str) -> Result<Value> {
    let kind = a
        .prim_kind()
        .ok_or_else(|| Error::eval(format!("Cannot shift {}", a.type_name())))?;
    let long = kind == PrimKind::Long;
    let left = a.as_i64()?;
    let amount = b.as_i64()?;
    Ok(if long {
        let n = (amount & 63) as u32;
        Value::Long(match op {
            "<<" => left.wrapping_shl(n),
            ">>" => left.wrapping_shr(n),
            _ => ((left as u64).wrapping_shr(n)) as i64,
        })
    } else {
        let left = left as i32;
        let n = (amount & 31) as u32;
        Value::Int(match op {
            "<<" => left.wrapping_shl(n),
            ">>" => left.wrapping_shr(n),
            _ => ((left as u32).wrapping_shr(n)) as i32,
        })
    })
}

pub fn logic(a: &Value, b: &Value, op: char) -> Result<Value> {
    match (a, b) {
        (Value::Bool(x), Value::Bool(y)) => Ok(Value::Bool(match op {
            '&' => *x & *y,
            '|' => *x | *y,
            _ => *x ^ *y,
        })),
        _ => {
            let (ka, kb) = numeric_kinds(a, b, "bitwise")?;
            let long = ka == PrimKind::Long || kb == PrimKind::Long;
            if long {
                let (x, y) = (a.as_i64()?, b.as_i64()?);
                Ok(Value::Long(match op {
                    '&' => x & y,
                    '|' => x | y,
                    _ => x ^ y,
                }))
            } else {
                let (x, y) = (a.as_i64()? as i32, b.as_i64()? as i32);
                Ok(Value::Int(match op {
                    '&' => x & y,
                    '|' => x | y,
                    _ => x ^ y,
                }))
            }
        }
    }
}

pub fn not(v: &Value) -> Result<Value> {
    match v {
        Value::Bool(b) => Ok(Value::Bool(!b)),
        other => Err(Error::eval(format!(
            "'!' on non boolean {}",
            other.type_name()
        ))),
    }
}

/// Convert a value to the given primitive kind, truncating like a Java cast.
pub fn cast_primitive(v: &Value, kind: PrimKind) -> Result<Value> {
    if kind == PrimKind::Boolean {
        return match v {
            Value::Bool(_) => Ok(v.clone()),
            Value::Str(s) if s.as_ref() == "true" || s.as_ref() == "false" => {
                Ok(Value::Bool(s.as_ref() == "true"))
            }
            other => Err(Error::eval(format!(
                "Cannot cast {} to boolean",
                other.type_name()
            ))),
        };
    }
    if matches!(v, Value::Bool(_)) {
        return Err(Error::eval("Cannot cast boolean to a number"));
    }
    if !v.is_numeric() {
        if let Value::Str(s) = v {
            // BeanShell allows numeric strings in casts.
            if let Ok(d) = s.trim().parse::<f64>() {
                return cast_number(d, kind);
            }
        }
        return Err(Error::eval(format!(
            "Cannot cast {} to {}",
            v.type_name(),
            kind.name()
        )));
    }
    let n = v.as_i64()?;
    let range_err = || {
        Error::eval(format!(
            "cannot assign number {} to type {}",
            n,
            kind.name()
        ))
    };
    match kind {
        PrimKind::Byte => {
            if n < i8::MIN as i64 || n > i8::MAX as i64 {
                return Err(range_err());
            }
            Ok(Value::Byte(n as i8))
        }
        PrimKind::Short => {
            if n < i16::MIN as i64 || n > i16::MAX as i64 {
                return Err(range_err());
            }
            Ok(Value::Short(n as i16))
        }
        PrimKind::Char => {
            if n < 0 || n > u16::MAX as i64 {
                return Err(Error::eval(format!(
                    "cannot assign number {} to type char",
                    n
                )));
            }
            Ok(Value::Char(char::from_u32(n as u32).unwrap_or('\u{0}')))
        }
        PrimKind::Int => {
            if n < i32::MIN as i64 || n > i32::MAX as i64 {
                return Err(range_err());
            }
            Ok(Value::Int(n as i32))
        }
        PrimKind::Long => Ok(Value::Long(n)),
        PrimKind::Float => Ok(Value::Float(v.as_f64()? as f32)),
        PrimKind::Double => Ok(Value::Double(v.as_f64()?)),
        PrimKind::Boolean => unreachable!(),
    }
}

fn cast_number(d: f64, kind: PrimKind) -> Result<Value> {
    Ok(match kind {
        PrimKind::Byte => Value::Byte(d as i8),
        PrimKind::Short => Value::Short(d as i16),
        PrimKind::Char => {
            Value::Char(char::from_u32((d as i64 as u32) & 0xffff).unwrap_or('\u{0}'))
        }
        PrimKind::Int => Value::Int(d as i32),
        PrimKind::Long => Value::Long(d as i64),
        PrimKind::Float => Value::Float(d as f32),
        PrimKind::Double => Value::Double(d),
        PrimKind::Boolean => return Err(Error::eval("Cannot cast number to boolean")),
    })
}

/// Java-ish floating point formatting.
pub fn format_float(v: f64, is_float: bool) -> String {
    let _ = is_float;
    if v.is_nan() {
        return "NaN".to_string();
    }
    if v.is_infinite() {
        return if v > 0.0 {
            "Infinity".to_string()
        } else {
            "-Infinity".to_string()
        };
    }
    if v == 0.0 {
        return if v.is_sign_negative() {
            "-0.0".to_string()
        } else {
            "0.0".to_string()
        };
    }
    let a = v.abs();
    if a >= 1e7 || a < 1e-3 {
        let s = format!("{:e}", v);
        let (mantissa, exp) = s.split_once('e').unwrap_or((s.as_str(), "0"));
        let mantissa = if mantissa.contains('.') {
            mantissa.to_string()
        } else {
            format!("{}.0", mantissa)
        };
        let exp = exp.trim_start_matches('+');
        format!("{}E{}", mantissa, exp)
    } else {
        let s = format!("{}", v);
        if s.contains('.') {
            s
        } else {
            format!("{}.0", s)
        }
    }
}
