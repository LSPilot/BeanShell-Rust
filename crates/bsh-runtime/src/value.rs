//! Runtime values.

use crate::class::{ClassDef, ClassValue};
use crate::error::{Error, Result};
use crate::interp::Interpreter;
use crate::primitive::{self, PrimKind};
use bsh_syntax::ast::TypeRef;
use std::cell::RefCell;
use std::collections::HashMap;
use std::fmt;
use std::rc::Rc;
use std::sync::Arc;

/// A host object implemented in Rust and exposed to scripts.
pub trait HostObject: fmt::Debug {
    fn class_name(&self) -> &'static str;
    fn call_method(
        &mut self,
        interp: &mut Interpreter,
        name: &str,
        args: Vec<Value>,
    ) -> Result<Value>;
    fn get_field(&mut self, _name: &str) -> Result<Value> {
        Err(Error::eval(format!("No such field: {}", _name)))
    }
    fn set_field(&mut self, name: &str, _v: Value) -> Result<()> {
        Err(Error::eval(format!("No such field: {}", name)))
    }
    fn display(&self) -> String {
        format!("{}@{}", self.class_name(), self.class_name().len())
    }
}

#[derive(Clone)]
pub enum Value {
    /// BeanShell's "void" / undefined value.
    Void,
    Null,
    Bool(bool),
    Char(char),
    Byte(i8),
    Short(i16),
    Int(i32),
    Long(i64),
    Float(f32),
    Double(f64),
    Str(Rc<str>),
    Array(Rc<RefCell<ArrayValue>>),
    Object(Rc<RefCell<ObjectValue>>),
    Class(Rc<ClassValue>),
    Method(Rc<MethodDef>),
    Builtin(Rc<RefCell<dyn HostObject>>),
    /// A namespace used as an object, e.g. the script level "this".
    Namespace(Rc<RefCell<crate::namespace::Namespace>>),
    /// A live Java object owned by the embedded JVM.
    Java(std::sync::Arc<bsh_jvm::JObj>),
    /// A lambda captured with its defining namespace.
    Lambda(Rc<LambdaValue>),
    /// A method reference such as String::valueOf or obj::method.
    MethodRef(Rc<MethodRefValue>),
}

/// A callable closure: the synthetic method holding parameters and body.
#[derive(Debug)]
pub struct LambdaValue {
    pub method: Rc<MethodDef>,
    pub source_file: Option<String>,
    pub source_text: Option<String>,
}

/// A method reference target: resolved eagerly when possible.
#[derive(Debug)]
pub enum MethodRefTarget {
    Value(Value),
    Deferred(Box<bsh_syntax::ast::Expr>, crate::interp::Ns),
}

#[derive(Debug)]
pub struct MethodRefValue {
    pub target: MethodRefTarget,
    /// Method name, or "new" for a constructor reference.
    pub name: String,
}

impl fmt::Debug for Value {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Value::Str(s) => write!(f, "Str({:?})", s),
            Value::Builtin(b) => write!(f, "Builtin({:?})", b.borrow()),
            other => write!(f, "{}", other.to_display_string()),
        }
    }
}

#[derive(Debug)]
pub struct ArrayValue {
    pub dims: usize,
    pub component: Option<TypeRef>,
    pub items: Vec<Value>,
}

#[derive(Debug)]
pub struct ObjectValue {
    pub class: Rc<ClassValue>,
    pub fields: HashMap<String, Value>,
    /// For scripted classes that extend a Java class, the live Java instance
    /// (a generated subclass) is stored here.
    pub java: Option<Arc<bsh_jvm::JObj>>,
}

/// A scripted or host method bound to its lexical namespace.
#[derive(Debug)]
pub struct MethodDef {
    pub name: String,
    pub params: Vec<bsh_syntax::ast::Param>,
    pub body: Option<Box<bsh_syntax::ast::Stmt>>,
    pub return_type: Option<TypeRef>,
    pub ns: Option<Rc<RefCell<crate::namespace::Namespace>>>,
    pub declaring_class: Option<Rc<ClassValue>>,
    pub is_ctor: bool,
    pub modifiers: bsh_syntax::ast::Modifiers,
    pub source_file: Option<String>,
    pub source_text: Option<String>,
}

