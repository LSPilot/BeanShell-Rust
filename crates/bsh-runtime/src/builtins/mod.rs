//! Host classes and functions available to scripts.
//!
//! A small, self contained model of the Java types that BeanShell scripts use
//! most often: strings, the math and system classes, the common collections and
//! the exception hierarchy.

use crate::class::ClassValue;
use crate::error::{Error, Result};
use crate::interp::{Interpreter, Ns};
use crate::primitive;
use crate::value::{ArrayValue, HostObject, ObjectValue, Value};
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

mod assertions;

// --------------------------------------------------------------------- host

#[derive(Debug)]
pub struct ListObj {
    pub items: Vec<Value>,
    pub kind: &'static str,
}

#[derive(Debug)]
pub struct MapObj {
    pub entries: Vec<(Value, Value)>,
    pub kind: &'static str,
}

#[derive(Debug, Clone)]
pub struct BuilderObj {
    pub buf: Rc<RefCell<String>>,
}

#[derive(Debug)]
pub struct PrintStreamObj {
    pub err: bool,
}

impl HostObject for ListObj {
    fn class_name(&self) -> &'static str {
        self.kind
    }

    fn call_method(
        &mut self,
        interp: &mut Interpreter,
        name: &str,
        args: Vec<Value>,
    ) -> Result<Value> {
        let _ = interp;
        match name {
            "add" if args.len() == 1 => {
                self.items.push(args[0].clone());
                Ok(Value::Bool(true))
            }
            "add" if args.len() == 2 => {
                let i = args[0].as_i64()? as usize;
                if i > self.items.len() {
                    return Err(Error::eval("Index out of bounds"));
                }
                self.items.insert(i, args[1].clone());
                Ok(Value::Void)
            }
            "addAll" if args.len() == 1 => {
                let items = interp.iterate(&args[0])?;
                self.items.extend(items);
                Ok(Value::Bool(true))
            }
            "get" if args.len() == 1 => {
                let i = args[0].as_i64()?;
                self.items
                    .get(i as usize)
                    .cloned()
                    .ok_or_else(|| Error::eval(format!("Index out of bounds: {}", i)))
            }
            "set" if args.len() == 2 => {
                let i = args[0].as_i64()?;
                match self.items.get_mut(i as usize) {
                    Some(slot) => {
                        let old = slot.clone();
                        *slot = args[1].clone();
                        Ok(old)
                    }
                    None => Err(Error::eval(format!("Index out of bounds: {}", i))),
                }
            }
            "size" => Ok(Value::Int(self.items.len() as i32)),
            "isEmpty" => Ok(Value::Bool(self.items.is_empty())),
            "clear" => {
                self.items.clear();
                Ok(Value::Void)
            }
            "contains" if args.len() == 1 => {
                Ok(Value::Bool(self.items.iter().any(|v| v.equality(&args[0]))))
            }
            "indexOf" if args.len() == 1 => {
                let pos = self.items.iter().position(|v| v.equality(&args[0]));
                Ok(Value::Int(pos.map(|p| p as i32).unwrap_or(-1)))
            }
            "remove" if args.len() == 1 => {
                if let Some(i) = self.items.iter().position(|v| v.equality(&args[0])) {
                    Ok(self.items.remove(i))
                } else {
                    Ok(Value::Bool(false))
                }
            }
            "remove" if args.len() == 1 => Ok(Value::Bool(false)),
            "toArray" if args.is_empty() => Ok(Value::Array(Rc::new(RefCell::new(ArrayValue {
                dims: 1,
                component: None,
                items: self.items.clone(),
            })))),
            "iterator" | "stream" => Err(Error::eval("Iterators are not supported yet")),
            "toString" => Ok(Value::string(self.display())),
            "subList" if args.len() == 2 => {
                let from = args[0].as_i64()? as usize;
                let to = (args[1].as_i64()? as usize).min(self.items.len());
                let items = self.items[from.min(to)..to].to_vec();
                Ok(Value::Builtin(Rc::new(RefCell::new(ListObj {
                    items,
                    kind: "ArrayList",
                }))))
            }
            "sort" => {
                self.items.sort_by(|a, b| {
                    primitive::compare(a, b)
                        .map(|c| c.cmp(&0))
                        .unwrap_or(std::cmp::Ordering::Equal)
                });
                Ok(Value::Void)
            }
            other => Err(Error::eval(format!(
                "No such method: {}() on {}",
                other, self.kind
            ))),
        }
    }

    fn display(&self) -> String {
        let inner: Vec<String> = self.items.iter().map(|v| display_java(v)).collect();
        format!("[{}]", inner.join(", "))
    }
}

impl HostObject for MapObj {
    fn class_name(&self) -> &'static str {
        self.kind
    }

    fn call_method(
        &mut self,
        interp: &mut Interpreter,
        name: &str,
        args: Vec<Value>,
    ) -> Result<Value> {
        let _ = interp;
        match name {
            "put" if args.len() == 2 => {
                if let Some(slot) = self.entries.iter_mut().find(|(k, _)| k.equality(&args[0])) {
                    let old = slot.1.clone();
                    slot.1 = args[1].clone();
                    Ok(old)
                } else {
                    self.entries.push((args[0].clone(), args[1].clone()));
                    Ok(Value::Null)
                }
            }
            "get" if args.len() == 1 => Ok(self
                .entries
                .iter()
                .find(|(k, _)| k.equality(&args[0]))
                .map(|(_, v)| v.clone())
                .unwrap_or(Value::Null)),
            "containsKey" if args.len() == 1 => Ok(Value::Bool(
                self.entries.iter().any(|(k, _)| k.equality(&args[0])),
            )),
            "containsValue" if args.len() == 1 => Ok(Value::Bool(
                self.entries.iter().any(|(_, v)| v.equality(&args[0])),
            )),
            "remove" if args.len() == 1 => {
                if let Some(i) = self.entries.iter().position(|(k, _)| k.equality(&args[0])) {
                    Ok(self.entries.remove(i).1)
                } else {
                    Ok(Value::Null)
                }
            }
            "size" => Ok(Value::Int(self.entries.len() as i32)),
            "isEmpty" => Ok(Value::Bool(self.entries.is_empty())),
            "clear" => {
                self.entries.clear();
                Ok(Value::Void)
            }
            "keySet" => {
                let items: Vec<Value> = self.entries.iter().map(|(k, _)| k.clone()).collect();
                Ok(Value::Builtin(Rc::new(RefCell::new(ListObj {
                    items,
                    kind: "HashSet",
                }))))
            }
            "values" => {
                let items: Vec<Value> = self.entries.iter().map(|(_, v)| v.clone()).collect();
                Ok(Value::Builtin(Rc::new(RefCell::new(ListObj {
                    items,
                    kind: "ArrayList",
                }))))
            }
            "toString" => Ok(Value::string(self.display())),
            other => Err(Error::eval(format!(
                "No such method: {}() on {}",
                other, self.kind
            ))),
        }
    }

    fn display(&self) -> String {
        let inner: Vec<String> = self
            .entries
            .iter()
            .map(|(k, v)| format!("{}={}", display_java(k), display_java(v)))
            .collect();
        format!("{{{}}}", inner.join(", "))
    }
}

