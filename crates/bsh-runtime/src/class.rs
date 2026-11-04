//! Classes: scripted class definitions and builtin class handles.

use crate::namespace::Namespace;
use crate::value::{FieldDef, MethodDef, Value};
use bsh_syntax::ast::Stmt;
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

#[derive(Debug)]
pub struct ClassDef {
    pub name: String,
    pub is_interface: bool,
    pub methods: Vec<Rc<MethodDef>>,
    pub fields: Vec<FieldDef>,
    /// Statements executing in the instance namespace when an object is built
    /// (loose variables, initializer blocks).
    pub instance_init: Vec<Stmt>,
    pub static_init: Vec<Stmt>,
    pub enum_constants: Vec<(String, Value)>,
}

#[derive(Debug)]
pub struct ClassValue {
    pub name: String,
    pub superclass: Option<Rc<ClassValue>>,
    pub interfaces: Vec<String>,
    pub def: Option<Rc<ClassDef>>,
    pub statics: RefCell<HashMap<String, Value>>,
    pub is_interface: bool,
    /// Set for classes that are implemented by the Rust host layer.
    pub builtin: Option<&'static str>,
}

impl ClassValue {
    pub fn new_scripted(
        name: impl Into<String>,
        def: Rc<ClassDef>,
        superclass: Option<Rc<ClassValue>>,
    ) -> Self {
        ClassValue {
            name: name.into(),
            superclass,
            interfaces: Vec::new(),
            is_interface: def.is_interface,
            def: Some(def),
            statics: RefCell::new(HashMap::new()),
            builtin: None,
        }
    }

    pub fn new_builtin(
        name: impl Into<String>,
        kind: &'static str,
        superclass: Option<Rc<ClassValue>>,
    ) -> Self {
        ClassValue {
            name: name.into(),
            superclass,
            interfaces: Vec::new(),
            is_interface: false,
            def: None,
            statics: RefCell::new(HashMap::new()),
            // An empty kind means "known name, no Rust implementation yet";
            // such classes fall back to the embedded JVM when used.
            builtin: Some(kind),
        }
    }

    pub fn is_a(&self, name: &str) -> bool {
        if self.name == name {
            return true;
        }
        let simple = self.name.rsplit('.').next().unwrap_or(&self.name);
        let target_simple = name.rsplit('.').next().unwrap_or(name);
        if simple == target_simple {
            return true;
        }
        if self.interfaces.iter().any(|i| i == name) {
            return true;
        }
        match &self.superclass {
            Some(s) => s.is_a(name),
            None => false,
        }
    }

    pub fn find_method(&self, name: &str, argc: usize) -> Option<Rc<MethodDef>> {
        if let Some(def) = &self.def {
            if let Some(m) = def.find_method(name, argc) {
                return Some(m.clone());
            }
        }
        match &self.superclass {
            Some(s) => s.find_method(name, argc),
            None => None,
        }
    }

    pub fn all_methods(&self, name: &str) -> Vec<Rc<MethodDef>> {
        let mut out = Vec::new();
        if let Some(def) = &self.def {
            for m in &def.methods {
                if m.name == name {
                    out.push(m.clone());
                }
            }
        }
        if let Some(s) = &self.superclass {
            out.extend(s.all_methods(name));
        }
        out
    }

    pub fn ctor(&self, argc: usize) -> Option<Rc<MethodDef>> {
        match &self.def {
            Some(def) => def
                .methods
                .iter()
                .find(|m| {
                    m.is_ctor
                        && (m.params.len() == argc || crate::value::has_varargs(&m.params, argc))
                })
                .cloned(),
            None => None,
        }
    }

    pub fn find_field(&self, name: &str) -> Option<FieldDef> {
        if let Some(def) = &self.def {
            if let Some(f) = def.fields.iter().find(|f| f.name == name) {
                return Some(FieldDef {
                    name: f.name.clone(),
                    ty: f.ty.clone(),
                    modifiers: f.modifiers,
                    init: f.init.clone(),
                });
            }
        }
        match &self.superclass {
            Some(s) => s.find_field(name),
            None => None,
        }
    }

    pub fn static_get(&self, name: &str) -> Option<Value> {
        if let Some(v) = self.statics.borrow().get(name) {
            return Some(v.clone());
        }
        match &self.superclass {
            Some(s) => s.static_get(name),
            None => None,
        }
    }

    pub fn namespace(&self) -> Option<Rc<RefCell<Namespace>>> {
        self.def
            .as_ref()
            .and_then(|d| d.methods.first())
            .and_then(|m| m.ns.clone())
    }
}