#[derive(Debug)]
pub struct FieldDef {
    pub name: String,
    pub ty: Option<TypeRef>,
    pub modifiers: bsh_syntax::ast::Modifiers,
    pub init: Option<bsh_syntax::ast::Expr>,
}

impl Value {
    pub fn string(s: impl Into<String>) -> Value {
        Value::Str(Rc::from(s.into()))
    }

    pub fn is_null_like(&self) -> bool {
        matches!(self, Value::Null | Value::Void)
    }

    pub fn is_numeric(&self) -> bool {
        matches!(
            self,
            Value::Byte(_)
                | Value::Short(_)
                | Value::Int(_)
                | Value::Long(_)
                | Value::Float(_)
                | Value::Double(_)
                | Value::Char(_)
        )
    }

    pub fn type_name(&self) -> String {
        match self {
            Value::Void => "void".to_string(),
            Value::Null => "null".to_string(),
            Value::Bool(_) => "boolean".to_string(),
            Value::Char(_) => "char".to_string(),
            Value::Byte(_) => "byte".to_string(),
            Value::Short(_) => "short".to_string(),
            Value::Int(_) => "int".to_string(),
            Value::Long(_) => "long".to_string(),
            Value::Float(_) => "float".to_string(),
            Value::Double(_) => "double".to_string(),
            Value::Str(_) => "String".to_string(),
            Value::Array(a) => {
                let b = a.borrow();
                match &b.component {
                    Some(t) => t.display_name(),
                    None => "Object[]".to_string(),
                }
            }
            Value::Object(o) => o.borrow().class.name.clone(),
            Value::Class(c) => format!("class {}", c.name),
            Value::Method(_) => "Method".to_string(),
            Value::Builtin(b) => b.borrow().class_name().to_string(),
            Value::Namespace(_) => "NameSpace".to_string(),
            Value::Java(o) => o.class_name().to_string(),
            Value::Lambda(_) => "Lambda".to_string(),
            Value::MethodRef(_) => "Method".to_string(),
        }
    }

    /// Java style string conversion used by print and string concatenation.
    pub fn to_display_string(&self) -> String {
        match self {
            Value::Void => "void".to_string(),
            Value::Null => "null".to_string(),
            Value::Bool(b) => b.to_string(),
            Value::Char(c) => c.to_string(),
            Value::Byte(v) => v.to_string(),
            Value::Short(v) => v.to_string(),
            Value::Int(v) => v.to_string(),
            Value::Long(v) => v.to_string(),
            Value::Float(v) => primitive::format_float(*v as f64, true),
            Value::Double(v) => primitive::format_float(*v, false),
            Value::Str(s) => s.to_string(),
            Value::Array(a) => {
                let b = a.borrow();
                let inner: Vec<String> = b.items.iter().map(|v| v.to_display_string()).collect();
                format!("[{}]", inner.join(", "))
            }
            Value::Object(o) => {
                let o = o.borrow();
                if o.class.find_method("toString", 0).is_none() {
                    if let Some(java) = &o.java {
                        return java.jvm().to_java_string(java);
                    }
                }
                // Prefer a scripted toString() when the class defines one.
                if o.class.find_method("toString", 0).is_some() {
                    let mut interp = Interpreter::detached();
                    match crate::eval::invoke_method(
                        &mut interp,
                        &o.class,
                        &Value::Object(Rc::new(RefCell::new(ObjectValue {
                            class: o.class.clone(),
                            fields: o.fields.clone(),
                            java: None,
                        }))),
                        "toString",
                        Vec::new(),
                    ) {
                        Ok(v) => return v.to_display_string(),
                        Err(_) => {}
                    }
                }
                format!(
                    "{}@{}",
                    o.class.name,
                    Rc::as_ptr(&o.class) as usize % 0xffff
                )
            }
            Value::Class(c) => format!("Class Identifier: {}", c.name),
            Value::Method(m) => format!("Method: {}", m.name),
            Value::Builtin(b) => b.borrow().display(),
            Value::Namespace(n) => {
                let b = n.borrow();
                format!(
                    "NameSpace: {}",
                    b.name.clone().unwrap_or_else(|| "anonymous".to_string())
                )
            }
            Value::Java(o) => o.jvm().to_java_string(o),
            Value::Lambda(l) => format!("Lambda: {}", l.method.name),
            Value::MethodRef(m) => format!("MethodRef: {}", m.name),
        }
    }