impl HostObject for BuilderObj {
    fn class_name(&self) -> &'static str {
        "StringBuilder"
    }

    fn call_method(
        &mut self,
        interp: &mut Interpreter,
        name: &str,
        args: Vec<Value>,
    ) -> Result<Value> {
        let _ = interp;
        match name {
            "append" => {
                {
                    let mut buf = self.buf.borrow_mut();
                    for a in &args {
                        buf.push_str(&display_java(a));
                    }
                }
                Ok(Value::Builtin(Rc::new(RefCell::new(BuilderObj {
                    buf: self.buf.clone(),
                }))))
            }
            "toString" => Ok(Value::string(self.buf.borrow().clone())),
            "length" => Ok(Value::Int(self.buf.borrow().chars().count() as i32)),
            "charAt" if args.len() == 1 => {
                let i = args[0].as_i64()? as usize;
                self.buf
                    .borrow()
                    .chars()
                    .nth(i)
                    .map(Value::Char)
                    .ok_or_else(|| Error::eval("String index out of range"))
            }
            "reverse" => {
                let reversed: String = self.buf.borrow().chars().rev().collect();
                *self.buf.borrow_mut() = reversed.clone();
                Ok(Value::string(reversed))
            }
            other => Err(Error::eval(format!(
                "No such method: {}() on StringBuilder",
                other
            ))),
        }
    }

    fn display(&self) -> String {
        self.buf.borrow().clone()
    }
}

impl HostObject for PrintStreamObj {
    fn class_name(&self) -> &'static str {
        "PrintStream"
    }

    fn call_method(
        &mut self,
        interp: &mut Interpreter,
        name: &str,
        args: Vec<Value>,
    ) -> Result<Value> {
        match name {
            "println" => {
                let text: String = args.iter().map(display_java).collect::<Vec<_>>().join("");
                interp.write_line(&text);
                Ok(Value::Void)
            }
            "print" => {
                let text: String = args.iter().map(display_java).collect::<Vec<_>>().join("");
                interp.write_out(&text);
                Ok(Value::Void)
            }
            "flush" | "close" => Ok(Value::Void),
            other => Err(Error::eval(format!(
                "No such method: {}() on PrintStream",
                other
            ))),
        }
    }
}

pub fn display_java(v: &Value) -> String {
    match v {
        Value::Str(s) => s.to_string(),
        other => other.to_display_string(),
    }
}

// ------------------------------------------------------------- registration

pub fn install_classes(interp: &mut Interpreter) {
    let mut register =
        |name: &'static str, super_name: Option<&'static str>, builtin: Option<&'static str>| {
            let sup = super_name.and_then(|s| interp.classes.get(s).cloned());
            let class = Rc::new(ClassValue::new_builtin(name, builtin.unwrap_or(""), sup));
            let simple = name.rsplit('.').next().unwrap();
            interp.classes.insert(name.to_string(), class.clone());
            interp
                .global
                .borrow_mut()
                .classes
                .insert(simple.to_string(), class);
        };
    register("java.lang.Object", None, None);
    register("java.lang.String", Some("java.lang.Object"), Some("string"));
    register("java.lang.Class", Some("java.lang.Object"), None);
    register("java.lang.Number", Some("java.lang.Object"), Some("number"));
    register("java.lang.Integer", Some("java.lang.Number"), Some("int"));
    register("java.lang.Long", Some("java.lang.Number"), Some("long"));
    register("java.lang.Double", Some("java.lang.Number"), Some("double"));
    register("java.lang.Float", Some("java.lang.Number"), Some("float"));
    register("java.lang.Short", Some("java.lang.Number"), Some("short"));
    register("java.lang.Byte", Some("java.lang.Number"), Some("byte"));
    register(
        "java.lang.Character",
        Some("java.lang.Object"),
        Some("char"),
    );
    register(
        "java.lang.Boolean",
        Some("java.lang.Object"),
        Some("boolean"),
    );
    register("java.lang.Math", Some("java.lang.Object"), None);
    register("java.lang.System", Some("java.lang.Object"), None);
    register(
        "java.lang.StringBuilder",
        Some("java.lang.Object"),
        Some("builder"),
    );
    register(
        "java.lang.StringBuffer",
        Some("java.lang.Object"),
        Some("builder"),
    );
    register(
        "java.io.PrintStream",
        Some("java.lang.Object"),
        Some("printstream"),
    );
    register("java.io.InputStream", Some("java.lang.Object"), None);
    register(
        "java.lang.Throwable",
        Some("java.lang.Object"),
        Some("exception"),
    );
    register(
        "java.lang.Exception",
        Some("java.lang.Throwable"),
        Some("exception"),
    );
    register(
        "java.lang.RuntimeException",
        Some("java.lang.Exception"),
        Some("exception"),
    );
    register(
        "java.lang.Error",
        Some("java.lang.Throwable"),
        Some("exception"),
    );
    register(
        "java.lang.IllegalArgumentException",
        Some("java.lang.RuntimeException"),
        Some("exception"),
    );
    register(
        "java.lang.IllegalStateException",
        Some("java.lang.RuntimeException"),
        Some("exception"),
    );
    register(
        "java.lang.NullPointerException",
        Some("java.lang.RuntimeException"),
        Some("exception"),
    );
    register(
        "java.lang.ArithmeticException",
        Some("java.lang.RuntimeException"),
        Some("exception"),
    );
    register(
        "java.lang.ClassCastException",
        Some("java.lang.RuntimeException"),
        Some("exception"),
    );
    register(
        "java.lang.NumberFormatException",
        Some("java.lang.IllegalArgumentException"),
        Some("exception"),
    );
    register(
        "java.lang.IndexOutOfBoundsException",
        Some("java.lang.RuntimeException"),
        Some("exception"),
    );
    register(
        "java.lang.ArrayIndexOutOfBoundsException",
        Some("java.lang.IndexOutOfBoundsException"),
        Some("exception"),
    );
    register(
        "java.lang.StringIndexOutOfBoundsException",
        Some("java.lang.IndexOutOfBoundsException"),
        Some("exception"),
    );
    register(
        "java.lang.UnsupportedOperationException",
        Some("java.lang.RuntimeException"),
        Some("exception"),
    );
    register(
        "java.io.IOException",
        Some("java.lang.Exception"),
        Some("exception"),
    );
    register(
        "bsh.EvalError",
        Some("java.lang.Exception"),
        Some("exception"),
    );
    register("bsh.TargetError", Some("bsh.EvalError"), Some("exception"));

    register("java.lang.Iterable", Some("java.lang.Object"), None);
    register("java.util.Collection", Some("java.lang.Iterable"), None);
    register("java.util.List", Some("java.util.Collection"), None);
    register("java.util.Set", Some("java.util.Collection"), None);
    register("java.util.Queue", Some("java.util.Collection"), None);
    register("java.util.Deque", Some("java.util.Queue"), None);
    register("java.util.Map", Some("java.lang.Object"), None);
    register("java.util.ArrayList", Some("java.util.List"), Some("list"));
    register("java.util.LinkedList", Some("java.util.List"), Some("list"));
    register("java.util.Vector", Some("java.util.List"), Some("list"));
    register("java.util.Stack", Some("java.util.Vector"), Some("list"));
    register("java.util.HashMap", Some("java.util.Map"), Some("map"));
    register(
        "java.util.LinkedHashMap",
        Some("java.util.HashMap"),
        Some("map"),
    );
    register("java.util.TreeMap", Some("java.util.Map"), Some("map"));
    register("java.util.HashSet", Some("java.util.Set"), Some("set"));
    register(
        "java.util.LinkedHashSet",
        Some("java.util.Set"),
        Some("set"),
    );
    register("java.util.TreeSet", Some("java.util.Set"), Some("set"));
    register("java.util.Arrays", Some("java.lang.Object"), None);
    register("java.util.Collections", Some("java.lang.Object"), None);
    register("java.lang.Thread", Some("java.lang.Object"), None);
    register("java.lang.Runnable", Some("java.lang.Object"), None);
    register("java.lang.Comparable", Some("java.lang.Object"), None);
    register("java.lang.CharSequence", Some("java.lang.Object"), None);

    // Static fields on System.
    if let Some(system) = interp.classes.get("java.lang.System") {
        system.statics.borrow_mut().insert(
            "out".to_string(),
            Value::Builtin(Rc::new(RefCell::new(PrintStreamObj { err: false }))),
        );
        system.statics.borrow_mut().insert(
            "err".to_string(),
            Value::Builtin(Rc::new(RefCell::new(PrintStreamObj { err: true }))),
        );
    }
    install_numeric_constants(interp);
}

