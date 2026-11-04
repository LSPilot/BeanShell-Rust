//! Conversion between interpreter values and JNI values.

use crate::error::{Error, Result};
use crate::interp::Interpreter;
use crate::value::Value;
use bsh_jvm::{JError, JObj, JVal};
use std::sync::Arc;

pub fn error(e: JError) -> Error {
    match e {
        JError::Thrown(o) => Error::Thrown(Value::Java(o)),
        JError::Api(m) => Error::Eval(m),
    }
}

pub fn from_jval(v: JVal) -> Value {
    match v {
        JVal::Null => Value::Null,
        JVal::Bool(b) => Value::Bool(b),
        JVal::Char(c) => Value::Char(c),
        JVal::Byte(b) => Value::Byte(b),
        JVal::Short(s) => Value::Short(s),
        JVal::Int(i) => Value::Int(i),
        JVal::Long(l) => Value::Long(l),
        JVal::Float(f) => Value::Float(f),
        JVal::Double(d) => Value::Double(d),
        JVal::Str(s) => Value::string(s),
        JVal::Obj(o) => Value::Java(o),
    }
}

pub fn to_jval(interp: &mut Interpreter, v: &Value) -> Result<JVal> {
    Ok(match v {
        Value::Void | Value::Null => JVal::Null,
        Value::Bool(b) => JVal::Bool(*b),
        Value::Char(c) => JVal::Char(*c),
        Value::Byte(b) => JVal::Byte(*b),
        Value::Short(s) => JVal::Short(*s),
        Value::Int(i) => JVal::Int(*i),
        Value::Long(l) => JVal::Long(*l),
        Value::Float(f) => JVal::Float(*f),
        Value::Double(d) => JVal::Double(*d),
        Value::Str(s) => JVal::Str(s.to_string()),
        Value::Java(o) => JVal::Obj(o.clone()),
        Value::Class(c) => {
            let jvm = interp
                .jvm()
                .ok_or_else(|| Error::eval("No JVM available"))?;
            let class = jvm.class_ref(interp.runtime_id, &c.name).map_err(error)?;
            JVal::Obj(class)
        }
        Value::Array(a) => {
            let jvm = interp
                .jvm()
                .ok_or_else(|| Error::eval("No JVM available"))?;
            let (component, items) = {
                let b = a.borrow();
                let component = component_name(interp, &b.component);
                (component, b.items.clone())
            };
            let created = jvm
                .new_array(interp.runtime_id, &component, items.len())
                .map_err(error)?;
            let obj: Arc<JObj> = match created {
                JVal::Obj(o) => o,
                _ => return Err(Error::eval("Cannot create Java array")),
            };
            for (i, item) in items.iter().enumerate() {
                let jv = to_jval(interp, item)?;
                jvm.array_set(&obj, i, &jv).map_err(error)?;
            }
            JVal::Obj(obj)
        }
        Value::Object(o) => {
            if let Some(java) = &o.borrow().java {
                JVal::Obj(java.clone())
            } else {
                return Err(Error::eval(format!(
                    "Scripted object of class {} does not extend a Java class",
                    o.borrow().class.name
                )));
            }
        }
        Value::Builtin(b) => {
            let class_name = {
                let b = b.borrow();
                b.class_name().to_string()
            };
            if matches!(
                class_name.as_str(),
                "ArrayList" | "LinkedList" | "HashSet" | "Set" | "Vector"
            ) {
                let items = crate::builtins::iterate(interp, v)?;
                let jvm = interp
                    .jvm()
                    .ok_or_else(|| Error::eval("No JVM available"))?;
                let created = jvm
                    .new_instance(interp.runtime_id, "java.util.ArrayList", &[])
                    .map_err(error)?;
                let list = match created {
                    JVal::Obj(o) => o,
                    _ => return Err(Error::eval("Cannot create java.util.ArrayList")),
                };
                for item in items {
                    let jv = to_jval(interp, &item)?;
                    jvm.call(&list, "add", &[jv]).map_err(error)?;
                }
                JVal::Obj(list)
            } else {
                return Err(Error::eval(format!(
                    "Cannot pass host object {} to Java",
                    class_name
                )));
            }
        }
        Value::Lambda(_) | Value::MethodRef(_) => return Err(Error::eval(
            "A lambda or method reference needs a target functional interface when passed to Java",
        )),
        Value::Namespace(ns) => {
            // bsh.This is a Runnable whose run() calls the namespace's run().
            let target = crate::proxy::ProxyTarget::Namespace(ns.clone());
            let proxy =
                crate::proxy::make_proxy(interp, &["java.lang.Runnable".to_string()], target)?;
            match proxy {
                Value::Java(object) => JVal::Obj(object),
                _ => return Err(Error::eval("Cannot create a Runnable for this namespace")),
            }
        }
        Value::Method(_) => {
            return Err(Error::eval(format!(
                "Cannot convert {} to a Java value",
                v.type_name()
            )))
        }
    })
}

fn component_name(interp: &mut Interpreter, ty: &Option<bsh_syntax::ast::TypeRef>) -> String {
    let ty = match ty {
        Some(t) => t,
        None => return "java.lang.Object".to_string(),
    };
    let name = ty.base_name();
    if ty.primitive.is_some() {
        return name;
    }
    if interp
        .jvm()
        .map(|j| j.class_exists(interp.runtime_id, &name))
        .unwrap_or(false)
        || interp.find_class(&name).is_some()
    {
        return name;
    }
    let candidate = format!("java.lang.{}", name);
    if interp
        .jvm()
        .map(|j| j.class_exists(interp.runtime_id, &candidate))
        .unwrap_or(false)
    {
        return candidate;
    }
    name
}