    pub fn as_bool(&self) -> Result<bool> {
        match self {
            Value::Bool(b) => Ok(*b),
            other => Err(Error::eval(format!(
                "Condition must be boolean, not {}",
                other.type_name()
            ))),
        }
    }

    pub fn as_i64(&self) -> Result<i64> {
        match self {
            Value::Byte(v) => Ok(*v as i64),
            Value::Short(v) => Ok(*v as i64),
            Value::Int(v) => Ok(*v as i64),
            Value::Long(v) => Ok(*v),
            Value::Char(c) => Ok(*c as i64),
            Value::Float(v) => Ok(*v as i64),
            Value::Double(v) => Ok(*v as i64),
            Value::Bool(_) => Err(Error::eval("Cannot convert boolean to number")),
            other => Err(Error::eval(format!(
                "Cannot convert {} to number",
                other.type_name()
            ))),
        }
    }

    pub fn as_f64(&self) -> Result<f64> {
        match self {
            Value::Byte(v) => Ok(*v as f64),
            Value::Short(v) => Ok(*v as f64),
            Value::Int(v) => Ok(*v as f64),
            Value::Long(v) => Ok(*v as f64),
            Value::Char(c) => Ok(*c as u32 as f64),
            Value::Float(v) => Ok(*v as f64),
            Value::Double(v) => Ok(*v),
            other => Err(Error::eval(format!(
                "Cannot convert {} to number",
                other.type_name()
            ))),
        }
    }

    /// Numeric primitive used for promotion decisions.
    pub fn prim_kind(&self) -> Option<PrimKind> {
        Some(match self {
            Value::Byte(_) => PrimKind::Byte,
            Value::Short(_) => PrimKind::Short,
            Value::Int(_) => PrimKind::Int,
            Value::Long(_) => PrimKind::Long,
            Value::Float(_) => PrimKind::Float,
            Value::Double(_) => PrimKind::Double,
            Value::Char(_) => PrimKind::Char,
            _ => return None,
        })
    }