fn install_numeric_constants(interp: &mut Interpreter) {
    let put = |class: &str, name: &str, v: Value| {
        if let Some(c) = interp.classes.get(class) {
            c.statics.borrow_mut().insert(name.to_string(), v);
        }
    };
    put("java.lang.Integer", "MAX_VALUE", Value::Int(i32::MAX));
    put("java.lang.Integer", "MIN_VALUE", Value::Int(i32::MIN));
    put("java.lang.Integer", "SIZE", Value::Int(32));
    put("java.lang.Long", "MAX_VALUE", Value::Long(i64::MAX));
    put("java.lang.Long", "MIN_VALUE", Value::Long(i64::MIN));
    put("java.lang.Short", "MAX_VALUE", Value::Short(i16::MAX));
    put("java.lang.Short", "MIN_VALUE", Value::Short(i16::MIN));
    put("java.lang.Byte", "MAX_VALUE", Value::Byte(i8::MAX));
    put("java.lang.Byte", "MIN_VALUE", Value::Byte(i8::MIN));
    put("java.lang.Double", "MAX_VALUE", Value::Double(f64::MAX));
    put(
        "java.lang.Double",
        "MIN_VALUE",
        Value::Double(f64::MIN_POSITIVE),
    );
    put("java.lang.Double", "NaN", Value::Double(f64::NAN));
    put(
        "java.lang.Double",
        "POSITIVE_INFINITY",
        Value::Double(f64::INFINITY),
    );
    put(
        "java.lang.Double",
        "NEGATIVE_INFINITY",
        Value::Double(f64::NEG_INFINITY),
    );
    put("java.lang.Float", "MAX_VALUE", Value::Float(f32::MAX));
    put("java.lang.Float", "NaN", Value::Float(f32::NAN));
    put("java.lang.Boolean", "TRUE", Value::Bool(true));
    put("java.lang.Boolean", "FALSE", Value::Bool(false));
    put("java.lang.Character", "MAX_VALUE", Value::Char('\u{ffff}'));
    put("java.lang.Character", "MIN_VALUE", Value::Char('\0'));
}

pub fn is_builtin_a(class_name: &str, target: &str) -> bool {
    let simple = |s: &str| s.rsplit('.').next().unwrap_or(s).to_string();
    let class_simple = simple(class_name);
    let target_simple = simple(target);
    if class_name == target || class_simple == target_simple {
        return true;
    }
    let parents: &[&str] = match class_name {
        "ArrayList" | "LinkedList" | "Vector" | "Stack" => &[
            "List",
            "java.util.List",
            "Collection",
            "java.util.Collection",
            "Iterable",
            "java.lang.Iterable",
            "Object",
            "java.lang.Object",
        ],
        "HashSet" | "LinkedHashSet" | "TreeSet" => &[
            "Set",
            "java.util.Set",
            "Collection",
            "java.util.Collection",
            "Iterable",
            "java.lang.Iterable",
            "Object",
            "java.lang.Object",
        ],
        "HashMap" | "LinkedHashMap" | "TreeMap" => {
            &["Map", "java.util.Map", "Object", "java.lang.Object"]
        }
        "StringBuilder" | "StringBuffer" => &[
            "CharSequence",
            "java.lang.CharSequence",
            "Object",
            "java.lang.Object",
        ],
        "PrintStream" => &["Object", "java.lang.Object"],
        _ => &["Object", "java.lang.Object"],
    };
    parents
        .iter()
        .any(|p| *p == target || *p == target_simple.as_str() || *p == class_simple.as_str())
}

// ------------------------------------------------------------------ statics

pub fn static_field(
    interp: &mut Interpreter,
    class: &Rc<ClassValue>,
    name: &str,
) -> Option<Result<Value>> {
    let _ = interp;
    match (class.name.as_str(), name) {
        ("java.lang.Boolean", "TYPE") => Some(Ok(Value::Class(class.clone()))),
        (_, "TYPE") => Some(Ok(Value::Class(class.clone()))),
        _ => None,
    }
}

pub fn static_call(
    interp: &mut Interpreter,
    class: &Rc<ClassValue>,
    name: &str,
    args: &[Value],
) -> Option<Result<Value>> {
    let result = match class.name.as_str() {
        "java.lang.Math" => Some(math_call(name, args)),
        "java.lang.System" => Some(system_call(interp, name, args)),
        "java.lang.Integer" | "java.lang.Long" | "java.lang.Double" | "java.lang.Float"
        | "java.lang.Short" | "java.lang.Byte" => {
            Some(number_static(class.name.as_str(), name, args))
        }
        "java.lang.Boolean" => Some(bool_static(name, args)),
        "java.lang.Character" => Some(char_static(name, args)),
        "java.lang.String" => Some(string_static(interp, name, args)),
        "java.util.Arrays" => Some(arrays_static(interp, name, args)),
        "java.util.Collections" => Some(collections_static(interp, name, args)),
        _ => None,
    };
    result
}

