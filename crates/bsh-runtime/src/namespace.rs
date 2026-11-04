//! Lexical namespaces: variables, methods and classes.

use crate::class::ClassValue;
use crate::fxhash::FxMap;
use crate::value::{MethodDef, Value};
use bsh_syntax::ast::TypeRef;
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

#[derive(Clone, Debug)]
pub struct Variable {
    pub value: Value,
    pub ty: Option<TypeRef>,
    pub is_final: bool,
    pub is_static: bool,
}

#[derive(Debug)]
pub struct Namespace {
    pub parent: Option<Rc<RefCell<Namespace>>>,
    pub vars: FxMap<String, Variable>,
    pub methods: HashMap<String, Vec<Rc<MethodDef>>>,
    pub classes: HashMap<String, Rc<ClassValue>>,
    pub imports: Vec<String>,
    pub this_obj: Option<Value>,
    pub name: Option<String>,
}

impl Namespace {
    pub fn root() -> Rc<RefCell<Namespace>> {
        Rc::new(RefCell::new(Namespace {
            parent: None,
            vars: FxMap::default(),
            methods: HashMap::new(),
            classes: HashMap::new(),
            imports: Vec::new(),
            this_obj: None,
            name: Some("global".to_string()),
        }))
    }

    pub fn child(parent: &Rc<RefCell<Namespace>>) -> Rc<RefCell<Namespace>> {
        Rc::new(RefCell::new(Namespace {
            parent: Some(parent.clone()),
            vars: FxMap::default(),
            methods: HashMap::new(),
            classes: HashMap::new(),
            imports: Vec::new(),
            this_obj: None,
            name: None,
        }))
    }

    /// Fast path for reads: clone only the value, not the declared type.
    pub fn lookup_value(ns: &Rc<RefCell<Namespace>>, name: &str) -> Option<Value> {
        let mut cur = Some(ns.clone());
        while let Some(n) = cur {
            let b = n.borrow();
            if let Some(v) = b.vars.get(name) {
                return Some(v.value.clone());
            }
            cur = b.parent.clone();
        }
        None
    }

    pub fn lookup_var(ns: &Rc<RefCell<Namespace>>, name: &str) -> Option<Variable> {
        let mut cur = Some(ns.clone());
        while let Some(n) = cur {
            let b = n.borrow();
            if let Some(v) = b.vars.get(name) {
                return Some(v.clone());
            }
            cur = b.parent.clone();
        }
        None
    }

    /// Find the namespace that owns a variable (for assignment).
    pub fn owner_of(ns: &Rc<RefCell<Namespace>>, name: &str) -> Option<Rc<RefCell<Namespace>>> {
        let mut cur = Some(ns.clone());
        while let Some(n) = cur {
            let has = n.borrow().vars.contains_key(name);
            if has {
                return Some(n);
            }
            cur = n.borrow().parent.clone();
        }
        None
    }

    pub fn get_this(ns: &Rc<RefCell<Namespace>>) -> Option<Value> {
        let mut cur = Some(ns.clone());
        while let Some(n) = cur {
            let t = n.borrow().this_obj.clone();
            if t.is_some() {
                return t;
            }
            cur = n.borrow().parent.clone();
        }
        None
    }

    pub fn find_method(
        ns: &Rc<RefCell<Namespace>>,
        name: &str,
        argc: usize,
    ) -> Option<Rc<MethodDef>> {
        let mut cur = Some(ns.clone());
        while let Some(n) = cur {
            let found = {
                let b = n.borrow();
                b.methods.get(name).and_then(|list| {
                    list.iter()
                        .find(|m| {
                            m.params.len() == argc || crate::value::has_varargs(&m.params, argc)
                        })
                        .cloned()
                })
            };
            if found.is_some() {
                return found;
            }
            cur = n.borrow().parent.clone();
        }
        None
    }

    pub fn find_class(ns: &Rc<RefCell<Namespace>>, name: &str) -> Option<Rc<ClassValue>> {
        let mut cur = Some(ns.clone());
        while let Some(n) = cur {
            let found = n.borrow().classes.get(name).cloned();
            if found.is_some() {
                return found;
            }
            cur = n.borrow().parent.clone();
        }
        None
    }
}