    pub fn instance_of(&self, class_name: &str) -> bool {
        if matches!(self, Value::Null | Value::Void) {
            return false;
        }
        if class_name == "Object" || class_name == "java.lang.Object" {
            return true;
        }
        match self {
            Value::Null | Value::Void => false,
            Value::Bool(_) => matches!(
                class_name,
                "boolean" | "Boolean" | "Object" | "java.lang.Boolean" | "java.lang.Object"
            ),
            Value::Char(_) => matches!(
                class_name,
                "char" | "Character" | "Object" | "java.lang.Character" | "java.lang.Object"
            ),
            Value::Byte(_) => matches!(
                class_name,
                "byte"
                    | "Byte"
                    | "Number"
                    | "Object"
                    | "java.lang.Byte"
                    | "java.lang.Number"
                    | "java.lang.Object"
            ),
            Value::Short(_) => matches!(
                class_name,
                "short"
                    | "Short"
                    | "Number"
                    | "Object"
                    | "java.lang.Short"
                    | "java.lang.Number"
                    | "java.lang.Object"
            ),
            Value::Int(_) => matches!(
                class_name,
                "int"
                    | "Integer"
                    | "Number"
                    | "Object"
                    | "java.lang.Integer"
                    | "java.lang.Number"
                    | "java.lang.Object"
            ),
            Value::Long(_) => matches!(
                class_name,
                "long"
                    | "Long"
                    | "Number"
                    | "Object"
                    | "java.lang.Long"
                    | "java.lang.Number"
                    | "java.lang.Object"
            ),
            Value::Float(_) => matches!(
                class_name,
                "float"
                    | "Float"
                    | "Number"
                    | "Object"
                    | "java.lang.Float"
                    | "java.lang.Number"
                    | "java.lang.Object"
            ),
            Value::Double(_) => matches!(
                class_name,
                "double"
                    | "Double"
                    | "Number"
                    | "Object"
                    | "java.lang.Double"
                    | "java.lang.Number"
                    | "java.lang.Object"
            ),
            Value::Str(_) => matches!(
                class_name,
                "String"
                    | "Object"
                    | "java.lang.String"
                    | "java.lang.Object"
                    | "CharSequence"
                    | "Comparable"
                    | "java.lang.CharSequence"
            ),
            Value::Array(_) => {
                matches!(class_name, "Object" | "java.lang.Object") || class_name.ends_with("[]")
            }
            Value::Object(o) => {
                let o = o.borrow();
                o.class.is_a(class_name)
                    || o.java
                        .as_ref()
                        .map(|java| {
                            java.jvm()
                                .is_instance(bsh_jvm::RuntimeId(0), java, class_name)
                        })
                        .unwrap_or(false)
            }
            Value::Class(_) => matches!(
                class_name,
                "Class" | "Object" | "java.lang.Class" | "java.lang.Object"
            ),
            Value::Method(_) => {
                matches!(class_name, "Method" | "Object" | "java.lang.reflect.Method")
            }
            Value::Builtin(b) => crate::builtins::is_builtin_a(b.borrow().class_name(), class_name),
            Value::Namespace(_) => matches!(
                class_name,
                "NameSpace" | "bsh.NameSpace" | "Object" | "java.lang.Object"
            ),
            Value::Lambda(_) | Value::MethodRef(_) => {
                matches!(
                    class_name,
                    "Object" | "java.lang.Object" | "Method" | "java.lang.reflect.Method"
                )
            }
            Value::Java(o) => {
                if class_name == "Object" || class_name == "java.lang.Object" {
                    return true;
                }
                if o.class_name() == class_name
                    || o.jvm().is_instance(bsh_jvm::RuntimeId(0), o, class_name)
                {
                    return true;
                }
                if !class_name.contains('.') {
                    return o.jvm().is_instance(
                        bsh_jvm::RuntimeId(0),
                        o,
                        &format!("java.lang.{}", class_name),
                    );
                }
                false
            }
        }
    }

    pub fn class_value(&self, interp: &Interpreter) -> Option<Rc<ClassValue>> {
        match self {
            Value::Object(o) => Some(o.borrow().class.clone()),
            Value::Class(c) => Some(c.clone()),
            other => interp.find_class(&other.type_name()),
        }
    }

    pub fn equality(&self, other: &Value) -> bool {
        match (self, other) {
            (Value::Void, Value::Void) => true,
            (Value::Null, Value::Null) => true,
            (Value::Bool(a), Value::Bool(b)) => a == b,
            (Value::Str(a), Value::Str(b)) => a == b,
            (Value::Char(a), Value::Char(b)) => a == b,
            (Value::Object(a), Value::Object(b)) => Rc::ptr_eq(a, b),
            (Value::Array(a), Value::Array(b)) => Rc::ptr_eq(a, b),
            (Value::Builtin(a), Value::Builtin(b)) => Rc::ptr_eq(a, b),
            (Value::Class(a), Value::Class(b)) => Rc::ptr_eq(a, b),
            (Value::Method(a), Value::Method(b)) => Rc::ptr_eq(a, b),
            (Value::Namespace(a), Value::Namespace(b)) => Rc::ptr_eq(a, b),
            (Value::Java(a), Value::Java(b)) => Arc::ptr_eq(a, b) || a.jvm().same_object(a, b),
            (a, b) if a.is_numeric() && b.is_numeric() => {
                let x = a.as_f64().unwrap_or(f64::NAN);
                let y = b.as_f64().unwrap_or(f64::NAN);
                x == y
            }
            _ => false,
        }
    }
}

impl ClassDef {
    pub fn find_method(&self, name: &str, argc: usize) -> Option<&Rc<MethodDef>> {
        self.methods
            .iter()
            .find(|m| m.name == name && (m.params.len() == argc || has_varargs(&m.params, argc)))
    }
}

pub fn has_varargs(params: &[bsh_syntax::ast::Param], argc: usize) -> bool {
    match params.last() {
        Some(p) if p.varargs => argc + 1 >= params.len(),
        _ => false,
    }
}