fn math_call(name: &str, args: &[Value]) -> Result<Value> {
    let a = |i: usize| {
        args.get(i)
            .map(|v| v.as_f64().unwrap_or(f64::NAN))
            .unwrap_or(f64::NAN)
    };
    Ok(match name {
        "abs" => match args.first() {
            Some(Value::Int(v)) => Value::Int(v.abs()),
            Some(Value::Long(v)) => Value::Long(v.abs()),
            Some(Value::Float(v)) => Value::Float(v.abs()),
            Some(Value::Double(v)) => Value::Double(v.abs()),
            _ => Value::Double(a(0).abs()),
        },
        "max" => {
            let c = primitive::compare(&args[0], &args[1])?;
            if c >= 0 {
                args[0].clone()
            } else {
                args[1].clone()
            }
        }
        "min" => {
            let c = primitive::compare(&args[0], &args[1])?;
            if c <= 0 {
                args[0].clone()
            } else {
                args[1].clone()
            }
        }
        "sqrt" => Value::Double(a(0).sqrt()),
        "pow" => Value::Double(a(0).powf(a(1))),
        "floor" => Value::Double(a(0).floor()),
        "ceil" => Value::Double(a(0).ceil()),
        "rint" => Value::Double(a(0).round()),
        "round" => Value::Long(a(0).round() as i64),
        "random" => Value::Double(pseudo_random()),
        "sin" => Value::Double(a(0).sin()),
        "cos" => Value::Double(a(0).cos()),
        "tan" => Value::Double(a(0).tan()),
        "log" => Value::Double(a(0).ln()),
        "log10" => Value::Double(a(0).log10()),
        "exp" => Value::Double(a(0).exp()),
        "toRadians" => Value::Double(a(0).to_radians()),
        "toDegrees" => Value::Double(a(0).to_degrees()),
        "signum" => Value::Double(a(0).signum()),
        "hypot" => Value::Double(a(0).hypot(a(1))),
        "cbrt" => Value::Double(a(0).cbrt()),
        "PI" => Value::Double(std::f64::consts::PI),
        "E" => Value::Double(std::f64::consts::E),
        other => {
            return Err(Error::eval(format!(
                "No such static method: Math.{}()",
                other
            )))
        }
    })
}

fn pseudo_random() -> f64 {
    use std::cell::Cell;
    thread_local! {
        static STATE: Cell<u64> = Cell::new(0x2545F4914F6CDD1D);
    }
    STATE.with(|s| {
        let mut x = s.get();
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        s.set(x);
        (x >> 11) as f64 / (1u64 << 53) as f64
    })
}

fn system_call(interp: &mut Interpreter, name: &str, _args: &[Value]) -> Result<Value> {
    match name {
        "currentTimeMillis" => Ok(Value::Long(
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_millis() as i64)
                .unwrap_or(0),
        )),
        "nanoTime" => Ok(Value::Long(
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos() as i64)
                .unwrap_or(0),
        )),
        "lineSeparator" => Ok(Value::string("\n")),
        "getProperty" => {
            if let Some(jvm) = interp.jvm() {
                let vals = interp.jval_args(_args)?;
                let value = jvm
                    .call_static(interp.runtime_id, "java.lang.System", "getProperty", &vals)
                    .map_err(crate::java::error)?;
                return Ok(crate::java::from_jval(value));
            }
            Ok(Value::Null)
        }
        "exit" => Err(Error::eval("System.exit called")),
        "gc" | "arraycopy" => Ok(Value::Void),
        _ => {
            let _ = interp;
            Err(Error::eval(format!(
                "No such static method: System.{}()",
                name
            )))
        }
    }
}

fn number_static(class: &str, name: &str, args: &[Value]) -> Result<Value> {
    let s = args.first().map(display_java).unwrap_or_default();
    Ok(match name {
        "parseInt" | "parseLong" | "parseShort" | "parseByte" => {
            let radix = args
                .get(1)
                .map(|v| v.as_i64().unwrap_or(10) as u32)
                .unwrap_or(10);
            let t = s.trim();
            let v = i64::from_str_radix(t, radix)
                .or_else(|_| t.parse::<f64>().map(|f| f as i64))
                .map_err(|_| Error::eval(format!("For input string: \"{}\"", t)))?;
            match name {
                "parseInt" => Value::Int(v as i32),
                "parseLong" => Value::Long(v),
                "parseShort" => Value::Short(v as i16),
                _ => Value::Byte(v as i8),
            }
        }
        "parseDouble" | "parseFloat" => {
            let v: f64 = s
                .trim()
                .parse()
                .map_err(|_| Error::eval(format!("For input string: \"{}\"", s)))?;
            if name == "parseFloat" {
                Value::Float(v as f32)
            } else {
                Value::Double(v)
            }
        }
        "valueOf" => match args.first() {
            Some(Value::Str(_)) => {
                let inner = number_static(
                    class,
                    &format!("parse{}", class.rsplit('.').next().unwrap_or("Int")),
                    args,
                )?;
                inner
            }
            Some(v) => v.clone(),
            None => Value::Null,
        },
        "toString" => match args.first() {
            Some(Value::Str(_)) => Value::string(s),
            Some(v) => Value::string(display_java(v)),
            None => Value::string(""),
        },
        "toHexString" => Value::string(format!("{:x}", args[0].as_i64().unwrap_or(0))),
        "toOctalString" => Value::string(format!("{:o}", args[0].as_i64().unwrap_or(0))),
        "toBinaryString" => Value::string(format!("{:b}", args[0].as_i64().unwrap_or(0))),
        "isNaN" => Value::Bool(
            args.first()
                .map(|v| v.as_f64().map(|f| f.is_nan()).unwrap_or(false))
                .unwrap_or(false),
        ),
        "isInfinite" => Value::Bool(
            args.first()
                .map(|v| v.as_f64().map(|f| f.is_infinite()).unwrap_or(false))
                .unwrap_or(false),
        ),
        "compare" => Value::Int(primitive::compare(&args[0], &args[1])?),
        "doubleValue" => Value::Double(args[0].as_f64()?),
        "intValue" => Value::Int(args[0].as_i64()? as i32),
        "longValue" => Value::Long(args[0].as_i64()?),
        other => {
            return Err(Error::eval(format!(
                "No such static method: {}.{}()",
                class, other
            )))
        }
    })
}

fn bool_static(name: &str, args: &[Value]) -> Result<Value> {
    Ok(match name {
        "valueOf" => match args.first() {
            Some(Value::Bool(b)) => Value::Bool(*b),
            Some(Value::Str(s)) => Value::Bool(s.eq_ignore_ascii_case("true")),
            _ => Value::Bool(false),
        },
        "parseBoolean" => Value::Bool(
            args.first()
                .map(|v| display_java(v).eq_ignore_ascii_case("true"))
                .unwrap_or(false),
        ),
        "toString" => Value::string(
            args.first()
                .map(display_java)
                .unwrap_or_else(|| "false".to_string()),
        ),
        other => {
            return Err(Error::eval(format!(
                "No such static method: Boolean.{}()",
                other
            )))
        }
    })
}

fn char_static(name: &str, args: &[Value]) -> Result<Value> {
    let c = match args.first() {
        Some(Value::Char(c)) => *c,
        Some(v) => v.as_i64()? as u8 as char,
        None => '\0',
    };
    Ok(match name {
        "isDigit" => Value::Bool(c.is_ascii_digit() || c.is_numeric()),
        "isLetter" => Value::Bool(c.is_alphabetic()),
        "isWhitespace" => Value::Bool(c.is_whitespace()),
        "isUpperCase" => Value::Bool(c.is_uppercase()),
        "isLowerCase" => Value::Bool(c.is_lowercase()),
        "toUpperCase" => Value::Char(c.to_uppercase().next().unwrap_or(c)),
        "toLowerCase" => Value::Char(c.to_lowercase().next().unwrap_or(c)),
        "valueOf" => Value::Char(c),
        "toString" => Value::string(c.to_string()),
        other => {
            return Err(Error::eval(format!(
                "No such static method: Character.{}()",
                other
            )))
        }
    })
}

fn string_static(interp: &mut Interpreter, name: &str, args: &[Value]) -> Result<Value> {
    match name {
        "valueOf" => Ok(Value::string(
            args.first()
                .map(display_java)
                .unwrap_or_else(|| "null".to_string()),
        )),
        "join" => {
            let sep = args.first().map(display_java).unwrap_or_default();
            // join(separator, iterable) and join(separator, array, ...) forms.
            if args.len() == 2 {
                match &args[1] {
                    Value::Builtin(_) | Value::Java(_) | Value::Array(_) | Value::Str(_) => {
                        let items = interp.iterate(&args[1])?;
                        let parts: Vec<String> = items.iter().map(display_java).collect();
                        return Ok(Value::string(parts.join(&sep)));
                    }
                    _ => {}
                }
            }
            if args.len() == 2 {
                let items = interp.iterate(&args[1])?;
                let parts: Vec<String> = items.iter().map(display_java).collect();
                return Ok(Value::string(parts.join(&sep)));
            }
            let rest: Vec<String> = args[1..].iter().map(display_java).collect();
            Ok(Value::string(rest.join(&sep)))
        }
        "format" => {
            // Prefer the JVM: exact Java formatting semantics (flags, width,
            // precision, Locale). The local fallback below only runs without
            // a JVM (--no-jvm).
            if let Some(jvm) = interp.jvm() {
                let mut values = Vec::with_capacity(args.len());
                let mut convertible = true;
                for arg in args {
                    match crate::java::to_jval(interp, arg) {
                        Ok(value) => values.push(value),
                        Err(_) => {
                            convertible = false;
                            break;
                        }
                    }
                }
                if convertible {
                    if let Ok(value) =
                        jvm.call_static(interp.runtime_id, "java.lang.String", "format", &values)
                    {
                        return Ok(crate::java::from_jval(value));
                    }
                }
            }
            let fmt = args.first().map(display_java).unwrap_or_default();
            Ok(Value::string(native_format(&fmt, args)))
        }
        other => Err(Error::eval(format!(
            "No such static method: String.{}()",
            other
        ))),
    }
}

/// Minimal printf-style formatter used when no JVM is available.
fn native_format(fmt: &str, args: &[Value]) -> String {
    let mut out = String::new();
    let mut ai = 1usize;
    let mut chars = fmt.chars().peekable();
    while let Some(c) = chars.next() {
        if c != '%' {
            out.push(c);
            continue;
        }
        let (mut minus, mut zero, mut plus, mut space) = (false, false, false, false);
        loop {
            match chars.peek().copied() {
                Some('-') => {
                    minus = true;
                    chars.next();
                }
                Some('0') => {
                    zero = true;
                    chars.next();
                }
                Some('+') => {
                    plus = true;
                    chars.next();
                }
                Some(' ') => {
                    space = true;
                    chars.next();
                }
                Some(',') => {
                    chars.next();
                }
                _ => break,
            }
        }
        let mut width = 0usize;
        while let Some(d) = chars.peek().copied().filter(|c| c.is_ascii_digit()) {
            width = width * 10 + d.to_digit(10).unwrap() as usize;
            chars.next();
        }
        let mut precision: Option<usize> = None;
        if chars.peek() == Some(&'.') {
            chars.next();
            let mut p = 0usize;
            while let Some(d) = chars.peek().copied().filter(|c| c.is_ascii_digit()) {
                p = p * 10 + d.to_digit(10).unwrap() as usize;
                chars.next();
            }
            precision = Some(p);
        }
        let conv = match chars.next() {
            Some(conv) => conv,
            None => {
                out.push('%');
                break;
            }
        };
        let mut text = match conv {
            '%' => "%".to_string(),
            'n' => "\n".to_string(),
            's' => {
                let s = args
                    .get(ai)
                    .map(display_java)
                    .unwrap_or_else(|| "null".to_string());
                ai += 1;
                match precision {
                    Some(p) => s.chars().take(p).collect(),
                    None => s,
                }
            }
            'b' => {
                let value = args
                    .get(ai)
                    .map(|v| match v {
                        Value::Bool(b) => *b,
                        Value::Null | Value::Void => false,
                        _ => true,
                    })
                    .unwrap_or(false);
                ai += 1;
                value.to_string()
            }
            'c' => {
                let ch = args
                    .get(ai)
                    .map(|v| match v {
                        Value::Char(c) => *c,
                        other => other
                            .as_i64()
                            .ok()
                            .and_then(|i| char::from_u32(i as u32))
                            .unwrap_or('\u{0}'),
                    })
                    .unwrap_or('\u{0}');
                ai += 1;
                ch.to_string()
            }
            'd' => {
                let value = args.get(ai).map(|v| v.as_i64().unwrap_or(0)).unwrap_or(0);
                ai += 1;
                let mut s = value.abs().to_string();
                if value < 0 {
                    s.insert(0, '-');
                } else if plus {
                    s.insert(0, '+');
                } else if space {
                    s.insert(0, ' ');
                }
                s
            }
            'x' => {
                let value = args.get(ai).map(|v| v.as_i64().unwrap_or(0)).unwrap_or(0);
                ai += 1;
                format!("{:x}", value)
            }
            'X' => {
                let value = args.get(ai).map(|v| v.as_i64().unwrap_or(0)).unwrap_or(0);
                ai += 1;
                format!("{:X}", value)
            }
            'o' => {
                let value = args.get(ai).map(|v| v.as_i64().unwrap_or(0)).unwrap_or(0);
                ai += 1;
                format!("{:o}", value)
            }
            'f' | 'F' | 'e' | 'E' | 'g' | 'G' => {
                let value = args
                    .get(ai)
                    .map(|v| v.as_f64().unwrap_or(0.0))
                    .unwrap_or(0.0);
                ai += 1;
                let p = precision.unwrap_or(6);
                match conv {
                    'f' | 'F' => format!("{:.*}", p, value),
                    'e' => format!("{:.*e}", p, value),
                    'E' => format!("{:.*E}", p, value),
                    _ => format!("{}", value),
                }
            }
            other => {
                ai += 1;
                format!("%{}", other)
            }
        };
        let len = text.chars().count();
        if width > len {
            let pad = width - len;
            let numeric = matches!(
                conv,
                'd' | 'x' | 'X' | 'o' | 'f' | 'F' | 'e' | 'E' | 'g' | 'G'
            );
            if minus {
                text.push_str(&" ".repeat(pad));
            } else if zero && numeric {
                if text.starts_with('-') || text.starts_with('+') || text.starts_with(' ') {
                    let sign = text[..1].to_string();
                    text = format!("{}{}{}", sign, "0".repeat(pad), &text[1..]);
                } else {
                    text = format!("{}{}", "0".repeat(pad), text);
                }
            } else {
                text = format!("{}{}", " ".repeat(pad), text);
            }
        }
        out.push_str(&text);
    }
    out
}

fn arrays_static(interp: &mut Interpreter, name: &str, args: &[Value]) -> Result<Value> {
    match name {
        "asList" => {
            let mut items = Vec::new();
            for a in args {
                match a {
                    Value::Array(arr) => items.extend(arr.borrow().items.clone()),
                    other => items.push(other.clone()),
                }
            }
            Ok(Value::Builtin(Rc::new(RefCell::new(ListObj {
                items,
                kind: "ArrayList",
            }))))
        }
        "toString" => match args.first() {
            Some(v) => Ok(Value::string(display_java(v))),
            None => Ok(Value::string("null")),
        },
        "sort" => {
            if let Some(Value::Array(a)) = args.first() {
                a.borrow_mut().items.sort_by(|x, y| {
                    primitive::compare(x, y)
                        .map(|c| c.cmp(&0))
                        .unwrap_or(std::cmp::Ordering::Equal)
                });
            }
            Ok(Value::Void)
        }
        "equals" => Ok(Value::Bool(match (args.first(), args.get(1)) {
            (Some(Value::Array(a)), Some(Value::Array(b))) => {
                let (a, b) = (a.borrow(), b.borrow());
                a.items.len() == b.items.len()
                    && a.items
                        .iter()
                        .zip(b.items.iter())
                        .all(|(x, y)| x.equality(y))
            }
            _ => false,
        })),
        "fill" => {
            let _ = interp;
            Ok(Value::Void)
        }
        other => Err(Error::eval(format!(
            "No such static method: Arrays.{}()",
            other
        ))),
    }
}

fn collections_static(_interp: &mut Interpreter, name: &str, args: &[Value]) -> Result<Value> {
    match name {
        "synchronizedList" | "unmodifiableList" | "synchronizedMap" | "unmodifiableMap"
        | "synchronizedSet" | "unmodifiableSet" => Ok(args.first().cloned().unwrap_or(Value::Null)),
        "sort" => {
            if let Some(Value::Builtin(b)) = args.first() {
                b.borrow_mut().call_method(_interp, "sort", Vec::new())?;
            }
            Ok(Value::Void)
        }
        "reverse" => {
            if let Some(Value::Builtin(b)) = args.first() {
                let mut obj = b.borrow_mut();
                let _ = obj.call_method(_interp, "size", Vec::new());
            }
            Ok(Value::Void)
        }
        "emptyList" | "emptySet" => Ok(Value::Builtin(Rc::new(RefCell::new(ListObj {
            items: Vec::new(),
            kind: "ArrayList",
        })))),
        "singletonList" => Ok(Value::Builtin(Rc::new(RefCell::new(ListObj {
            items: vec![args.first().cloned().unwrap_or(Value::Null)],
            kind: "ArrayList",
        })))),
        other => Err(Error::eval(format!(
            "No such static method: Collections.{}()",
            other
        ))),
    }
}

// ---------------------------------------------------------------- construct

pub fn construct(
    interp: &mut Interpreter,
    class: &Rc<ClassValue>,
    kind: &str,
    args: Vec<Value>,
) -> Result<Value> {
    match kind {
        "list" => {
            let mut items = Vec::new();
            if let Some(Value::Array(a)) = args.first() {
                items = a.borrow().items.clone();
            } else if let Some(v) = args.first() {
                if matches!(v, Value::Builtin(_)) {
                    items = interp.iterate(v)?;
                }
            }
            Ok(Value::Builtin(Rc::new(RefCell::new(ListObj {
                items,
                kind: if class.name.contains("LinkedList") {
                    "LinkedList"
                } else {
                    "ArrayList"
                },
            }))))
        }
        "set" => {
            let mut items: Vec<Value> = Vec::new();
            if let Some(Value::Array(a)) = args.first() {
                items = a.borrow().items.clone();
            } else if let Some(v) = args.first() {
                if matches!(v, Value::Builtin(_)) {
                    items = interp.iterate(v)?;
                }
            }
            Ok(Value::Builtin(Rc::new(RefCell::new(ListObj {
                items,
                kind: "HashSet",
            }))))
        }
        "map" => {
            let _ = interp;
            let entries = Vec::new();
            Ok(Value::Builtin(Rc::new(RefCell::new(MapObj {
                entries,
                kind: "HashMap",
            }))))
        }
        "builder" => {
            let mut buf = String::new();
            for a in &args {
                buf.push_str(&display_java(a));
            }
            Ok(Value::Builtin(Rc::new(RefCell::new(BuilderObj {
                buf: Rc::new(RefCell::new(buf)),
            }))))
        }
        "string" => Ok(Value::string(
            args.first().map(display_java).unwrap_or_default(),
        )),
        "printstream" => Ok(Value::Builtin(Rc::new(RefCell::new(PrintStreamObj {
            err: false,
        })))),
        "exception" => {
            let mut fields = HashMap::new();
            if let Some(v) = args.first() {
                fields.insert("message".to_string(), Value::string(display_java(v)));
            }
            Ok(Value::Object(Rc::new(RefCell::new(ObjectValue {
                class: class.clone(),
                fields,
                java: None,
            }))))
        }
        "int" | "long" | "double" | "float" | "short" | "byte" | "char" | "boolean" | "number" => {
            if let Some(v) = args.first() {
                Ok(v.clone())
            } else {
                Ok(Value::Int(0))
            }
        }
        _ => Ok(Value::Object(Rc::new(RefCell::new(ObjectValue {
            class: class.clone(),
            fields: HashMap::new(),
            java: None,
        })))),
    }
}

pub fn exception_method(
    class: &Rc<ClassValue>,
    name: &str,
    args: &[Value],
    this: Option<&Value>,
) -> Option<Result<Value>> {
    if class.builtin != Some("exception") && !class.is_a("Throwable") && !class.is_a("EvalError") {
        return None;
    }
    let message = match this {
        Some(Value::Object(o)) => o
            .borrow()
            .fields
            .get("message")
            .cloned()
            .unwrap_or(Value::Null),
        _ => Value::Null,
    };
    Some(match (name, args.len()) {
        ("getMessage", 0) => Ok(message),
        ("getLocalizedMessage", 0) => Ok(message),
        ("getCause", 0) => Ok(Value::Null),
        ("toString", 0) => Ok(Value::string(match &message {
            Value::Null | Value::Void => class.name.clone(),
            m => format!("{}: {}", class.name, display_java(m)),
        })),
        ("printStackTrace", 0) => Ok(Value::Void),
        ("getClass", 0) => Ok(Value::Class(class.clone())),
        _ => return None,
    })
}

pub fn instance_method(
    _interp: &mut Interpreter,
    class: &Rc<ClassValue>,
    _kind: &str,
    name: &str,
    args: &[Value],
) -> Option<Result<Value>> {
    // Methods defined by the wrapper classes on instances.
    match (class.name.rsplit('.').next().unwrap_or(""), name) {
        ("Double", "doubleValue") => Some(Ok(args.first().cloned().unwrap_or(Value::Double(0.0)))),
        (_, "equals") if args.len() == 1 => Some(Ok(Value::Bool(args.len() == 1))),
        _ => None,
    }
}

// ------------------------------------------------------------------ strings

pub fn string_method(
    interp: &mut Interpreter,
    s: &str,
    name: &str,
    args: Vec<Value>,
) -> Result<Value> {
    let chars: Vec<char> = s.chars().collect();
    let arg_str = |i: usize| args.get(i).map(display_java).unwrap_or_default();
    match name {
        "length" => Ok(Value::Int(chars.len() as i32)),
        "isEmpty" => Ok(Value::Bool(chars.is_empty())),
        "charAt" if args.len() == 1 => {
            let i = args[0].as_i64()?;
            chars
                .get(i as usize)
                .map(|c| Value::Char(*c))
                .ok_or_else(|| Error::eval(format!("String index out of range: {}", i)))
        }
        "substring" if args.len() == 1 => {
            let from = args[0].as_i64()? as usize;
            Ok(Value::string(
                chars
                    .get(from..)
                    .map(|c| c.iter().collect::<String>())
                    .unwrap_or_default(),
            ))
        }
        "substring" if args.len() == 2 => {
            let from = args[0].as_i64()? as usize;
            let to = args[1].as_i64()? as usize;
            if from > to || to > chars.len() {
                return Err(Error::eval("String index out of range"));
            }
            Ok(Value::string(chars[from..to].iter().collect::<String>()))
        }
        "equals" if args.len() == 1 => Ok(Value::Bool(match &args[0] {
            Value::Str(o) => o.as_ref() == s,
            other => s == other.to_display_string(),
        })),
        "equalsIgnoreCase" if args.len() == 1 => {
            Ok(Value::Bool(s.eq_ignore_ascii_case(&arg_str(0))))
        }
        "indexOf" if !args.is_empty() => {
            let needle = arg_str(0);
            let from = args
                .get(1)
                .map(|v| v.as_i64().unwrap_or(0) as usize)
                .unwrap_or(0);
            let hay: String = chars.iter().collect();
            let pos = hay
                .char_indices()
                .filter(|(i, _)| *i >= from)
                .find(|(i, _)| hay[*i..].starts_with(&needle))
                .map(|(i, _)| hay[..i].chars().count() as i32)
                .unwrap_or(-1);
            Ok(Value::Int(pos))
        }
        "lastIndexOf" if !args.is_empty() => {
            let needle = arg_str(0);
            let hay: String = chars.iter().collect();
            let pos = hay
                .rfind(&needle)
                .map(|i| hay[..i].chars().count() as i32)
                .unwrap_or(-1);
            Ok(Value::Int(pos))
        }
        "toUpperCase" => Ok(Value::string(s.to_uppercase())),
        "toLowerCase" => Ok(Value::string(s.to_lowercase())),
        "trim" | "strip" => Ok(Value::string(s.trim().to_string())),
        "replace" if args.len() == 2 => {
            if let (Value::Char(a), Value::Char(c)) = (&args[0], &args[1]) {
                Ok(Value::string(
                    s.chars()
                        .map(|x| if x == *a { *c } else { x })
                        .collect::<String>(),
                ))
            } else {
                Ok(Value::string(s.replace(&arg_str(0), &arg_str(1))))
            }
        }
        "split" if !args.is_empty() => {
            let parts: Vec<Value> = s
                .split(arg_str(0).as_str())
                .map(|p| Value::string(p.to_string()))
                .collect();
            Ok(Value::Array(Rc::new(RefCell::new(ArrayValue {
                dims: 1,
                component: None,
                items: parts,
            }))))
        }
        "contains" if args.len() == 1 => Ok(Value::Bool(s.contains(&arg_str(0)))),
        "startsWith" if !args.is_empty() => Ok(Value::Bool(s.starts_with(&arg_str(0)))),
        "endsWith" if !args.is_empty() => Ok(Value::Bool(s.ends_with(&arg_str(0)))),
        "compareTo" if args.len() == 1 => Ok(Value::Int(match s.cmp(arg_str(0).as_str()) {
            std::cmp::Ordering::Less => -1,
            std::cmp::Ordering::Equal => 0,
            std::cmp::Ordering::Greater => 1,
        })),
        "compareToIgnoreCase" if args.len() == 1 => Ok(Value::Int(
            match s.to_lowercase().cmp(&arg_str(0).to_lowercase()) {
                std::cmp::Ordering::Less => -1,
                std::cmp::Ordering::Equal => 0,
                std::cmp::Ordering::Greater => 1,
            },
        )),
        "concat" if args.len() == 1 => Ok(Value::string(format!("{}{}", s, arg_str(0)))),
        "toString" => Ok(Value::string(s.to_string())),
        "intern" => Ok(Value::string(s.to_string())),
        "hashCode" => {
            let mut h: i32 = 0;
            for c in chars {
                h = h.wrapping_mul(31).wrapping_add(c as i32);
            }
            Ok(Value::Int(h))
        }
        "toCharArray" => Ok(Value::Array(Rc::new(RefCell::new(ArrayValue {
            dims: 1,
            component: None,
            items: chars.into_iter().map(Value::Char).collect(),
        })))),
        "getBytes" => Ok(Value::Array(Rc::new(RefCell::new(ArrayValue {
            dims: 1,
            component: None,
            items: s.as_bytes().iter().map(|b| Value::Byte(*b as i8)).collect(),
        })))),
        "repeat" if args.len() == 1 => {
            Ok(Value::string(s.repeat(args[0].as_i64()?.max(0) as usize)))
        }
        "matches" if args.len() == 1 => Ok(Value::Bool(simple_matches(s, &arg_str(0)))),
        "isBlank" => Ok(Value::Bool(chars.iter().all(|c| c.is_whitespace()))),
        other
            if matches!(
                other,
                "lines"
                    | "stripIndent"
                    | "translateEscapes"
                    | "formatted"
                    | "indent"
                    | "transform"
                    | "chars"
                    | "codePoints"
                    | "describeConstable"
            ) =>
        {
            if let Some(jvm) = interp.jvm() {
                if let Ok(object) = jvm.new_string_object(s) {
                    let mut values = Vec::new();
                    for argument in &args {
                        values.push(crate::java::to_jval(interp, argument)?);
                    }
                    return match jvm.call(&object, other, &values) {
                        Ok(value) => Ok(crate::java::from_jval(value)),
                        Err(bsh_jvm::JError::Thrown(t)) => Err(Error::Thrown(Value::Java(t))),
                        Err(error) => Err(crate::java::error(error)),
                    };
                }
            }
            Err(Error::eval(format!(
                "No such method: {}() on String",
                other
            )))
        }
        other => {
            let _ = interp;
            Err(Error::eval(format!(
                "No such method: {}() on String",
                other
            )))
        }
    }
}

/// Very small regex subset: ^ $ . and * plus literal text.
fn simple_matches(text: &str, pattern: &str) -> bool {
    if pattern == ".*" {
        return true;
    }
    let p = pattern.trim_start_matches('^').trim_end_matches('$');
    if p.contains(".*") {
        let mut parts = p.split(".*");
        let first = parts.next().unwrap_or("");
        let last = parts.next().unwrap_or("");
        if first.is_empty() && last.is_empty() {
            return true;
        }
        return text.starts_with(first) && text.ends_with(last);
    }
    if p.contains('.') {
        return text.len() == p.len();
    }
    text == p
}

// ---------------------------------------------------------------- functions

pub fn is_builtin_function(name: &str) -> bool {
    matches!(
        name,
        "print"
            | "println"
            | "error"
            | "log"
            | "eval"
            | "source"
            | "unset"
            | "getSourceFileInfo"
            | "exit"
            | "invoke"
            | "toString"
            | "__close"
            | "assert"
            | "assertTrue"
            | "assertFalse"
            | "assertNull"
            | "assertNotNull"
            | "assertEquals"
            | "assertNotEquals"
            | "assertSame"
            | "assertNotSame"
            | "assertArrayEquals"
            | "fail"
    )
}

pub fn call_function(
    interp: &mut Interpreter,
    name: &str,
    args: Vec<Value>,
    ns: &Ns,
) -> Result<Value> {
    let result = call_function_inner(interp, name, args, ns);
    match result {
        Err(Error::Eval(message)) => Err(Error::Eval(interp.decoration(&message))),
        other => other,
    }
}

fn call_function_inner(
    interp: &mut Interpreter,
    name: &str,
    args: Vec<Value>,
    ns: &Ns,
) -> Result<Value> {
    match name {
        "assert" | "assertTrue" | "assertFalse" | "assertNull" | "assertNotNull"
        | "assertEquals" | "assertNotEquals" | "assertSame" | "assertNotSame"
        | "assertArrayEquals" | "fail" => return assertions::call(name, &args),
        "print" | "println" | "error" => {
            // println(x) used via this.interpreter.println(...)
            let text: String = args.iter().map(display_java).collect::<Vec<_>>().join("");
            interp.write_line(&text);
            Ok(Value::Void)
        }
        "log" => {
            let text = args.iter().map(display_java).collect::<Vec<_>>().join(" ");
            interp.write_line(&text);
            Ok(Value::Void)
        }
        "__close" => {
            let value = args.first().cloned().unwrap_or(Value::Null);
            if !matches!(value, Value::Null | Value::Void) {
                interp.call_method_on(&value, "close", Vec::new(), ns)?;
            }
            Ok(Value::Void)
        }
        "eval" => {
            let src = match args.first() {
                Some(Value::Str(s)) => s.to_string(),
                Some(other) => other.to_display_string(),
                None => return Err(Error::eval("eval() requires a string argument")),
            };
            let target = match args.get(1) {
                Some(Value::Namespace(n)) => n.clone(),
                _ => ns.clone(),
            };
            let result = interp.eval_source_in(&src, &target);
            // bsh accepts a bare expression here: eval("1+2") returns 3.
            if result.is_err() && !src.contains(';') && !src.contains('{') {
                let retry = format!("{};", src);
                return interp.eval_source_in(&retry, &target);
            }
            result
        }
        "source" => {
            let path = match args.first() {
                Some(Value::Str(s)) => s.to_string(),
                Some(other) => other.to_display_string(),
                None => return Err(Error::eval("source() requires a file name")),
            };
            interp.eval_file(&path)
        }
        "unset" => {
            if let Some(Value::Str(s)) = args.first() {
                if let Some(owner) = crate::namespace::Namespace::owner_of(ns, s) {
                    owner.borrow_mut().vars.remove(s.as_ref());
                }
            }
            Ok(Value::Void)
        }
        "getSourceFileInfo" => Ok(Value::string(
            interp
                .source_file
                .clone()
                .unwrap_or_else(|| "unknown".to_string()),
        )),
        "toString" => Ok(Value::string(
            args.first()
                .map(display_java)
                .unwrap_or_else(|| "null".to_string()),
        )),
        "invoke" => Err(Error::eval("invoke() requires a Method value")),
        "exit" => Err(Error::eval("exit called")),
        other => Err(Error::eval(format!("No such command: {}", other))),
    }
}

pub fn object_field(_interp: &mut Interpreter, base: &Value, name: &str) -> Option<Result<Value>> {
    match (base, name) {
        (Value::Object(o), "message") => Some(Ok(o
            .borrow()
            .fields
            .get("message")
            .cloned()
            .unwrap_or(Value::Null))),
        _ => None,
    }
}

pub fn array_method(
    _interp: &mut Interpreter,
    base: &Value,
    name: &str,
    args: Vec<Value>,
) -> Result<Value> {
    match (base, name) {
        (Value::Array(a), "clone") if args.is_empty() => {
            Ok(Value::Array(Rc::new(RefCell::new(ArrayValue {
                dims: a.borrow().dims,
                component: a.borrow().component.clone(),
                items: a.borrow().items.clone(),
            }))))
        }
        (Value::Array(_), "length") => Ok(Value::Int(0)),
        (_, name) => Err(Error::eval(format!("No such method: {}() on array", name))),
    }
}

pub fn index_get(interp: &mut Interpreter, base: &Value, idx: &Value) -> Result<Value> {
    match base {
        Value::Builtin(b) => {
            let mut obj = b.borrow_mut();
            match obj.class_name() {
                "ArrayList" | "LinkedList" | "HashSet" => {
                    obj.call_method(interp, "get", vec![idx.clone()])
                }
                _ => obj.call_method(interp, "get", vec![idx.clone()]),
            }
        }
        other => Err(Error::eval(format!("Cannot index {}", other.type_name()))),
    }
}

pub fn index_set(interp: &mut Interpreter, base: &Value, idx: &Value, value: Value) -> Result<()> {
    match base {
        Value::Builtin(b) => {
            let mut obj = b.borrow_mut();
            obj.call_method(interp, "set", vec![idx.clone(), value])?;
            Ok(())
        }
        other => Err(Error::eval(format!("Cannot index {}", other.type_name()))),
    }
}

pub fn iterate(interp: &mut Interpreter, value: &Value) -> Result<Vec<Value>> {
    match value {
        Value::Builtin(b) => {
            let mut obj = b.borrow_mut();
            match obj.class_name() {
                "ArrayList" | "LinkedList" | "HashSet" | "Set" => obj
                    .call_method(interp, "toArray", Vec::new())
                    .and_then(|v| match v {
                        Value::Array(a) => Ok(a.borrow().items.clone()),
                        _ => Ok(Vec::new()),
                    }),
                "HashMap" => {
                    let v = obj.call_method(interp, "keySet", Vec::new())?;
                    match v {
                        Value::Builtin(k) => {
                            let mut k = k.borrow_mut();
                            match k.call_method(interp, "toArray", Vec::new())? {
                                Value::Array(a) => Ok(a.borrow().items.clone()),
                                _ => Ok(Vec::new()),
                            }
                        }
                        _ => Ok(Vec::new()),
                    }
                }
                _ => Err(Error::eval(format!("{} is not iterable", obj.class_name()))),
            }
        }
        other => Err(Error::eval(format!(
            "{} is not iterable",
            other.type_name()
        ))),
    }
}
