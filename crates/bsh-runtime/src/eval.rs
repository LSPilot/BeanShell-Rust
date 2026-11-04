//! The tree walking evaluator.

use crate::builtins;
use crate::class::ClassValue;
use crate::error::{Error, Result};
use crate::interp::{default_value, Interpreter, Ns};
use crate::namespace::Namespace;
use crate::primitive::{self, PrimKind};
use crate::value::{ArrayValue, MethodDef, ObjectValue, Value};
use bsh_syntax::ast::*;
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use std::time::{Duration, Instant};

pub enum Flow {
    Normal,
    Break(Option<String>),
    Continue(Option<String>),
    Return(Value),
    Yield(Value),
}

impl Interpreter {
    // ------------------------------------------------------------ statements

    pub fn exec_stmt(&mut self, stmt: &Stmt, ns: &Ns) -> Result<Flow> {
        self.current_span = Some(stmt.span);
        match &stmt.kind {
            StmtKind::Empty => Ok(Flow::Normal),
            StmtKind::Block(stmts) => {
                let child = Namespace::child(ns);
                self.exec_block(stmts, &child)
            }
            StmtKind::Expr(e) => {
                let v = self.eval(e, ns)?;
                self.last_value = v;
                Ok(Flow::Normal)
            }
            StmtKind::VarDecl(vd) => {
                self.exec_var_decl(vd, ns)?;
                Ok(Flow::Normal)
            }
            StmtKind::Method(md) => {
                let m = self.make_method(
                    ns,
                    &md.name,
                    md.params.clone(),
                    md.body.clone(),
                    md.return_type.clone(),
                    md.modifiers,
                    md.is_ctor,
                    None,
                );
                self.define_method(ns, m);
                Ok(Flow::Normal)
            }
            StmtKind::Class(cd) => {
                self.exec_class(cd, ns)?;
                Ok(Flow::Normal)
            }
            StmtKind::If {
                cond,
                then_branch,
                else_branch,
            } => {
                if self.eval(cond, ns)?.as_bool()? {
                    self.exec_stmt(then_branch, ns)
                } else if let Some(e) = else_branch {
                    self.exec_stmt(e, ns)
                } else {
                    Ok(Flow::Normal)
                }
            }
            StmtKind::While { cond, body } => {
                // Reuse one body namespace instead of allocating per iteration.
                let body_ns = Namespace::child(ns);
                loop {
                    if !self.eval(cond, ns)?.as_bool()? {
                        break;
                    }
                    let flow = match &body.kind {
                        StmtKind::Block(stmts) => self.exec_block(stmts, &body_ns)?,
                        _ => self.exec_stmt(body, &body_ns)?,
                    };
                    match flow {
                        Flow::Normal => {}
                        Flow::Continue(None) => continue,
                        Flow::Continue(Some(_)) => continue,
                        Flow::Break(None) => break,
                        Flow::Break(Some(l)) => return Ok(Flow::Break(Some(l))),
                        Flow::Return(v) => return Ok(Flow::Return(v)),
                        Flow::Yield(v) => return Ok(Flow::Yield(v)),
                    }
                }
                Ok(Flow::Normal)
            }
            StmtKind::DoWhile { body, cond } => {
                let body_ns = Namespace::child(ns);
                loop {
                    let flow = match &body.kind {
                        StmtKind::Block(stmts) => self.exec_block(stmts, &body_ns)?,
                        _ => self.exec_stmt(body, &body_ns)?,
                    };
                    match flow {
                        Flow::Normal | Flow::Continue(None) => {}
                        Flow::Continue(Some(l)) => return Ok(Flow::Continue(Some(l))),
                        Flow::Break(None) => break,
                        Flow::Break(Some(l)) => return Ok(Flow::Break(Some(l))),
                        Flow::Return(v) => return Ok(Flow::Return(v)),
                        Flow::Yield(v) => return Ok(Flow::Yield(v)),
                    }
                    if !self.eval(cond, ns)?.as_bool()? {
                        break;
                    }
                }
                Ok(Flow::Normal)
            }
            StmtKind::For {
                init,
                cond,
                update,
                body,
            } => {
                let child = Namespace::child(ns);
                if let Some(init) = init {
                    match self.exec_stmt(init, &child)? {
                        Flow::Normal => {}
                        Flow::Return(v) => return Ok(Flow::Return(v)),
                        _ => {}
                    }
                }
                let body_ns = Namespace::child(&child);
                loop {
                    if let Some(c) = cond {
                        if !self.eval(c, &child)?.as_bool()? {
                            break;
                        }
                    }
                    let flow = match &body.kind {
                        StmtKind::Block(stmts) => self.exec_block(stmts, &body_ns)?,
                        _ => self.exec_stmt(body, &body_ns)?,
                    };
                    match flow {
                        Flow::Normal | Flow::Continue(None) => {}
                        Flow::Continue(Some(l)) => return Ok(Flow::Continue(Some(l))),
                        Flow::Break(None) => break,
                        Flow::Break(Some(l)) => return Ok(Flow::Break(Some(l))),
                        Flow::Return(v) => return Ok(Flow::Return(v)),
                        Flow::Yield(v) => return Ok(Flow::Yield(v)),
                    }
                    for u in update {
                        self.eval(u, &child)?;
                    }
                }
                Ok(Flow::Normal)
            }
            StmtKind::ForEach {
                ty,
                name,
                iter,
                body,
            } => {
                let iterable = self.eval(iter, ns)?;
                let items = self.iterate(&iterable)?;
                let child = Namespace::child(ns);
                let body_ns = Namespace::child(&child);
                // Java 10 "var" in for-each heads.
                let declared_ty = ty
                    .as_ref()
                    .filter(|t| !(t.dims == 0 && t.base_name() == "var"))
                    .cloned();
                for item in items {
                    let value = match &declared_ty {
                        Some(t) => self.cast_value(item, t)?,
                        None => item,
                    };
                    child.borrow_mut().vars.insert(
                        name.clone(),
                        crate::namespace::Variable {
                            value,
                            ty: declared_ty.clone(),
                            is_final: false,
                            is_static: false,
                        },
                    );
                    let flow = match &body.kind {
                        StmtKind::Block(stmts) => self.exec_block(stmts, &body_ns)?,
                        _ => self.exec_stmt(body, &body_ns)?,
                    };
                    match flow {
                        Flow::Normal | Flow::Continue(None) => {}
                        Flow::Continue(Some(l)) => return Ok(Flow::Continue(Some(l))),
                        Flow::Break(None) => break,
                        Flow::Break(Some(l)) => return Ok(Flow::Break(Some(l))),
                        Flow::Return(v) => return Ok(Flow::Return(v)),
                        Flow::Yield(v) => return Ok(Flow::Yield(v)),
                    }
                }
                Ok(Flow::Normal)
            }
            StmtKind::Yield(e) => {
                let v = self.eval(e, ns)?;
                Ok(Flow::Yield(v))
            }
            StmtKind::Switch { selector, arms } => {
                let sel = self.eval(selector, ns)?;
                let mut start: Option<usize> = None;
                for (i, arm) in arms.iter().enumerate() {
                    if let Some(pattern) = &arm.pattern {
                        if self.switch_pattern_matches(&sel, pattern) {
                            start = Some(i);
                            break;
                        }
                    } else if let Some(label) = &arm.label {
                        let lv = self.eval(label, ns)?;
                        if loose_equals(&sel, &lv) {
                            start = Some(i);
                            break;
                        }
                    }
                }
                let start = match start {
                    Some(i) => Some(i),
                    None => arms
                        .iter()
                        .position(|a| a.label.is_none() && a.pattern.is_none()),
                };
                if let Some(start) = start {
                    let child = Namespace::child(ns);
                    if let Some(pattern) = &arms[start].pattern {
                        if let Some(name) = &pattern.name {
                            child.borrow_mut().vars.insert(
                                name.clone(),
                                crate::namespace::Variable {
                                    value: sel.clone(),
                                    ty: None,
                                    is_final: false,
                                    is_static: false,
                                },
                            );
                        }
                        for (_, cname) in &pattern.components {
                            let v = self.get_member(&sel, cname, &child)?;
                            child.borrow_mut().vars.insert(
                                cname.clone(),
                                crate::namespace::Variable {
                                    value: v,
                                    ty: None,
                                    is_final: false,
                                    is_static: false,
                                },
                            );
                        }
                    }
                    for arm in &arms[start..] {
                        for s in &arm.stmts {
                            match self.exec_stmt(s, &child)? {
                                Flow::Normal => {}
                                Flow::Break(None) => return Ok(Flow::Normal),
                                Flow::Break(Some(l)) => return Ok(Flow::Break(Some(l))),
                                Flow::Continue(c) => return Ok(Flow::Continue(c)),
                                Flow::Return(v) => return Ok(Flow::Return(v)),
                                Flow::Yield(v) => return Ok(Flow::Yield(v)),
                            }
                        }
                        if arm.arrow {
                            break;
                        }
                    }
                }
                Ok(Flow::Normal)
            }
            StmtKind::Try {
                body,
                catches,
                finally,
            } => {
                let result = self.exec_stmt(body, ns);
                let mut flow = match result {
                    Ok(f) => f,
                    Err(Error::Thrown(v)) => return self.handle_throw(v, catches, finally, ns),
                    Err(Error::Eval(msg)) => {
                        let v = self.make_exception("bsh.EvalError", &msg);
                        return self.handle_throw(v, catches, finally, ns);
                    }
                    Err(e) => return Err(e),
                };
                if let Some(fin) = finally {
                    match self.exec_stmt(fin, ns)? {
                        Flow::Normal => {}
                        other => flow = other,
                    }
                }
                Ok(flow)
            }
            StmtKind::Assert { cond, message } => {
                if !self.eval(cond, ns)?.as_bool()? {
                    let text = match message {
                        Some(message) => self.eval(message, ns)?.to_display_string(),
                        None => "assertion failed".to_string(),
                    };
                    return Err(self.assertion_failure(&text));
                }
                Ok(Flow::Normal)
            }
            StmtKind::Throw(e) => {
                let v = self.eval(e, ns)?;
                Err(Error::Thrown(v))
            }
            StmtKind::Return(e) => {
                let v = match e {
                    Some(e) => self.eval(e, ns)?,
                    None => Value::Void,
                };
                Ok(Flow::Return(v))
            }
            StmtKind::Break(l) => Ok(Flow::Break(l.clone())),
            StmtKind::Continue(l) => Ok(Flow::Continue(l.clone())),
            StmtKind::Labeled { label, body } => match self.exec_stmt(body, ns)? {
                Flow::Break(Some(l)) if &l == label => Ok(Flow::Normal),
                Flow::Continue(Some(l)) if &l == label => Ok(Flow::Normal),
                other => Ok(other),
            },
            StmtKind::Synchronized { lock, body } => {
                self.eval(lock, ns)?;
                self.exec_stmt(body, ns)
            }
            StmtKind::Import(imp) => {
                let name = imp.name.join(".");
                if imp.is_static {
                    let (class, member) = if imp.wildcard {
                        (name, None)
                    } else {
                        let mut parts = imp.name.clone();
                        let member = parts.pop().unwrap_or_default();
                        (parts.join("."), Some(member))
                    };
                    self.static_imports.push((class, member));
                    return Ok(Flow::Normal);
                }
                // Keep the wildcard marker: resolve_class looks for "pkg.*".
                let import_name = if imp.wildcard {
                    format!("{}.*", name)
                } else {
                    name.clone()
                };
                ns.borrow_mut().imports.push(import_name);
                // Import a concrete class so short names resolve. resolve_class
                // also probes the JVM (and nested Outer.Inner forms).
                if !imp.wildcard {
                    if let Some(class) = self.resolve_class(&name) {
                        let simple = class
                            .name
                            .rsplit(['.', '$'])
                            .next()
                            .unwrap_or(&class.name)
                            .to_string();
                        ns.borrow_mut().classes.insert(simple, class);
                    }
                }
                Ok(Flow::Normal)
            }
            StmtKind::Package(_) => Ok(Flow::Normal),
        }
    }

    fn exec_block(&mut self, stmts: &[Stmt], ns: &Ns) -> Result<Flow> {
        for s in stmts {
            match self.exec_stmt(s, ns)? {
                Flow::Normal => {}
                other => return Ok(other),
            }
        }
        Ok(Flow::Normal)
    }

    fn exec_class(&mut self, cd: &ClassDecl, ns: &Ns) -> Result<()> {
        let class = self.define_class(ns, cd)?;
        // Run static initializer statements in a class namespace.
        let cns = Namespace::child(ns);
        cns.borrow_mut().this_obj = Some(Value::Class(class.clone()));
        if let Some(def) = &class.def {
            for s in &def.static_init.clone() {
                let _ = self.exec_stmt(s, &cns);
            }
        }
        Ok(())
    }

    fn exec_var_decl(&mut self, vd: &VarDecl, ns: &Ns) -> Result<()> {
        // Java 10 "var": infer the type from the initializer.
        let ty = vd
            .ty
            .clone()
            .filter(|t| !(t.dims == 0 && t.base_name() == "var"));
        for var in &vd.vars {
            let value = match &var.init {
                Some(e) => {
                    let v = self.eval(e, ns)?;
                    match &ty {
                        Some(t) => self.cast_value(v, t)?,
                        None => v,
                    }
                }
                None => default_value(ty.as_ref()),
            };
            // Java 22 unnamed variable: evaluate but do not bind.
            if var.name == "_" {
                continue;
            }
            // Re-declaring with the same type replaces the value in place.
            ns.borrow_mut().vars.insert(
                var.name.clone(),
                crate::namespace::Variable {
                    value,
                    ty: ty.clone(),
                    is_final: vd.modifiers.is_final,
                    is_static: vd.modifiers.is_static,
                },
            );
        }
        Ok(())
    }

    fn handle_throw(
        &mut self,
        thrown: Value,
        catches: &[CatchClause],
        finally: &Option<Box<Stmt>>,
        ns: &Ns,
    ) -> Result<Flow> {
        // Remember where the exception came from: running finally must not
        // change the reported location of the original exception.
        let thrown_span = self.current_span;
        for clause in catches {
            let matches = if clause.types.is_empty() {
                true
            } else {
                clause
                    .types
                    .iter()
                    .any(|t| self.matches_exception(&thrown, &t.base_name()))
            };
            if matches {
                let child = Namespace::child(ns);
                child.borrow_mut().vars.insert(
                    clause.name.clone(),
                    crate::namespace::Variable {
                        value: thrown,
                        ty: None,
                        is_final: false,
                        is_static: false,
                    },
                );
                let flow = self.exec_stmt(&clause.body, &child)?;
                let flow = if let Some(fin) = finally {
                    match self.exec_stmt(fin, ns)? {
                        Flow::Normal => flow,
                        other => other,
                    }
                } else {
                    flow
                };
                return Ok(flow);
            }
        }
        if let Some(fin) = finally {
            let _ = self.exec_stmt(fin, ns)?;
        }
        self.current_span = thrown_span;
        Err(Error::Thrown(thrown))
    }

    fn switch_pattern_matches(
        &mut self,
        value: &Value,
        pattern: &bsh_syntax::ast::SwitchPattern,
    ) -> bool {
        let display = pattern.ty.display_name();
        let resolved = Namespace::find_class(&self.global, &display)
            .or_else(|| self.resolve_class(&display))
            .map(|class| class.name.clone())
            .unwrap_or(display);
        self.value_instance_of(value, &resolved)
    }

    fn value_instance_of(&mut self, value: &Value, name: &str) -> bool {
        match value {
            Value::Java(object) => self
                .jvm()
                .map(|jvm| jvm.is_instance(self.runtime_id, object, name))
                .unwrap_or(false),
            Value::Object(object) => {
                // Scripted classes are only known to the Rust side; the JVM
                // class loader used by the bridge cannot resolve them by name.
                if object.borrow().class.is_a(name) {
                    return true;
                }
                let java = object.borrow().java.clone();
                match java {
                    Some(java) => self
                        .jvm()
                        .map(|jvm| jvm.is_instance(self.runtime_id, &java, name))
                        .unwrap_or(false),
                    None => false,
                }
            }
            _ => value.instance_of(name),
        }
    }

    fn matches_exception(&self, thrown: &Value, type_name: &str) -> bool {
        if let Value::Java(o) = thrown {
            let jvm = o.jvm();
            if jvm.is_instance(self.runtime_id, o, type_name) {
                return true;
            }
            // "catch (Throwable)" / "catch (Exception)" use simple names.
            let simple = type_name.rsplit('.').next().unwrap_or(type_name);
            if o.class_name() == type_name || o.class_name().rsplit('.').next() == Some(simple) {
                return true;
            }
            if !type_name.contains('.') {
                for prefix in ["java.lang.", "java.util.", "bsh."] {
                    if jvm.is_instance(self.runtime_id, o, &format!("{}{}", prefix, type_name)) {
                        return true;
                    }
                }
            }
            return false;
        }
        if let Value::Object(o) = thrown {
            let class = o.borrow().class.clone();
            if class.is_a(type_name) {
                return true;
            }
            if !type_name.contains('.') {
                return class.is_a(&format!("java.lang.{}", type_name));
            }
            return false;
        }
        matches!(
            type_name,
            "Object"
                | "java.lang.Object"
                | "Throwable"
                | "Exception"
                | "EvalError"
                | "bsh.EvalError"
        )
    }

    pub fn make_exception(&mut self, class_name: &str, message: &str) -> Value {
        // Prefer a real Java throwable so scripts can read e.getMessage()
        // and log e.toString() with the actual error text.
        if let Some(jvm) = self.jvm() {
            let argument = bsh_jvm::JVal::Str(message.to_string());
            if let Ok(value) =
                jvm.new_instance(self.runtime_id, class_name, std::slice::from_ref(&argument))
            {
                return crate::java::from_jval(value);
            }
            if let Ok(value) = jvm.new_instance(
                self.runtime_id,
                "java.lang.RuntimeException",
                std::slice::from_ref(&argument),
            ) {
                return crate::java::from_jval(value);
            }
        }
        let class = match self.find_class(class_name) {
            Some(c) => c,
            None => match self.find_class("RuntimeException") {
                Some(c) => c,
                None => return Value::string(message.to_string()),
            },
        };
        let mut fields = HashMap::new();
        fields.insert("message".to_string(), Value::string(message.to_string()));
        Value::Object(Rc::new(RefCell::new(ObjectValue {
            class,
            fields,
            java: None,
        })))
    }

    // ----------------------------------------------------------- expressions

    pub fn eval(&mut self, e: &Expr, ns: &Ns) -> Result<Value> {
        self.current_span = Some(e.span);
        match &e.kind {
            ExprKind::Literal(l) => Ok(literal_value(l)),
            ExprKind::Name(parts) => self.eval_name(parts, ns),
            ExprKind::Field(base, name) => {
                let b = self.eval(base, ns)?;
                self.get_member(&b, name, ns)
            }
            ExprKind::SafeField(base, name) => {
                let b = self.eval(base, ns)?;
                if b.is_null_like() {
                    Ok(Value::Null)
                } else {
                    self.get_member(&b, name, ns)
                }
            }
            ExprKind::Index(base, idx) => {
                let b = self.eval(base, ns)?;
                let i = self.eval(idx, ns)?;
                self.get_index(&b, &i)
            }
            ExprKind::Slice { base, parts } => {
                let b = self.eval(base, ns)?;
                let mut bounds = Vec::new();
                for p in parts {
                    bounds.push(match p {
                        Some(e) => Some(self.eval(e, ns)?.as_i64()?),
                        None => None,
                    });
                }
                self.slice(&b, &bounds)
            }
            ExprKind::Call { target, args } => self.eval_call(target, args, ns),
            ExprKind::NewArray { ty, dims, init } => self.eval_new_array(ty, dims, init, ns),
            ExprKind::New { ty, args, anon } => self.eval_new(ty, args, anon.as_deref(), ns),
            ExprKind::NewInner { ty, args, .. } => self.eval_new(ty, args, None, ns),
            ExprKind::Cast(inner, ty) => {
                let v = self.eval(inner, ns)?;
                self.cast_value(v, ty)
            }
            ExprKind::InstanceOf(inner, ty) => {
                let v = self.eval(inner, ns)?;
                let display = ty.display_name();
                // Resolve imports/classloaders first: "instanceof ViewGroup"
                // must test android.view.ViewGroup, not the bare simple name.
                let resolved = Namespace::find_class(ns, &display)
                    .or_else(|| self.resolve_class(&display))
                    .map(|class| class.name.clone())
                    .unwrap_or(display);
                Ok(Value::Bool(self.value_instance_of(&v, &resolved)))
            }
            ExprKind::InstanceOfPattern(inner, ty, name) => {
                let v = self.eval(inner, ns)?;
                let display = ty.display_name();
                let resolved = Namespace::find_class(ns, &display)
                    .or_else(|| self.resolve_class(&display))
                    .map(|class| class.name.clone())
                    .unwrap_or(display);
                let matched = self.value_instance_of(&v, &resolved);
                if matched {
                    ns.borrow_mut().vars.insert(
                        name.clone(),
                        crate::namespace::Variable {
                            value: v,
                            ty: None,
                            is_final: false,
                            is_static: false,
                        },
                    );
                }
                Ok(Value::Bool(matched))
            }
            ExprKind::SwitchExpr { selector, arms } => {
                let sel = self.eval(selector, ns)?;
                let mut matched = None;
                for arm in arms.iter() {
                    if let Some(pattern) = &arm.pattern {
                        if self.switch_pattern_matches(&sel, pattern) {
                            matched = Some(arm);
                            break;
                        }
                    } else if let Some(label) = &arm.label {
                        let lv = self.eval(label, ns)?;
                        if loose_equals(&sel, &lv) {
                            matched = Some(arm);
                            break;
                        }
                    }
                }
                let arm = matched.or_else(|| {
                    arms.iter()
                        .find(|arm| arm.label.is_none() && arm.pattern.is_none())
                });
                let arm = match arm {
                    Some(arm) => arm,
                    None => return Err(Error::eval("switch expression has no matching arm")),
                };
                let child = Namespace::child(ns);
                if let Some(pattern) = &arm.pattern {
                    if let Some(name) = &pattern.name {
                        child.borrow_mut().vars.insert(
                            name.clone(),
                            crate::namespace::Variable {
                                value: sel.clone(),
                                ty: None,
                                is_final: false,
                                is_static: false,
                            },
                        );
                    }
                    for (_, cname) in &pattern.components {
                        let v = self.get_member(&sel, cname, &child)?;
                        child.borrow_mut().vars.insert(
                            cname.clone(),
                            crate::namespace::Variable {
                                value: v,
                                ty: None,
                                is_final: false,
                                is_static: false,
                            },
                        );
                    }
                }
                match &arm.body {
                    bsh_syntax::ast::SwitchExprBody::Expr(e) => self.eval(e, &child),
                    bsh_syntax::ast::SwitchExprBody::Block(stmts) => {
                        match self.exec_block(stmts, &child)? {
                            Flow::Yield(v) => Ok(v),
                            Flow::Normal => Err(Error::eval("switch expression arm did not yield")),
                            Flow::Return(v) => Err(Error::eval(format!(
                                "return inside switch expression: {}",
                                v.to_display_string()
                            ))),
                            Flow::Break(_) => Err(Error::eval("break inside switch expression")),
                            Flow::Continue(_) => {
                                Err(Error::eval("continue inside switch expression"))
                            }
                        }
                    }
                }
            }
            ExprKind::Unary { op, expr } => self.eval_unary(*op, expr, ns),
            ExprKind::Binary { op, lhs, rhs } => self.eval_binary(*op, lhs, rhs, ns),
            ExprKind::Assign { op, lhs, rhs } => self.eval_assign(*op, lhs, rhs, ns),
            ExprKind::Ternary {
                cond,
                then_expr,
                else_expr,
            } => {
                if self.eval(cond, ns)?.as_bool()? {
                    self.eval(then_expr, ns)
                } else {
                    self.eval(else_expr, ns)
                }
            }
            ExprKind::Elvis { cond, else_expr } => {
                let v = self.eval(cond, ns)?;
                match &v {
                    Value::Null | Value::Void => self.eval(else_expr, ns),
                    Value::Bool(false) => self.eval(else_expr, ns),
                    _ => Ok(v),
                }
            }
            ExprKind::ArrayInit(items) => {
                let mut vals = Vec::with_capacity(items.len());
                for i in items {
                    vals.push(self.eval(i, ns)?);
                }
                Ok(Value::Array(Rc::new(RefCell::new(ArrayValue {
                    dims: 1,
                    component: None,
                    items: vals,
                }))))
            }
            ExprKind::This => Ok(self.this_value(ns)),
            ExprKind::Super => Ok(self.this_value(ns)),
            ExprKind::Lambda { params, body } => {
                let method = Rc::new(MethodDef {
                    name: "lambda".to_string(),
                    params: params.clone(),
                    body: Some(body.clone()),
                    return_type: None,
                    ns: Some(ns.clone()),
                    declaring_class: None,
                    is_ctor: false,
                    modifiers: Default::default(),
                    source_file: self.source_file.clone(),
                    source_text: self.source_text.clone(),
                });
                Ok(Value::Lambda(Rc::new(crate::value::LambdaValue {
                    method,
                    source_file: self.source_file.clone(),
                    source_text: self.source_text.clone(),
                })))
            }
            ExprKind::MethodRef(base, name) => {
                let target = match self.eval(base, ns) {
                    Ok(value) => crate::value::MethodRefTarget::Value(value),
                    Err(_) => crate::value::MethodRefTarget::Deferred(base.clone(), ns.clone()),
                };
                Ok(Value::MethodRef(Rc::new(crate::value::MethodRefValue {
                    target,
                    name: name.clone(),
                })))
            }
        }
    }

    fn this_value(&self, ns: &Ns) -> Value {
        match Namespace::get_this(ns) {
            Some(v) => v,
            None => Value::Namespace(ns.clone()),
        }
    }

    pub fn eval_name(&mut self, parts: &[String], ns: &Ns) -> Result<Value> {
        if parts.len() == 1 {
            let name = &parts[0];
            if let Some(value) = Namespace::lookup_value(ns, name) {
                return Ok(value);
            }
            // Fields of the current object and static fields of the class.
            if let Some(this) = Namespace::get_this(ns) {
                match &this {
                    Value::Object(o) => {
                        let o = o.borrow();
                        if let Some(v) = o.fields.get(name) {
                            return Ok(v.clone());
                        }
                        if o.class.find_field(name).is_some() {
                            return Ok(Value::Null);
                        }
                        if let Some(java) = &o.java {
                            let jvm = java.jvm().clone();
                            match jvm.get_field(java, name) {
                                Ok(v) => return Ok(crate::java::from_jval(v)),
                                Err(bsh_jvm::JError::Thrown(t)) => {
                                    return Err(Error::Thrown(Value::Java(t)))
                                }
                                Err(_) => {}
                            }
                        }
                    }
                    Value::Class(c) => {
                        if let Some(v) = c.static_get(name) {
                            return Ok(v);
                        }
                    }
                    _ => {}
                }
            }
            if let Some(c) = self.resolve_class(name) {
                return Ok(Value::Class(c));
            }
            if name.ends_with("[]") || is_primitive_name(name) {
                let c = self.class_for_name(name);
                return Ok(Value::Class(c));
            }
            if let Some(value) = self.static_import_field(name) {
                return Ok(value);
            }
            return Err(Error::eval(format!(
                "Attempt to resolve method: {}() on undefined variable or class name: {}",
                name, name
            )));
        }
        // A variable shadows any class with the same first segment. Checking
        // values first also avoids a JVM class lookup on every field access.
        if let Some(value) = Namespace::lookup_value(ns, &parts[0]) {
            let mut val = value;
            for p in &parts[1..] {
                val = self.get_member(&val, p, ns)?;
            }
            return Ok(val);
        }
        // Longest class prefix wins: java.util.ArrayList, System.out, ...
        for k in (1..=parts.len()).rev() {
            let full = parts[..k].join(".");
            if let Some(c) = self.resolve_class(&full) {
                let mut val = Value::Class(c);
                for p in &parts[k..] {
                    val = self.get_member(&val, p, ns)?;
                }
                return Ok(val);
            }
        }
        Err(Error::eval(format!(
            "Attempt to resolve method: {}() on undefined variable or class name: {}",
            parts.join("."),
            parts[0]
        )))
    }

    fn assertion_failure(&mut self, text: &str) -> Error {
        // bsh itself treats "assert" as a no-op; we evaluate it and report a
        // normal EvalError so the failure is visible and catchable.
        Error::eval(format!("AssertionError: {}", text))
    }

    fn invoke_host_method(
        &mut self,
        name: &str,
        args: Vec<Value>,
        _ns: &Ns,
    ) -> Option<Result<Value>> {
        // Dynamic loaders can make previously missing classes visible.
        if matches!(
            name,
            "loadDex" | "loadApk" | "loadJar" | "loadAar" | "loadJava" | "loadClass"
        ) {
            self.missing_classes.clear();
            self.host_method_cache.clear();
        }
        let argc = args.len();
        let candidates: Vec<(usize, crate::interp::HostMethod)> = match self.host_methods.get(name)
        {
            Some(methods) => methods
                .iter()
                .enumerate()
                .filter(|(_, method)| {
                    method.argc == argc || (method.varargs && argc + 1 >= method.argc)
                })
                .map(|(index, method)| (index, method.clone()))
                .collect(),
            None => return None,
        };
        if candidates.is_empty() {
            return None;
        }
        let jvm = self.jvm()?;
        let need_params = args_need_param_types(&args);
        let cache_key = if need_params {
            Some((
                name.to_string(),
                argc,
                args.iter().map(host_arg_tag).collect::<Vec<String>>(),
            ))
        } else {
            None
        };
        let cached = cache_key
            .as_ref()
            .and_then(|key| self.host_method_cache.get(key))
            .cloned();
        let fallback_candidates = candidates.clone();
        for (host_index, host) in candidates {
            let mut values = None;
            let mut signature_used: Option<Vec<String>> = None;
            if need_params {
                if let Some((cached_index, cached_signature)) = &cached {
                    if *cached_index == host_index {
                        match self.jval_args_typed(Some(cached_signature), &args) {
                            Ok(converted) => {
                                values = Some(converted);
                                signature_used = Some(cached_signature.clone());
                            }
                            Err(_) => {
                                if let Some(key) = &cache_key {
                                    self.host_method_cache.remove(key);
                                }
                            }
                        }
                    }
                }
            }
            if need_params && values.is_none() {
                let signatures = match &host.instance {
                    Some(instance) => jvm.method_overloads(
                        self.runtime_id,
                        Some(instance.as_ref()),
                        &instance.class_name,
                        name,
                        args.len(),
                        false,
                    ),
                    None => jvm.method_overloads(
                        self.runtime_id,
                        None,
                        &host.class_name,
                        name,
                        args.len(),
                        true,
                    ),
                };
                let signatures = match signatures {
                    Some(signatures) => signatures,
                    None => continue,
                };
                let mut best: Option<(i32, Vec<String>)> = None;
                for mut signature in signatures {
                    let is_varargs = signature
                        .last()
                        .map(|value| value == "@@VARARGS@@")
                        .unwrap_or(false);
                    if is_varargs {
                        signature.pop();
                    }
                    let signature = if is_varargs {
                        expand_varargs_signature(&signature, args.len())
                    } else if signature.len() == args.len() {
                        Some(signature)
                    } else {
                        None
                    };
                    let signature = match signature {
                        Some(signature) => signature,
                        None => continue,
                    };
                    if signature.len() != args.len() {
                        continue;
                    }
                    let mut score = 0;
                    let mut valid = true;
                    for (value, parameter) in args.iter().zip(signature.iter()) {
                        match score_host_argument(jvm.as_ref(), self.runtime_id, value, parameter) {
                            Some(value) => score += value,
                            None => {
                                valid = false;
                                break;
                            }
                        }
                    }
                    if valid && best.as_ref().map_or(true, |(current, _)| score > *current) {
                        best = Some((score, signature));
                    }
                }
                let signature = match best {
                    Some((_, signature)) => signature,
                    None => continue,
                };
                match self.jval_args_typed(Some(&signature), &args) {
                    Ok(converted) => {
                        values = Some(converted);
                        signature_used = Some(signature);
                    }
                    Err(_) => continue,
                }
            } else {
                values = Some(match self.jval_args_typed(None, &args) {
                    Ok(converted) => converted,
                    Err(_) => continue,
                });
            }
            let values = match values {
                Some(values) => values,
                None => continue,
            };
            let result = match &host.instance {
                Some(instance) => jvm.call(instance, name, &values),
                None => jvm.call_static(self.runtime_id, &host.class_name, name, &values),
            };
            return Some(match result {
                Ok(value) => {
                    if let (Some(key), Some(signature)) = (cache_key, signature_used) {
                        self.host_method_cache.insert(key, (host_index, signature));
                    }
                    Ok(crate::java::from_jval(value))
                }
                Err(bsh_jvm::JError::Thrown(thrown))
                    if thrown.class_name == "java.lang.NoSuchMethodException" =>
                {
                    if let Some(key) = &cache_key {
                        self.host_method_cache.remove(key);
                    }
                    continue;
                }
                Err(bsh_jvm::JError::Thrown(thrown)) => Err(Error::Thrown(Value::Java(thrown))),
                Err(error) => Err(Error::eval(format!("{}", error))),
            });
        }
        // Compatibility fallback for host methods whose overload signature
        // cannot be represented in the scorer (Kotlin bridge methods,
        // scripted receivers, varargs edge cases). This keeps existing Java
        // hook plugins working while the fast path handles the common cases.
        for (_, host) in fallback_candidates {
            let params = if need_params {
                match &host.instance {
                    Some(instance) => jvm.method_params(
                        self.runtime_id,
                        Some(instance.as_ref()),
                        &instance.class_name,
                        name,
                        args.len(),
                        false,
                    ),
                    None => jvm.method_params(
                        self.runtime_id,
                        None,
                        &host.class_name,
                        name,
                        args.len(),
                        true,
                    ),
                }
            } else {
                None
            };
            if need_params && params.is_none() {
                continue;
            }
            let values = match self.jval_args_typed(params.as_deref(), &args) {
                Ok(values) => values,
                Err(_) => continue,
            };
            let result = match &host.instance {
                Some(instance) => jvm.call(instance, name, &values),
                None => jvm.call_static(self.runtime_id, &host.class_name, name, &values),
            };
            return Some(match result {
                Ok(value) => Ok(crate::java::from_jval(value)),
                Err(bsh_jvm::JError::Thrown(thrown))
                    if thrown.class_name == "java.lang.NoSuchMethodException" =>
                {
                    continue;
                }
                Err(bsh_jvm::JError::Thrown(thrown)) => Err(Error::Thrown(Value::Java(thrown))),
                Err(error) => Err(Error::eval(format!("{}", error))),
            });
        }
        Some(Err(Error::eval(format!(
            "No matching overload for {}()",
            name
        ))))
    }

    fn static_import_field(&mut self, name: &str) -> Option<Value> {
        let imports = self.static_imports.clone();
        for (class, member) in imports {
            if let Some(member_name) = &member {
                if member_name != name {
                    continue;
                }
            }
            if let Some(class) = self.resolve_class(&class) {
                if let Some(value) = class.static_get(name) {
                    if !matches!(
                        value,
                        Value::Method(_)
                            | Value::Builtin(_)
                            | Value::Lambda(_)
                            | Value::MethodRef(_)
                    ) {
                        return Some(value);
                    }
                }
            }
        }
        None
    }

    fn static_import_call(
        &mut self,
        name: &str,
        args: Vec<Value>,
        ns: &Ns,
    ) -> Option<Result<Value>> {
        let imports = self.static_imports.clone();
        for (class, member) in imports {
            if let Some(member_name) = &member {
                if member_name != name {
                    continue;
                }
            }
            let Some(class_value) = self.resolve_class(&class) else {
                continue;
            };
            if class_value.static_get(name).is_some() || class_value.builtin.is_some() {
                match self.call_on_class(&class_value, name, args.clone(), ns, None) {
                    Ok(value) => return Some(Ok(value)),
                    Err(Error::Eval(message)) if message.contains("No such") => continue,
                    Err(error) => return Some(Err(error)),
                }
            }
        }
        None
    }

    pub fn get_member(&mut self, base: &Value, name: &str, _ns: &Ns) -> Result<Value> {
        match base {
            Value::Namespace(n) => {
                if name == "namespace" {
                    return Ok(Value::Namespace(n.clone()));
                }
                if name == "interpreter" {
                    return Ok(Value::Namespace(n.clone()));
                }
                if name == "caller" {
                    let parent = n.borrow().parent.clone();
                    return Ok(match parent {
                        Some(p) => Value::Namespace(p),
                        None => Value::Namespace(n.clone()),
                    });
                }
                if name == "class" {
                    return Ok(base.clone());
                }
                if let Some(value) = Namespace::lookup_value(n, name) {
                    return Ok(value);
                }
                if let Some(c) = Namespace::find_class(n, name) {
                    return Ok(Value::Class(c));
                }
                Err(Error::eval(format!("No such variable: {}", name)))
            }
            Value::Class(c) => {
                if name == "class" {
                    return Ok(Value::Class(c.clone()));
                }
                if let Some(v) = c.static_get(name) {
                    return Ok(v);
                }
                if c.builtin == Some("java") {
                    if let Some(jvm) = self.jvm() {
                        if let Ok(value) = jvm.get_static(self.runtime_id, &c.name, name) {
                            return Ok(crate::java::from_jval(value));
                        }
                        // SomeClass.class.getName() and friends: expose the
                        // java.lang.Class instance method as a callable value.
                        if is_class_literal_method(name) {
                            if let Ok(class_object) = jvm.class_ref(self.runtime_id, &c.name) {
                                return Ok(Value::MethodRef(Rc::new(
                                    crate::value::MethodRefValue {
                                        target: crate::value::MethodRefTarget::Value(Value::Java(
                                            class_object,
                                        )),
                                        name: name.to_string(),
                                    },
                                )));
                            }
                        }
                    }
                }
                if let Some(nested) = self.resolve_class(&format!("{}.{}", c.name, name)) {
                    return Ok(Value::Class(nested));
                }
                if let Some(nested) = self.resolve_class(name) {
                    return Ok(Value::Class(nested));
                }
                if let Some(v) = builtins::static_field(self, c, name) {
                    return v;
                }
                if let Some(jvm) = self.jvm() {
                    match jvm.get_static(self.runtime_id, &c.name, name) {
                        Ok(v) => return Ok(crate::java::from_jval(v)),
                        Err(bsh_jvm::JError::Thrown(t)) => {
                            return Err(Error::Thrown(Value::Java(t)))
                        }
                        Err(_) => {}
                    }
                }
                Err(Error::eval(format!(
                    "No static field {} in class {}",
                    name, c.name
                )))
            }
            Value::Java(o) => {
                if name == "class" {
                    if let Some(class) = self.resolve_class(o.class_name()) {
                        return Ok(Value::Class(class));
                    }
                    let class = o.jvm().class_of(o).map_err(crate::java::error)?;
                    return Ok(Value::Java(class));
                }
                if o.class_name().starts_with('[') && name == "length" {
                    let len = o.jvm().array_len(o).map_err(crate::java::error)?;
                    return Ok(Value::Int(len as i32));
                }
                let jvm = o.jvm().clone();
                match jvm.get_field(o, name) {
                    Ok(v) => Ok(crate::java::from_jval(v)),
                    Err(bsh_jvm::JError::Thrown(t)) => {
                        // Bean-style property read: x.foo -> getFoo() / isFoo().
                        let suffix = capitalize_name(name);
                        for getter in [format!("get{}", suffix), format!("is{}", suffix)] {
                            if let Ok(v) = jvm.call(o, &getter, &[]) {
                                return Ok(crate::java::from_jval(v));
                            }
                        }
                        Err(Error::Thrown(Value::Java(t)))
                    }
                    Err(e) => Err(Error::eval(format!("{}", e))),
                }
            }
            Value::Object(o) => {
                let o = o.borrow();
                if name == "class" {
                    return Ok(Value::Class(o.class.clone()));
                }
                if name == "super" {
                    return Ok(base.clone());
                }
                if let Some(v) = o.fields.get(name) {
                    return Ok(v.clone());
                }
                if o.class.find_field(name).is_some() {
                    return Ok(Value::Null);
                }
                let java = o.java.clone();
                drop(o);
                if let Some(java) = java {
                    let jvm = java.jvm().clone();
                    match jvm.get_field(&java, name) {
                        Ok(v) => return Ok(crate::java::from_jval(v)),
                        Err(bsh_jvm::JError::Thrown(t)) => {
                            return Err(Error::Thrown(Value::Java(t)))
                        }
                        Err(_) => {}
                    }
                }
                if let Some(v) = builtins::object_field(self, base, name) {
                    return v;
                }
                Err(Error::eval(format!("No such field: {}", name)))
            }
            Value::Builtin(b) => b.borrow_mut().get_field(name),
            Value::Array(a) => {
                if name == "length" {
                    return Ok(Value::Int(a.borrow().items.len() as i32));
                }
                Err(Error::eval(format!("No such field: {} on array", name)))
            }
            Value::Str(s) => {
                if name == "length" {
                    return Ok(Value::Int(s.chars().count() as i32));
                }
                Err(Error::eval(format!("No such field: {}", name)))
            }
            Value::Null | Value::Void => Err(Error::eval(format!(
                "Null Pointer: cannot access field '{}' of null",
                name
            ))),
            _ => Err(Error::eval(format!("No such field: {}", name))),
        }
    }

    pub fn set_member(&mut self, base: &Value, name: &str, value: Value) -> Result<()> {
        match base {
            Value::Namespace(n) => {
                if let Some(owner) = Namespace::owner_of(n, name) {
                    owner
                        .borrow_mut()
                        .vars
                        .get_mut(name)
                        .map(|v| v.value = value);
                } else {
                    n.borrow_mut().vars.insert(
                        name.to_string(),
                        crate::namespace::Variable {
                            value,
                            ty: None,
                            is_final: false,
                            is_static: false,
                        },
                    );
                }
                Ok(())
            }
            Value::Object(o) => {
                if o.borrow().fields.contains_key(name) {
                    o.borrow_mut().fields.insert(name.to_string(), value);
                    return Ok(());
                }
                let java = o.borrow().java.clone();
                if let Some(java) = java {
                    let jvm = java.jvm().clone();
                    let jv = crate::java::to_jval(self, &value)?;
                    match jvm.set_field(&java, name, &jv) {
                        Ok(()) => return Ok(()),
                        Err(bsh_jvm::JError::Thrown(t)) => {
                            return Err(Error::Thrown(Value::Java(t)))
                        }
                        Err(_) => {}
                    }
                }
                o.borrow_mut().fields.insert(name.to_string(), value);
                Ok(())
            }
            Value::Builtin(b) => b.borrow_mut().set_field(name, value),
            Value::Class(c) => {
                c.statics.borrow_mut().insert(name.to_string(), value);
                Ok(())
            }
            Value::Java(o) => {
                let jvm = o.jvm().clone();
                let jv = crate::java::to_jval(self, &value)?;
                if jvm.set_field(o, name, &jv).is_ok() {
                    return Ok(());
                }
                // Bean-style property write: x.foo = v -> setFoo(v).
                let setter = format!("set{}", capitalize_name(name));
                match jvm.call(o, &setter, std::slice::from_ref(&jv)) {
                    Ok(_) => Ok(()),
                    Err(bsh_jvm::JError::Thrown(t)) => Err(Error::Thrown(Value::Java(t))),
                    Err(e) => Err(Error::eval(format!("{}", e))),
                }
            }
            Value::Null | Value::Void => {
                Err(Error::eval("Null Pointer: cannot set a field of null"))
            }
            _ => Err(Error::eval(format!(
                "Cannot set field {} on {}",
                name,
                base.type_name()
            ))),
        }
    }

    fn get_index(&mut self, base: &Value, idx: &Value) -> Result<Value> {
        match base {
            Value::Array(a) => {
                let i = idx.as_i64()?;
                let a = a.borrow();
                let i = check_index(i, a.items.len())?;
                Ok(a.items[i].clone())
            }
            Value::Str(s) => {
                let i = idx.as_i64()?;
                let chars: Vec<char> = s.chars().collect();
                let i = check_index(i, chars.len())?;
                Ok(Value::Char(chars[i]))
            }
            Value::Java(o) => {
                let jvm = o.jvm().clone();
                if o.class_name().starts_with('[') {
                    let i = idx.as_i64()?;
                    if i < 0 || i as usize >= jvm.array_len(o).map_err(crate::java::error)? {
                        return Err(Error::eval(format!("Array index out of bounds: {}", i)));
                    }
                    let v = jvm.array_get(o, i as usize).map_err(crate::java::error)?;
                    Ok(crate::java::from_jval(v))
                } else {
                    let args = self.jval_args(std::slice::from_ref(idx))?;
                    match jvm.call(o, "get", &args) {
                        Ok(v) => Ok(crate::java::from_jval(v)),
                        Err(bsh_jvm::JError::Thrown(t)) => Err(Error::Thrown(Value::Java(t))),
                        Err(e) => Err(Error::eval(format!("{}", e))),
                    }
                }
            }
            Value::Builtin(_) | Value::Object(_) => builtins::index_get(self, base, idx),
            other => Err(Error::eval(format!("Cannot index {}", other.type_name()))),
        }
    }

    fn set_index(&mut self, base: &Value, idx: &Value, value: Value) -> Result<()> {
        match base {
            Value::Array(a) => {
                let i = idx.as_i64()?;
                let len = a.borrow().items.len();
                let i = check_index(i, len)?;
                a.borrow_mut().items[i] = value;
                Ok(())
            }
            Value::Java(o) => {
                let jvm = o.jvm().clone();
                if o.class_name().starts_with('[') {
                    let i = idx.as_i64()?;
                    if i < 0 || i as usize >= jvm.array_len(o).map_err(crate::java::error)? {
                        return Err(Error::eval(format!("Array index out of bounds: {}", i)));
                    }
                    let jv = crate::java::to_jval(self, &value)?;
                    jvm.array_set(o, i as usize, &jv)
                        .map_err(crate::java::error)
                } else {
                    let args = self.jval_args(&[idx.clone(), value])?;
                    match jvm.call(o, "put", &args) {
                        Ok(_) => Ok(()),
                        Err(_) => match jvm.call(o, "set", &args) {
                            Ok(_) => Ok(()),
                            Err(bsh_jvm::JError::Thrown(t)) => Err(Error::Thrown(Value::Java(t))),
                            Err(e) => Err(Error::eval(format!("{}", e))),
                        },
                    }
                }
            }
            Value::Builtin(_) | Value::Object(_) => builtins::index_set(self, base, idx, value),
            other => Err(Error::eval(format!("Cannot index {}", other.type_name()))),
        }
    }

    fn slice(&mut self, base: &Value, bounds: &[Option<i64>]) -> Result<Value> {
        let items = self.iterate(base)?;
        let len = items.len() as i64;
        let (start, end, step) = match bounds.len() {
            0 => (0, len, 1),
            1 => (bounds[0].unwrap_or(0), len, 1),
            2 => (bounds[0].unwrap_or(0), bounds[1].unwrap_or(len), 1),
            _ => (
                bounds[0].unwrap_or(0),
                bounds[1].unwrap_or(len),
                bounds[2].unwrap_or(1),
            ),
        };
        let step = if step == 0 { 1 } else { step };
        let mut out = Vec::new();
        let mut i = start;
        if step > 0 {
            while i < end.min(len) {
                if i >= 0 {
                    out.push(items[i as usize].clone());
                }
                i += step;
            }
        } else {
            while i > end.max(-1) {
                if i >= 0 && i < len {
                    out.push(items[i as usize].clone());
                }
                i += step;
            }
        }
        Ok(Value::Array(Rc::new(RefCell::new(ArrayValue {
            dims: 1,
            component: None,
            items: out,
        }))))
    }

    pub fn iterate(&mut self, value: &Value) -> Result<Vec<Value>> {
        match value {
            Value::Array(a) => Ok(a.borrow().items.clone()),
            Value::Str(s) => Ok(s.chars().map(Value::Char).collect()),
            Value::Java(o) => {
                let jvm = o.jvm().clone();
                if o.class_name().starts_with('[') {
                    let len = jvm.array_len(o).map_err(crate::java::error)?;
                    let mut out = Vec::with_capacity(len);
                    for i in 0..len {
                        let v = jvm.array_get(o, i).map_err(crate::java::error)?;
                        out.push(crate::java::from_jval(v));
                    }
                    return Ok(out);
                }
                let which = if jvm.is_instance(self.runtime_id, o, "java.util.Map") {
                    "keySet"
                } else {
                    "toArray"
                };
                let arr = jvm.call(o, which, &[]).map_err(crate::java::error)?;
                let arr = match arr {
                    bsh_jvm::JVal::Obj(a) => a,
                    _ => return Ok(Vec::new()),
                };
                let len = jvm.array_len(&arr).map_err(crate::java::error)?;
                let mut out = Vec::with_capacity(len);
                for i in 0..len {
                    let v = jvm.array_get(&arr, i).map_err(crate::java::error)?;
                    out.push(crate::java::from_jval(v));
                }
                Ok(out)
            }
            Value::Builtin(_) | Value::Object(_) => builtins::iterate(self, value),
            Value::Null => Err(Error::eval("Cannot iterate null")),
            other => Err(Error::eval(format!("Cannot iterate {}", other.type_name()))),
        }
    }

    fn eval_new_array(
        &mut self,
        ty: &TypeRef,
        dims: &[Option<Expr>],
        init: &Option<Vec<Expr>>,
        ns: &Ns,
    ) -> Result<Value> {
        if let Some(items) = init {
            let mut vals = Vec::with_capacity(items.len());
            for i in items {
                vals.push(self.eval(i, ns)?);
            }
            return Ok(Value::Array(Rc::new(RefCell::new(ArrayValue {
                dims: ty.dims.max(1),
                component: Some(ty.clone()),
                items: vals,
            }))));
        }
        let mut sizes = Vec::new();
        for d in dims {
            match d {
                Some(e) => sizes.push(self.eval(e, ns)?.as_i64()?),
                None => sizes.push(-1),
            }
        }
        self.build_array(ty, &sizes, 0)
    }

    fn build_array(&mut self, ty: &TypeRef, sizes: &[i64], depth: usize) -> Result<Value> {
        if depth >= sizes.len() {
            return Ok(Value::Null);
        }
        let n = sizes[depth];
        if n < 0 {
            return Ok(Value::Null);
        }
        let mut items = Vec::with_capacity(n as usize);
        for _ in 0..n {
            let item = if depth + 1 < sizes.len() && sizes[depth + 1] >= 0 {
                self.build_array(ty, sizes, depth + 1)?
            } else {
                // Array elements use the scalar component default.
                let scalar = TypeRef {
                    name: ty.name.clone(),
                    primitive: ty.primitive,
                    dims: 0,
                    span: ty.span,
                };
                default_value(Some(&scalar))
            };
            items.push(item);
        }
        Ok(Value::Array(Rc::new(RefCell::new(ArrayValue {
            dims: sizes.len() - depth,
            component: Some(ty.clone()),
            items,
        }))))
    }

    fn eval_new(
        &mut self,
        ty: &TypeRef,
        args: &[Expr],
        anon: Option<&ClassDecl>,
        ns: &Ns,
    ) -> Result<Value> {
        let name = ty.base_name();
        let mut values = Vec::with_capacity(args.len());
        for a in args {
            values.push(self.eval(a, ns)?);
        }
        if name == "Object" || name == "java.lang.Object" {
            if values.is_empty() {
                return Ok(Value::Object(Rc::new(RefCell::new(ObjectValue {
                    class: self.find_class("Object").unwrap(),
                    fields: HashMap::new(),
                    java: None,
                }))));
            }
        }
        let mut class = match self.resolve_class(&name) {
            Some(c) => c,
            None => return Err(Error::eval(format!("Unknown class: {}", name))),
        };
        if let Some(anon_decl) = anon {
            class = self.make_anonymous_class(&class, anon_decl, ns)?;
        }
        self.construct(class, values, ns)
    }

    fn make_anonymous_class(
        &mut self,
        base: &Rc<ClassValue>,
        decl: &ClassDecl,
        ns: &Ns,
    ) -> Result<Rc<ClassValue>> {
        let mut cd = decl.clone();
        if cd.name.starts_with("$anon") {
            cd.name = format!("{}$anon", base.name);
        }
        cd.extends = None;
        let defined = self.define_class(ns, &cd)?;
        let def = defined.def.clone();
        let c = Rc::new(ClassValue {
            name: cd.name.clone(),
            superclass: Some(base.clone()),
            interfaces: Vec::new(),
            def,
            statics: RefCell::new(HashMap::new()),
            is_interface: false,
            builtin: None,
        });
        Ok(c)
    }

    /// True when a class is backed by the JVM rather than a Rust model.
    fn java_backed(class: &Rc<ClassValue>) -> bool {
        // A class without a script definition is provided by the JVM.
        class.def.is_none()
    }

    /// Build a generated Java subclass for a scripted class that extends a
    /// Java class, and run its script side initialization.
    fn construct_java_subclass(
        &mut self,
        class: Rc<ClassValue>,
        superclass: Rc<ClassValue>,
        args: Vec<Value>,
        ns: &Ns,
    ) -> Result<Value> {
        let jvm = self.jvm().ok_or_else(|| Error::eval("No JVM available"))?;
        if !jvm.supports_script_classes() {
            return Err(Error::eval(format!(
                "Class {} cannot extend the Java class {} on Android: ART does not allow defining Java class bytecode at runtime (desktop bsh-rs does). Use an interface/anonymous class, or a function returning this with a run() method: new Thread(myFunction())",
                class.name, superclass.name
            )));
        }
        if jvm
            .ctor_params(self.runtime_id, &superclass.name, 0)
            .is_none()
        {
            return Err(Error::eval(format!(
                "Class {} has no no-argument constructor; a scripted subclass requires one",
                superclass.name
            )));
        }
        let object = Rc::new(RefCell::new(ObjectValue {
            class: class.clone(),
            fields: HashMap::new(),
            java: None,
        }));
        let this = Value::Object(object.clone());
        let handle = crate::proxy::register_instance(
            self,
            crate::proxy::ProxyTarget::Scripted(this.clone()),
        );
        let internal = format!("bshrs/gen/Script{}", handle);
        let binary = format!("bshrs.gen.Script{}", handle);
        let mut builder =
            bsh_jvm::classfile::ClassBuilder::new(&internal, &superclass.name.replace('.', "/"));
        builder.add_constructor();
        let rows = jvm
            .overridable_methods(&superclass.name)
            .map_err(crate::java::error)?;
        let mut seen: Vec<(String, usize)> = Vec::new();
        if let Some(def) = &class.def {
            for method in &def.methods {
                if method.is_ctor {
                    continue;
                }
                let arity = method.params.len();
                if seen.iter().any(|(n, a)| n == &method.name && *a == arity) {
                    continue;
                }
                for row in &rows {
                    let parts: Vec<&str> = row.split('|').collect();
                    if parts.len() < 2 || parts[0] != method.name {
                        continue;
                    }
                    let param_types: Vec<String> =
                        parts[2..].iter().map(|s| s.to_string()).collect();
                    if param_types.len() != arity {
                        continue;
                    }
                    let ret = parts[1].to_string();
                    let desc = bsh_jvm::classfile::method_descriptor(&param_types, &ret);
                    let param_descs: Vec<String> = param_types
                        .iter()
                        .map(|t| bsh_jvm::classfile::type_descriptor(t))
                        .collect();
                    let ret_desc = bsh_jvm::classfile::type_descriptor(&ret);
                    builder.add_dispatch_method(&method.name, &desc, &param_descs, &ret_desc);
                    builder.add_super_forwarder(
                        &method.name,
                        &desc,
                        &param_descs,
                        &ret_desc,
                        arity,
                    );
                    seen.push((method.name.clone(), arity));
                    break;
                }
            }
        }
        let bytes = builder.finish();
        let class_object = jvm
            .define_script_class(&binary, &bytes)
            .map_err(crate::java::error)?;
        let instance = jvm
            .new_script_instance(&class_object, handle)
            .map_err(crate::java::error)?;
        let java_object = match instance {
            bsh_jvm::JVal::Obj(o) => o,
            _ => return Err(Error::eval("Java subclass instantiation failed")),
        };
        object.borrow_mut().java = Some(java_object);

        // Script fields and instance initializer statements.
        let frame = Namespace::child(ns);
        frame.borrow_mut().this_obj = Some(this.clone());
        if let Some(def) = &class.def {
            for field in &def.fields {
                let value = match &field.init {
                    Some(e) => {
                        let value = self.eval(e, &frame)?;
                        match &field.ty {
                            Some(t) => self.cast_value(value, t)?,
                            None => value,
                        }
                    }
                    None => default_value(field.ty.as_ref()),
                };
                object.borrow_mut().fields.insert(field.name.clone(), value);
            }
            for stmt in &def.instance_init {
                let _ = self.exec_stmt(stmt, &frame);
            }
            let loose: Vec<(String, Value)> = frame
                .borrow()
                .vars
                .iter()
                .map(|(k, v)| (k.clone(), v.value.clone()))
                .collect();
            for (k, v) in loose {
                object.borrow_mut().fields.insert(k, v);
            }
            frame.borrow_mut().vars.clear();
        }
        match class.ctor(args.len()) {
            Some(ctor) => {
                self.invoke_ctor(&ctor, this, args)?;
            }
            None if !args.is_empty() => {
                return Err(Error::eval(format!(
                    "No constructor with {} arguments in {}",
                    args.len(),
                    class.name
                )));
            }
            None => {}
        }
        Ok(Value::Object(object))
    }

    pub fn construct(&mut self, class: Rc<ClassValue>, args: Vec<Value>, ns: &Ns) -> Result<Value> {
        if class.builtin == Some("java") {
            let jvm = self.jvm().ok_or_else(|| Error::eval("No JVM available"))?;
            let params = jvm.ctor_params(self.runtime_id, &class.name, args.len());
            let vals = self.jval_args_typed(params.as_deref(), &args)?;
            let value = jvm
                .new_instance(self.runtime_id, &class.name, &vals)
                .map_err(crate::java::error)?;
            return Ok(crate::java::from_jval(value));
        }
        // A known Java class without a Rust implementation: construct in the JVM.
        if class.builtin == Some("") && class.def.is_none() {
            let jvm = self.jvm().ok_or_else(|| Error::eval("No JVM available"))?;
            let params = jvm.ctor_params(self.runtime_id, &class.name, args.len());
            let vals = self.jval_args_typed(params.as_deref(), &args)?;
            let value = jvm
                .new_instance(self.runtime_id, &class.name, &vals)
                .map_err(crate::java::error)?;
            return Ok(crate::java::from_jval(value));
        }
        if let Some(kind) = class.builtin {
            return builtins::construct(self, &class, kind, args);
        }
        if let Some(superclass) = class.superclass.clone() {
            if Self::java_backed(&superclass) {
                // Interfaces cannot be subclassed; anonymous classes implementing
                // them stay scripted objects and reach Java through a proxy.
                let is_interface = self
                    .jvm()
                    .map(|j| j.is_interface(self.runtime_id, &superclass.name))
                    .unwrap_or(false);
                if !is_interface {
                    return self.construct_java_subclass(class, superclass, args, ns);
                }
            }
        }
        // Initialize the whole superclass chain first, on one shared object.
        let chain = self.class_chain(&class);
        let object = Rc::new(RefCell::new(ObjectValue {
            class: class.clone(),
            fields: HashMap::new(),
            java: None,
        }));
        let this = Value::Object(object.clone());
        // Fields + instance initializers, superclass first.
        for c in chain.iter().rev() {
            let frame = Namespace::child(ns);
            frame.borrow_mut().this_obj = Some(this.clone());
            if let Some(def) = &c.def {
                for f in &def.fields {
                    let v = match &f.init {
                        Some(e) => {
                            let val = self.eval(e, &frame)?;
                            match &f.ty {
                                Some(t) => self.cast_value(val, t)?,
                                None => val,
                            }
                        }
                        None => default_value(f.ty.as_ref()),
                    };
                    object.borrow_mut().fields.insert(f.name.clone(), v);
                }
                for s in &def.instance_init {
                    let _ = self.exec_stmt(s, &frame);
                }
                // Loose variables created by initializer statements are fields.
                let loose: Vec<(String, Value)> = frame
                    .borrow()
                    .vars
                    .iter()
                    .map(|(k, v)| (k.clone(), v.value.clone()))
                    .collect();
                for (k, v) in loose {
                    object.borrow_mut().fields.insert(k, v);
                }
                frame.borrow_mut().vars.clear();
            }
        }
        // Constructor: most specific class first, falling back to no-arg.
        let ctor = class.ctor(args.len());
        if let Some(m) = ctor {
            self.invoke_ctor(&m, this.clone(), args)?;
        }
        let _ = ns;
        Ok(Value::Object(object))
    }

    fn class_chain(&self, class: &Rc<ClassValue>) -> Vec<Rc<ClassValue>> {
        let mut out = Vec::new();
        let mut cur = Some(class.clone());
        while let Some(c) = cur {
            out.push(c.clone());
            cur = c.superclass.clone();
        }
        out
    }

    fn invoke_ctor(&mut self, m: &Rc<MethodDef>, this: Value, args: Vec<Value>) -> Result<Value> {
        self.invoke_scripted(m, Some(this), args)
    }

    // ------------------------------------------------------------- operators

    fn eval_unary(&mut self, op: UnOp, expr: &Expr, ns: &Ns) -> Result<Value> {
        match op {
            UnOp::Neg => {
                let v = self.eval(expr, ns)?;
                primitive::negate(&v)
            }
            UnOp::Plus => {
                let v = self.eval(expr, ns)?;
                if v.is_numeric() {
                    Ok(v)
                } else {
                    Err(Error::eval(format!("Unary plus on {}", v.type_name())))
                }
            }
            UnOp::Not => {
                let v = self.eval(expr, ns)?;
                primitive::not(&v)
            }
            UnOp::BitNot => {
                let v = self.eval(expr, ns)?;
                primitive::bit_not(&v)
            }
            UnOp::PreInc | UnOp::PreDec | UnOp::PostInc | UnOp::PostDec => {
                if let ExprKind::Name(parts) = &expr.kind {
                    if parts.len() == 1 {
                        if let Some(owner) = Namespace::owner_of(ns, &parts[0]) {
                            let current =
                                { owner.borrow().vars.get(&parts[0]).map(|v| v.value.clone()) };
                            if let Some(Value::Int(old)) = current {
                                let new = if matches!(op, UnOp::PreInc | UnOp::PostInc) {
                                    old.wrapping_add(1)
                                } else {
                                    old.wrapping_sub(1)
                                };
                                let stored =
                                    self.store_typed(&owner, &parts[0], Value::Int(new))?;
                                return if matches!(op, UnOp::PreInc | UnOp::PreDec) {
                                    Ok(stored)
                                } else {
                                    Ok(Value::Int(old))
                                };
                            }
                        }
                    }
                }
                let old = self.eval(expr, ns)?;
                let one = Value::Int(1);
                let new = match op {
                    UnOp::PreInc | UnOp::PostInc => primitive::add(&old, &one)?,
                    _ => primitive::sub(&old, &one)?,
                };
                self.assign_to(expr, new.clone(), ns)?;
                if matches!(op, UnOp::PreInc | UnOp::PreDec) {
                    Ok(new)
                } else {
                    Ok(old)
                }
            }
        }
    }

    fn eval_binary(&mut self, op: BinOp, lhs: &Expr, rhs: &Expr, ns: &Ns) -> Result<Value> {
        match op {
            BinOp::And => {
                let l = self.eval(lhs, ns)?.as_bool()?;
                if !l {
                    return Ok(Value::Bool(false));
                }
                Ok(Value::Bool(self.eval(rhs, ns)?.as_bool()?))
            }
            BinOp::Or => {
                let l = self.eval(lhs, ns)?.as_bool()?;
                if l {
                    return Ok(Value::Bool(true));
                }
                Ok(Value::Bool(self.eval(rhs, ns)?.as_bool()?))
            }
            BinOp::Coalesce => {
                let l = self.eval(lhs, ns)?;
                if l.is_null_like() {
                    self.eval(rhs, ns)
                } else {
                    Ok(l)
                }
            }
            BinOp::Add => {
                let l = self.eval(lhs, ns)?;
                let r = self.eval(rhs, ns)?;
                if let (Value::Int(a), Value::Int(b)) = (&l, &r) {
                    return Ok(Value::Int(a.wrapping_add(*b)));
                }
                if matches!(l, Value::Str(_)) || matches!(r, Value::Str(_)) {
                    return Ok(Value::string(format!(
                        "{}{}",
                        l.to_display_string(),
                        r.to_display_string()
                    )));
                }
                primitive::add(&l, &r)
            }
            _ => {
                let l = self.eval(lhs, ns)?;
                let r = self.eval(rhs, ns)?;
                if let (Value::Int(a), Value::Int(b)) = (&l, &r) {
                    let fast = match op {
                        BinOp::Sub => Some(Value::Int(a.wrapping_sub(*b))),
                        BinOp::Mul => Some(Value::Int(a.wrapping_mul(*b))),
                        BinOp::Div if *b != 0 => Some(Value::Int(a.wrapping_div(*b))),
                        BinOp::Mod if *b != 0 => Some(Value::Int(a.wrapping_rem(*b))),
                        BinOp::Lt => Some(Value::Bool(a < b)),
                        BinOp::Gt => Some(Value::Bool(a > b)),
                        BinOp::Le => Some(Value::Bool(a <= b)),
                        BinOp::Ge => Some(Value::Bool(a >= b)),
                        BinOp::Eq => Some(Value::Bool(a == b)),
                        BinOp::Ne => Some(Value::Bool(a != b)),
                        BinOp::BitAnd => Some(Value::Int(a & b)),
                        BinOp::BitOr => Some(Value::Int(a | b)),
                        BinOp::BitXor => Some(Value::Int(a ^ b)),
                        BinOp::Shl => Some(Value::Int(a.wrapping_shl((*b & 31) as u32))),
                        BinOp::Shr => Some(Value::Int(a.wrapping_shr((*b & 31) as u32))),
                        BinOp::Ushr => Some(Value::Int(
                            ((*a as u32).wrapping_shr((*b & 31) as u32)) as i32,
                        )),
                        BinOp::Cmp => Some(Value::Int(if a < b {
                            -1
                        } else if a > b {
                            1
                        } else {
                            0
                        })),
                        _ => None,
                    };
                    if let Some(value) = fast {
                        return Ok(value);
                    }
                }
                match op {
                    BinOp::Sub => primitive::sub(&l, &r),
                    BinOp::Mul => primitive::mul(&l, &r),
                    BinOp::Div => primitive::div(&l, &r),
                    BinOp::Mod => primitive::rem(&l, &r),
                    BinOp::Pow => primitive::pow(&l, &r),
                    BinOp::Eq => Ok(Value::Bool(l.equality(&r))),
                    BinOp::Ne => Ok(Value::Bool(!l.equality(&r))),
                    BinOp::Lt => Ok(Value::Bool(primitive::compare(&l, &r)? < 0)),
                    BinOp::Gt => Ok(Value::Bool(primitive::compare(&l, &r)? > 0)),
                    BinOp::Le => Ok(Value::Bool(primitive::compare(&l, &r)? <= 0)),
                    BinOp::Ge => Ok(Value::Bool(primitive::compare(&l, &r)? >= 0)),
                    BinOp::Cmp => Ok(Value::Int(primitive::compare(&l, &r)?)),
                    BinOp::BitAnd => primitive::logic(&l, &r, '&'),
                    BinOp::BitOr => primitive::logic(&l, &r, '|'),
                    BinOp::BitXor => primitive::logic(&l, &r, '^'),
                    BinOp::Shl => primitive::shift(&l, &r, "<<"),
                    BinOp::Shr => primitive::shift(&l, &r, ">>"),
                    BinOp::Ushr => primitive::shift(&l, &r, ">>>"),
                    BinOp::And | BinOp::Or | BinOp::Coalesce | BinOp::Add => unreachable!(),
                }
            }
        }
    }

    fn eval_assign(&mut self, op: AssignOp, lhs: &Expr, rhs: &Expr, ns: &Ns) -> Result<Value> {
        let rv = self.eval(rhs, ns)?;
        if op == AssignOp::Assign {
            return self.assign_to(lhs, rv, ns);
        }
        if let ExprKind::Name(parts) = &lhs.kind {
            if parts.len() == 1 {
                if let Some(owner) = Namespace::owner_of(ns, &parts[0]) {
                    let current = { owner.borrow().vars.get(&parts[0]).map(|v| v.value.clone()) };
                    if let Some(Value::Int(a)) = current {
                        if let Value::Int(b) = &rv {
                            let fast = match op {
                                AssignOp::Add => Some(Value::Int(a.wrapping_add(*b))),
                                AssignOp::Sub => Some(Value::Int(a.wrapping_sub(*b))),
                                AssignOp::Mul => Some(Value::Int(a.wrapping_mul(*b))),
                                AssignOp::Div if *b != 0 => Some(Value::Int(a.wrapping_div(*b))),
                                AssignOp::Mod if *b != 0 => Some(Value::Int(a.wrapping_rem(*b))),
                                AssignOp::BitAnd => Some(Value::Int(a & b)),
                                AssignOp::BitOr => Some(Value::Int(a | b)),
                                AssignOp::BitXor => Some(Value::Int(a ^ b)),
                                AssignOp::Shl => Some(Value::Int(a.wrapping_shl((*b & 31) as u32))),
                                AssignOp::Shr => Some(Value::Int(a.wrapping_shr((*b & 31) as u32))),
                                AssignOp::Ushr => Some(Value::Int(
                                    ((a as u32).wrapping_shr((*b & 31) as u32)) as i32,
                                )),
                                _ => None,
                            };
                            if let Some(value) = fast {
                                return self.store_typed(&owner, &parts[0], value);
                            }
                        }
                    }
                }
            }
        }
        let cur = self.eval(lhs, ns)?;
        let computed = match op {
            AssignOp::Add => {
                if matches!(cur, Value::Str(_)) || matches!(rv, Value::Str(_)) {
                    Value::string(format!(
                        "{}{}",
                        cur.to_display_string(),
                        rv.to_display_string()
                    ))
                } else {
                    primitive::add(&cur, &rv)?
                }
            }
            AssignOp::Sub => primitive::sub(&cur, &rv)?,
            AssignOp::Mul => primitive::mul(&cur, &rv)?,
            AssignOp::Div => primitive::div(&cur, &rv)?,
            AssignOp::Mod => primitive::rem(&cur, &rv)?,
            AssignOp::Pow => primitive::pow(&cur, &rv)?,
            AssignOp::BitAnd => primitive::logic(&cur, &rv, '&')?,
            AssignOp::BitOr => primitive::logic(&cur, &rv, '|')?,
            AssignOp::BitXor => primitive::logic(&cur, &rv, '^')?,
            AssignOp::Shl => primitive::shift(&cur, &rv, "<<")?,
            AssignOp::Shr => primitive::shift(&cur, &rv, ">>")?,
            AssignOp::Ushr => primitive::shift(&cur, &rv, ">>>")?,
            AssignOp::Coalesce => {
                if cur.is_null_like() {
                    rv
                } else {
                    cur
                }
            }
            AssignOp::Assign => unreachable!(),
        };
        self.assign_to(lhs, computed, ns)
    }

    /// Write a value into an existing variable, applying its declared type.
    fn store_typed(&mut self, owner: &Ns, name: &str, value: Value) -> Result<Value> {
        // Fast path for primitive typed variables: no TypeRef clone.
        let primitive_kind = {
            let b = owner.borrow();
            b.vars.get(name).and_then(|v| v.ty.as_ref()).and_then(|t| {
                if t.dims == 0 {
                    t.primitive.map(crate::primitive::PrimKind::from_ast)
                } else {
                    None
                }
            })
        };
        let value = match primitive_kind {
            Some(kind) => crate::primitive::cast_primitive(&value, kind)?,
            None => {
                let typed = {
                    let b = owner.borrow();
                    b.vars.get(name).and_then(|v| v.ty.clone())
                };
                match typed {
                    Some(t) => self.cast_value(value, &t)?,
                    None => value,
                }
            }
        };
        if let Some(var) = owner.borrow_mut().vars.get_mut(name) {
            var.value = value.clone();
        }
        Ok(value)
    }

    pub fn assign_to(&mut self, target: &Expr, value: Value, ns: &Ns) -> Result<Value> {
        match &target.kind {
            ExprKind::Name(parts) => {
                if parts.len() == 1 {
                    let name = &parts[0];
                    if let Some(owner) = Namespace::owner_of(ns, name) {
                        return self.store_typed(&owner, name, value);
                    }
                    // A loose name that matches a field of "this" assigns the field.
                    if let Some(this) = Namespace::get_this(ns) {
                        match &this {
                            Value::Object(o) => {
                                let is_field = o.borrow().class.find_field(name).is_some();
                                if is_field {
                                    o.borrow_mut().fields.insert(name.clone(), value.clone());
                                    return Ok(value);
                                }
                                if let Some(java) = o.borrow().java.clone() {
                                    let jvm = java.jvm().clone();
                                    let jv = crate::java::to_jval(self, &value)?;
                                    match jvm.set_field(&java, name, &jv) {
                                        Ok(()) => return Ok(value),
                                        Err(bsh_jvm::JError::Thrown(t)) => {
                                            return Err(Error::Thrown(Value::Java(t)))
                                        }
                                        Err(_) => {}
                                    }
                                }
                            }
                            Value::Class(c) => {
                                let c = c.clone();
                                if c.static_get(name).is_some() {
                                    c.statics.borrow_mut().insert(name.clone(), value.clone());
                                    return Ok(value);
                                }
                            }
                            _ => {}
                        }
                    }
                    ns.borrow_mut().vars.insert(
                        name.clone(),
                        crate::namespace::Variable {
                            value: value.clone(),
                            ty: None,
                            is_final: false,
                            is_static: false,
                        },
                    );
                    return Ok(value);
                }
                let base = self.eval_name(&parts[..parts.len() - 1], ns)?;
                let name = parts.last().unwrap().clone();
                self.set_member(&base, &name, value.clone())?;
                Ok(value)
            }
            ExprKind::Field(base, name) => {
                let b = self.eval(base, ns)?;
                self.set_member(&b, name, value.clone())?;
                Ok(value)
            }
            ExprKind::Index(base, idx) => {
                let b = self.eval(base, ns)?;
                let i = self.eval(idx, ns)?;
                self.set_index(&b, &i, value.clone())?;
                Ok(value)
            }
            _ => Err(Error::eval("Can't assign to prefix.")),
        }
    }

    pub fn cast_value(&mut self, value: Value, ty: &TypeRef) -> Result<Value> {
        if ty.dims > 0 {
            if matches!(value, Value::Null) {
                return Ok(Value::Null);
            }
            if matches!(value, Value::Array(_)) {
                return Ok(value);
            }
            // Reflection and other Java APIs return real Java arrays.
            if let Value::Java(object) = &value {
                if object.class_name.starts_with('[') {
                    return Ok(value);
                }
            }
            let type_name = value.type_name();
            return Err(Error::eval(format!(
                "Cannot cast {} to {}",
                type_name,
                ty.display_name()
            )));
        }
        if let Some(p) = ty.primitive {
            let kind = match p {
                bsh_syntax::ast::PrimKind::Boolean => PrimKind::Boolean,
                bsh_syntax::ast::PrimKind::Byte => PrimKind::Byte,
                bsh_syntax::ast::PrimKind::Char => PrimKind::Char,
                bsh_syntax::ast::PrimKind::Short => PrimKind::Short,
                bsh_syntax::ast::PrimKind::Int => PrimKind::Int,
                bsh_syntax::ast::PrimKind::Long => PrimKind::Long,
                bsh_syntax::ast::PrimKind::Float => PrimKind::Float,
                bsh_syntax::ast::PrimKind::Double => PrimKind::Double,
                bsh_syntax::ast::PrimKind::Void => return Ok(Value::Void),
            };
            if matches!(value, Value::Void) {
                return Ok(default_value(Some(ty)));
            }
            if matches!(value, Value::Null) {
                return Err(Error::eval(format!(
                    "Cannot assign null to {}",
                    ty.display_name()
                )));
            }
            return primitive::cast_primitive(&value, kind);
        }
        // Reference type: arrays, strings and scripted objects are checked.
        let written = ty.base_name();
        if matches!(value, Value::Null) {
            return Ok(Value::Null);
        }
        // Imported simple names resolve to the fully qualified Java name.
        let name = match self.resolve_class(&written) {
            Some(class) => class.name.clone(),
            None => written,
        };
        // Lambdas and scripted objects implement Java interfaces via a proxy.
        if let Some(jvm) = self.jvm() {
            let has_java_instance = match &value {
                Value::Java(_) => true,
                Value::Object(o) => o.borrow().java.is_some(),
                _ => false,
            };
            let interface = if jvm.is_interface(self.runtime_id, &name) {
                Some(name.clone())
            } else {
                self.resolve_class(&name)
                    .filter(|c| jvm.is_interface(self.runtime_id, &c.name))
                    .map(|c| c.name.clone())
            };
            if let Some(interface) = interface.filter(|_| !has_java_instance) {
                match &value {
                    Value::Lambda(l) => {
                        return self.make_proxy(
                            &[interface.clone()],
                            crate::proxy::ProxyTarget::Lambda(l.clone()),
                        )
                    }
                    Value::Object(_) => {
                        return self.make_proxy(
                            &[interface.clone()],
                            crate::proxy::ProxyTarget::Scripted(value.clone()),
                        )
                    }
                    Value::MethodRef(m) => {
                        return self.make_proxy(
                            &[interface.clone()],
                            crate::proxy::ProxyTarget::MethodRef(m.clone()),
                        )
                    }
                    _ => {}
                }
            }
        }
        if self.value_instance_of(&value, &name) {
            return Ok(value);
        }
        if name == "String" || name == "java.lang.String" {
            return Ok(Value::string(value.to_display_string()));
        }
        // Wrapper / number conversions.
        let numeric_target = match name.as_str() {
            "Integer" | "java.lang.Integer" | "int" => Some(PrimKind::Int),
            "Long" | "java.lang.Long" | "long" => Some(PrimKind::Long),
            "Double" | "java.lang.Double" | "double" => Some(PrimKind::Double),
            "Float" | "java.lang.Float" | "float" => Some(PrimKind::Float),
            "Short" | "java.lang.Short" | "short" => Some(PrimKind::Short),
            "Byte" | "java.lang.Byte" | "byte" => Some(PrimKind::Byte),
            "Character" | "java.lang.Character" | "char" => Some(PrimKind::Char),
            "Boolean" | "java.lang.Boolean" | "boolean" => Some(PrimKind::Boolean),
            _ => None,
        };
        if let Some(k) = numeric_target {
            if value.is_numeric() || matches!(value, Value::Bool(_)) {
                return primitive::cast_primitive(&value, k);
            }
        }
        if value.is_numeric() && (name == "Number" || name == "java.lang.Number") {
            return Ok(value);
        }
        Err(Error::eval(format!(
            "Cannot cast {} to {}",
            value.type_name(),
            name
        )))
    }

    // ----------------------------------------------------------------- calls

    fn eval_call(&mut self, target: &Expr, arg_exprs: &[Expr], ns: &Ns) -> Result<Value> {
        // super.method(...) dispatches from the superclass of "this".
        if let ExprKind::Field(base, name) = &target.kind {
            if matches!(base.kind, ExprKind::Super) {
                let this = self.this_value(ns);
                if let Value::Object(o) = &this {
                    let runtime_class = o.borrow().class.clone();
                    let java = o.borrow().java.clone();
                    // super is relative to the method's declaring class, not the
                    // dynamic class of the receiver.
                    let declaring = self
                        .method_class_stack
                        .last()
                        .cloned()
                        .flatten()
                        .unwrap_or_else(|| runtime_class.clone());
                    let Some(start) = declaring.superclass.clone() else {
                        return Err(Error::eval(format!(
                            "Class {} has no superclass for super.{}()",
                            declaring.name, name
                        )));
                    };
                    let mut args = Vec::with_capacity(arg_exprs.len());
                    for a in arg_exprs {
                        args.push(self.eval(a, ns)?);
                    }
                    if let Some(java) = java {
                        let jvm = java.jvm().clone();
                        let mut vals = self.jval_args(&args)?;
                        let special = format!("__super_{}_{}", name, args.len());
                        if let Ok(v) = jvm.call(&java, &special, &vals) {
                            return Ok(crate::java::from_jval(v));
                        }
                        vals = self.jval_args(&args)?;
                        return match jvm.call(&java, name, &vals) {
                            Ok(v) => Ok(crate::java::from_jval(v)),
                            Err(bsh_jvm::JError::Thrown(t)) => Err(Error::Thrown(Value::Java(t))),
                            Err(e) => Err(Error::eval(format!("{}", e))),
                        };
                    }
                    return self.call_on_class(&start, name, args, ns, Some(this.clone()));
                }
            }
        }
        if let ExprKind::Name(parts) = &target.kind {
            return self.call_name(parts, arg_exprs, ns);
        }
        if let ExprKind::Field(base, name) = &target.kind {
            let base_val = self.eval(base, ns)?;
            let mut args = Vec::with_capacity(arg_exprs.len());
            for a in arg_exprs {
                args.push(self.eval(a, ns)?);
            }
            return self.call_method_on(&base_val, name, args, ns);
        }
        if let ExprKind::SafeField(base, name) = &target.kind {
            let base_val = self.eval(base, ns)?;
            if base_val.is_null_like() {
                return Ok(Value::Null);
            }
            let mut args = Vec::with_capacity(arg_exprs.len());
            for a in arg_exprs {
                args.push(self.eval(a, ns)?);
            }
            return self.call_method_on(&base_val, name, args, ns);
        }
        let callee = self.eval(target, ns)?;
        let mut args = Vec::with_capacity(arg_exprs.len());
        for a in arg_exprs {
            args.push(self.eval(a, ns)?);
        }
        self.call_value(
            callee,
            &arg_exprs.iter().map(|_| "").collect::<Vec<_>>(),
            args,
            ns,
            target,
        )
    }

    fn call_name(&mut self, parts: &[String], arg_exprs: &[Expr], ns: &Ns) -> Result<Value> {
        let mut args = Vec::with_capacity(arg_exprs.len());
        for a in arg_exprs {
            args.push(self.eval(a, ns)?);
        }
        if parts.len() == 1 {
            let name = &parts[0];
            // super(...) and this(...) constructor delegation is handled in constructors.
            if let Some(m) = Namespace::find_method(ns, name, args.len()) {
                // A plain function call keeps the callee's own "this"; only
                // instance methods receive an object.
                let this = if m.declaring_class.is_some() {
                    Namespace::get_this(ns)
                } else {
                    None
                };
                return self.invoke_scripted(&m, this, args);
            }
            if let Some(value) = Namespace::lookup_value(ns, name) {
                match value {
                    Value::Method(m) => return self.invoke_scripted(&m, None, args),
                    Value::Lambda(l) => {
                        let m = l.method.clone();
                        return self.invoke_scripted(&m, None, args);
                    }
                    Value::MethodRef(m) => {
                        return self.invoke_method_ref(&m, args, ns);
                    }
                    _ => {}
                }
            }
            if let Some(this) = Namespace::get_this(ns) {
                if let Value::Object(o) = &this {
                    let class = o.borrow().class.clone();
                    if let Some(m) = class.find_method(name, args.len()) {
                        return self.invoke_scripted(&m, Some(this.clone()), args);
                    }
                }
            }
            // Host-registered Java methods (plugin SDK) shadow builtins.
            if let Some(result) = self.invoke_host_method(name, args.clone(), ns) {
                return result;
            }
            if builtins::is_builtin_function(name) {
                return builtins::call_function(self, name, args, ns);
            }
            if let Some(result) = self.static_import_call(name, args, ns) {
                return result;
            }
            return Err(Error::eval(format!(
                "Attempt to resolve method: {}() on undefined variable or class name: {}",
                name, name
            )));
        }
        // A variable receiver shadows a class of the same name.
        if Namespace::lookup_value(ns, &parts[0]).is_some() {
            let base = self.eval_name(&parts[..parts.len() - 1], ns)?;
            return self.call_method_on(&base, &parts[parts.len() - 1], args, ns);
        }
        // Class prefix?
        for k in (1..parts.len()).rev() {
            let full = parts[..k].join(".");
            if let Some(c) = self.resolve_class(&full) {
                if k == parts.len() - 1 {
                    return self.call_on_class(&c, &parts[k], args, ns, None);
                }
                let base = self.eval_name(&parts[..k], ns)?;
                let mut val = base;
                for p in &parts[k..parts.len() - 1] {
                    val = self.get_member(&val, p, ns)?;
                }
                return self.call_method_on(&val, &parts[parts.len() - 1], args, ns);
            }
        }
        let base = self.eval_name(&parts[..parts.len() - 1], ns)?;
        self.call_method_on(&base, &parts[parts.len() - 1], args, ns)
    }

    fn call_value(
        &mut self,
        callee: Value,
        _names: &[&str],
        args: Vec<Value>,
        ns: &Ns,
        target: &Expr,
    ) -> Result<Value> {
        match callee {
            Value::Method(m) => self.invoke_scripted(&m, None, args),
            Value::Lambda(l) => {
                let m = l.method.clone();
                self.invoke_scripted(&m, None, args)
            }
            Value::MethodRef(m) => self.invoke_method_ref(&m, args, ns),
            other => {
                // obj.method() through an arbitrary expression: resolve the
                // method name from the call target when possible.
                if let ExprKind::Field(_, name) = &target.kind {
                    self.call_method_on(&other, name, args, ns)
                } else {
                    Err(Error::eval(format!("Cannot invoke {}", other.type_name())))
                }
            }
        }
    }

    pub fn make_proxy(
        &mut self,
        interfaces: &[String],
        target: crate::proxy::ProxyTarget,
    ) -> Result<Value> {
        crate::proxy::make_proxy(self, interfaces, target)
    }

    /// Block until a thread finishes, draining cross-thread script callbacks.
    fn pump_join(
        &mut self,
        thread: &std::sync::Arc<bsh_jvm::JObj>,
        args: &[Value],
    ) -> Result<Value> {
        let timeout = args.first().map(|v| v.as_i64().unwrap_or(0)).unwrap_or(0);
        let deadline = if timeout > 0 {
            Some(Instant::now() + Duration::from_millis(timeout as u64))
        } else {
            None
        };
        let jvm = thread.jvm().clone();
        loop {
            crate::proxy::pump(self);
            match jvm
                .call(thread, "isAlive", &[])
                .map_err(crate::java::error)?
            {
                bsh_jvm::JVal::Bool(true) => {}
                _ => return Ok(Value::Void),
            }
            if let Some(deadline) = deadline {
                if Instant::now() >= deadline {
                    return Ok(Value::Void);
                }
            }
            std::thread::sleep(Duration::from_millis(1));
        }
    }

    /// Sleep, draining cross-thread script callbacks while waiting.
    fn pump_sleep(&mut self, millis: i64) -> Result<Value> {
        let deadline = Instant::now() + Duration::from_millis(millis.max(0) as u64);
        loop {
            crate::proxy::pump(self);
            if Instant::now() >= deadline {
                return Ok(Value::Void);
            }
            std::thread::sleep(Duration::from_millis(1));
        }
    }

    /// Invoke a method reference (instance, static or constructor).
    pub fn invoke_method_ref(
        &mut self,
        method_ref: &Rc<crate::value::MethodRefValue>,
        args: Vec<Value>,
        ns: &Ns,
    ) -> Result<Value> {
        let base = match &method_ref.target {
            crate::value::MethodRefTarget::Value(v) => v.clone(),
            crate::value::MethodRefTarget::Deferred(expr, captured) => {
                let captured = captured.clone();
                self.eval(expr, &captured)?
            }
        };
        if method_ref.name == "new" {
            return match base {
                Value::Class(class) => self.construct(class, args, ns),
                other => Err(Error::eval(format!(
                    "Cannot construct from {}",
                    other.type_name()
                ))),
            };
        }
        if let Value::Class(class) = &base {
            let class = class.clone();
            match self.call_on_class(&class, &method_ref.name, args.clone(), ns, None) {
                Ok(value) => return Ok(value),
                Err(first_error) => {
                    // Unbound instance reference: the receiver is the first argument.
                    if let Some(receiver) = args.first().cloned() {
                        return self.call_method_on(
                            &receiver,
                            &method_ref.name,
                            args[1..].to_vec(),
                            ns,
                        );
                    }
                    return Err(first_error);
                }
            }
        }
        self.call_method_on(&base, &method_ref.name, args, ns)
    }

    /// Convert arguments using the parameter types selected by the Java side,
    /// creating proxies for lambdas and scripted objects when an interface is
    /// expected.
    fn jval_args_typed(
        &mut self,
        params: Option<&[String]>,
        args: &[Value],
    ) -> Result<Vec<bsh_jvm::JVal>> {
        let mut out = Vec::with_capacity(args.len());
        for (i, arg) in args.iter().enumerate() {
            if let Some(types) = params {
                if let Some(expected) = types.get(i) {
                    let is_interface = self
                        .jvm()
                        .map(|j| j.is_interface(self.runtime_id, expected))
                        .unwrap_or(false);
                    if is_interface {
                        match arg {
                            Value::Lambda(l) => {
                                let proxy = self.make_proxy(
                                    &[expected.clone()],
                                    crate::proxy::ProxyTarget::Lambda(l.clone()),
                                )?;
                                out.push(crate::java::to_jval(self, &proxy)?);
                                continue;
                            }
                            Value::Object(o) if o.borrow().java.is_none() => {
                                let proxy = self.make_proxy(
                                    &[expected.clone()],
                                    crate::proxy::ProxyTarget::Scripted(arg.clone()),
                                )?;
                                out.push(crate::java::to_jval(self, &proxy)?);
                                continue;
                            }
                            Value::MethodRef(m) => {
                                let proxy = self.make_proxy(
                                    &[expected.clone()],
                                    crate::proxy::ProxyTarget::MethodRef(m.clone()),
                                )?;
                                out.push(crate::java::to_jval(self, &proxy)?);
                                continue;
                            }
                            _ => {}
                        }
                    }
                }
            }
            out.push(crate::java::to_jval(self, arg)?);
        }
        Ok(out)
    }

    /// Try a Java static call for a class that is partially modelled in Rust.
    fn try_jvm_static(
        &mut self,
        class: &Rc<ClassValue>,
        name: &str,
        args: &[Value],
    ) -> Option<Value> {
        let jvm = self.jvm()?;
        let params = jvm.method_params(self.runtime_id, None, &class.name, name, args.len(), true);
        let vals = self.jval_args_typed(params.as_deref(), args).ok()?;
        match jvm.call_static(self.runtime_id, &class.name, name, &vals) {
            Ok(v) => Some(crate::java::from_jval(v)),
            Err(_) => None,
        }
    }

    pub fn call_method_on(
        &mut self,
        base: &Value,
        name: &str,
        args: Vec<Value>,
        ns: &Ns,
    ) -> Result<Value> {
        // java.lang.Object.getClass() for host-backed values as well.
        if name == "getClass" && args.is_empty() {
            if let Some(jvm) = self.jvm() {
                let candidates: Vec<String> = match base {
                    Value::Str(_) => vec!["java.lang.String".to_string()],
                    Value::Builtin(builtin) => {
                        let simple = builtin.borrow().class_name().to_string();
                        vec![
                            format!("java.util.{}", simple),
                            format!("java.lang.{}", simple),
                            simple,
                        ]
                    }
                    Value::Java(object) => vec![object.class_name.clone()],
                    Value::Object(object) => {
                        let object = object.borrow();
                        match &object.java {
                            Some(java) => vec![java.class_name.clone()],
                            None => vec![object.class.name.clone()],
                        }
                    }
                    _ => Vec::new(),
                };
                for candidate in candidates {
                    if !candidate.is_empty() && jvm.class_exists(self.runtime_id, &candidate) {
                        if let Ok(class) = jvm.class_ref(self.runtime_id, &candidate) {
                            return Ok(Value::Java(class));
                        }
                    }
                }
            }
        }
        // Boxed primitives expose java.lang.Number style accessors.
        if args.is_empty() {
            if let Some(value) = primitive_wrapper_method(base, name) {
                return Ok(value);
            }
        }
        match base {
            Value::Object(o) => {
                let class = o.borrow().class.clone();
                if class.find_method(name, args.len()).is_none() {
                    let java = o.borrow().java.clone();
                    if let Some(java) = java {
                        let jvm = java.jvm().clone();
                        if name == "join"
                            && jvm.is_instance(self.runtime_id, &java, "java.lang.Thread")
                        {
                            return self.pump_join(&java, &args);
                        }
                        if name == "start"
                            && jvm.is_instance(self.runtime_id, &java, "java.lang.Thread")
                        {
                            self.async_threads.push(java.clone());
                        }
                        let params = if args_need_param_types(&args) {
                            jvm.method_params(
                                self.runtime_id,
                                Some(&java),
                                java.class_name(),
                                name,
                                args.len(),
                                false,
                            )
                        } else {
                            None
                        };
                        let vals = self.jval_args_typed(params.as_deref(), &args)?;
                        return match jvm.call(&java, name, &vals) {
                            Ok(v) => Ok(crate::java::from_jval(v)),
                            Err(bsh_jvm::JError::Thrown(t)) => Err(Error::Thrown(Value::Java(t))),
                            Err(e) => Err(Error::eval(format!("{}", e))),
                        };
                    }
                }
                self.call_on_class(&class, name, args, ns, Some(base.clone()))
            }
            Value::Java(o) => {
                let jvm = o.jvm().clone();
                if name == "join" && jvm.is_instance(self.runtime_id, o, "java.lang.Thread") {
                    return self.pump_join(o, &args);
                }
                if name == "start" && jvm.is_instance(self.runtime_id, o, "java.lang.Thread") {
                    self.async_threads.push(o.clone());
                }
                let params = if args_need_param_types(&args) {
                    jvm.method_params(
                        self.runtime_id,
                        Some(o),
                        o.class_name(),
                        name,
                        args.len(),
                        false,
                    )
                } else {
                    None
                };
                let vals = self.jval_args_typed(params.as_deref(), &args)?;
                match jvm.call(o, name, &vals) {
                    Ok(v) => Ok(crate::java::from_jval(v)),
                    Err(bsh_jvm::JError::Thrown(t)) => Err(Error::Thrown(Value::Java(t))),
                    Err(e) => Err(Error::eval(format!("{}", e))),
                }
            }
            Value::Builtin(b) => b.borrow_mut().call_method(self, name, args),
            Value::Str(s) => builtins::string_method(self, s, name, args),
            Value::Class(c) => self.call_on_class(&c, name, args, ns, None),
            Value::Namespace(n) => {
                match name {
                    "getInvocationLine" => return Ok(Value::Int(-1)),
                    "getInvocationText" => return Ok(Value::string("")),
                    "getSourceFileInfo" => {
                        return Ok(Value::string(self.source_file.clone().unwrap_or_default()))
                    }
                    "getThis" => return Ok(base.clone()),
                    "getClass" if args.len() == 1 => {
                        if let Value::Str(s) = &args[0] {
                            let name = s.to_string();
                            return Ok(self
                                .resolve_class(&name)
                                .map(Value::Class)
                                .unwrap_or(Value::Null));
                        }
                        return Ok(Value::Null);
                    }
                    "get" if args.len() == 1 => {
                        if let Value::Str(s) = &args[0] {
                            return Ok(Namespace::lookup_value(n, s).unwrap_or(Value::Void));
                        }
                        return Ok(Value::Void);
                    }
                    "set" if args.len() == 2 => {
                        if let Value::Str(s) = &args[0] {
                            let key = s.to_string();
                            n.borrow_mut().vars.insert(
                                key,
                                crate::namespace::Variable {
                                    value: args[1].clone(),
                                    ty: None,
                                    is_final: false,
                                    is_static: false,
                                },
                            );
                        }
                        return Ok(Value::Void);
                    }
                    "getVariable" if args.len() == 1 => {
                        if let Value::Str(s) = &args[0] {
                            return Ok(Namespace::lookup_value(n, s).unwrap_or(Value::Void));
                        }
                        return Ok(Value::Void);
                    }
                    "setVariable" if args.len() == 2 => {
                        if let Value::Str(s) = &args[0] {
                            let key = s.to_string();
                            n.borrow_mut().vars.insert(
                                key,
                                crate::namespace::Variable {
                                    value: args[1].clone(),
                                    ty: None,
                                    is_final: false,
                                    is_static: false,
                                },
                            );
                        }
                        return Ok(Value::Void);
                    }
                    "clear" => {
                        n.borrow_mut().vars.clear();
                        return Ok(Value::Void);
                    }
                    _ => {}
                }
                if let Some(m) = Namespace::find_method(n, name, args.len()) {
                    return self.invoke_scripted(&m, Some(base.clone()), args);
                }
                if builtins::is_builtin_function(name) {
                    return builtins::call_function(self, name, args, n);
                }
                Err(Error::eval(format!("No such method: {}", name)))
            }
            Value::Array(_) => builtins::array_method(self, base, name, args),
            Value::Null | Value::Void => Err(Error::eval(format!(
                "Null Pointer: cannot invoke method {}() on null",
                name
            ))),
            Value::Method(m) => self.invoke_scripted(&m, None, args),
            Value::Lambda(l) => {
                let m = l.method.clone();
                self.invoke_scripted(&m, None, args)
            }
            Value::MethodRef(m) => {
                let m = m.clone();
                self.invoke_method_ref(&m, args, ns)
            }
            other => Err(Error::eval(format!(
                "No such method: {}() on {}",
                name,
                other.type_name()
            ))),
        }
    }

    pub fn call_on_class(
        &mut self,
        class: &Rc<ClassValue>,
        name: &str,
        args: Vec<Value>,
        _ns: &Ns,
        this: Option<Value>,
    ) -> Result<Value> {
        if class.name == "java.lang.Thread" && name == "sleep" && !args.is_empty() {
            let millis = args[0].as_i64()?;
            return self.pump_sleep(millis);
        }
        if class.builtin == Some("java") {
            let jvm = self.jvm().ok_or_else(|| Error::eval("No JVM available"))?;
            if let Some(params) =
                jvm.method_params(self.runtime_id, None, &class.name, name, args.len(), true)
            {
                let vals = self.jval_args_typed(Some(&params), &args)?;
                match jvm.call_static(self.runtime_id, &class.name, name, &vals) {
                    Ok(v) => return Ok(crate::java::from_jval(v)),
                    // Class literals can also call java.lang.Class methods
                    // (SomeClass.class.getName()); fall through to that path.
                    Err(bsh_jvm::JError::Thrown(t))
                        if t.class_name == "java.lang.NoSuchMethodException" => {}
                    Err(bsh_jvm::JError::Thrown(t)) => return Err(Error::Thrown(Value::Java(t))),
                    Err(e) => return Err(Error::eval(format!("{}", e))),
                }
            }
        }
        if let Some(m) = class.find_method(name, args.len()) {
            return self.invoke_scripted(&m, this, args);
        }
        if let Some(kind) = class.builtin {
            if let Some(r) = builtins::instance_method(self, class, kind, name, &args) {
                return r;
            }
        }
        if this.is_none() {
            if let Some(r) = builtins::static_call(self, class, name, &args) {
                match r {
                    Ok(v) => return Ok(v),
                    Err(e) => {
                        // Some Java statics are only reachable through the JVM.
                        if let Some(v) = self.try_jvm_static(class, name, &args) {
                            return Ok(v);
                        }
                        return Err(e);
                    }
                }
            }
            if let Some(v) = self.try_jvm_static(class, name, &args) {
                return Ok(v);
            }
        }
        // Object protocol fallbacks.
        match name {
            "toString" if args.is_empty() => match &this {
                Some(v) => Ok(Value::string(v.to_display_string())),
                None => Ok(Value::string(class.name.clone())),
            },
            "hashCode" if args.is_empty() => Ok(Value::Int(match &this {
                Some(Value::Object(o)) => Rc::as_ptr(o) as usize as i32,
                _ => 0,
            })),
            "equals" if args.len() == 1 => Ok(Value::Bool(match &this {
                Some(v) => v.equality(&args[0]),
                None => false,
            })),
            "getClass" if args.is_empty() => Ok(Value::Class(class.clone())),
            _ => {
                // Class literals also behave as java.lang.Class instances:
                // SomeClass.class.getName(), .getSuperclass(), ...
                {
                    if let Some(jvm) = self.jvm() {
                        if let Ok(class_object) = jvm.class_ref(self.runtime_id, &class.name) {
                            let params = jvm.method_params(
                                self.runtime_id,
                                Some(class_object.as_ref()),
                                "java.lang.Class",
                                name,
                                args.len(),
                                false,
                            );
                            if params.is_some() {
                                let values = self.jval_args_typed(params.as_deref(), &args)?;
                                return match jvm.call(&class_object, name, &values) {
                                    Ok(value) => Ok(crate::java::from_jval(value)),
                                    Err(bsh_jvm::JError::Thrown(thrown)) => {
                                        Err(Error::Thrown(Value::Java(thrown)))
                                    }
                                    Err(error) => Err(Error::eval(format!("{}", error))),
                                };
                            }
                        }
                    }
                }
                if let Some(v) = builtins::exception_method(class, name, &args, this.as_ref()) {
                    return v;
                }
                match &this {
                    Some(v) => Err(Error::eval(format!(
                        "No such method: {}() on {}",
                        name,
                        v.type_name()
                    ))),
                    None => Err(Error::eval(format!(
                        "No such static method: {}.{}()",
                        class.name, name
                    ))),
                }
            }
        }
    }

    pub fn invoke_scripted(
        &mut self,
        m: &Rc<MethodDef>,
        this: Option<Value>,
        args: Vec<Value>,
    ) -> Result<Value> {
        self.method_name_stack.push(m.name.clone());
        let previous_file = self.source_file.clone();
        let previous_text = self.source_text.clone();
        if m.source_file.is_some() {
            self.source_file = m.source_file.clone();
        }
        if m.source_text.is_some() {
            self.source_text = m.source_text.clone();
        }
        let result = self.invoke_scripted_inner(m, this, args);
        // Decorate while the method is still on the stack so the error records
        // both the failing line and "in method ...".
        let result = match result {
            Err(Error::Eval(message)) => Err(Error::Eval(self.decoration(&message))),
            Err(Error::Thrown(value)) => Err(Error::Eval(
                self.decoration(&format!("TargetError: {}", value.to_display_string())),
            )),
            other => other,
        };
        self.method_name_stack.pop();
        self.source_file = previous_file;
        self.source_text = previous_text;
        result
    }

    fn invoke_scripted_inner(
        &mut self,
        m: &Rc<MethodDef>,
        this: Option<Value>,
        args: Vec<Value>,
    ) -> Result<Value> {
        self.push_frame()?;
        let parent = m.ns.clone().unwrap_or_else(|| self.global.clone());
        let frame = Namespace::child(&parent);
        let frame_this = match &this {
            Some(v) => Some(v.clone()),
            // bsh "this" inside a method is the call frame namespace, so
            // nested methods (and bsh.This/Runnable callbacks) can see them.
            None if m.declaring_class.is_none() => Some(Value::Namespace(frame.clone())),
            None => None,
        };
        {
            let mut f = frame.borrow_mut();
            f.this_obj = frame_this;
            f.name = Some(m.name.clone());
        }
        // BeanShell refuses to pass a void value as a method argument.
        if args.iter().any(|a| matches!(a, Value::Void)) {
            return Err(Error::eval("Undefined argument: void"));
        }
        // Bind parameters.
        for (i, p) in m.params.iter().enumerate() {
            let value = if p.varargs && i + 1 == m.params.len() {
                let rest = args[i.min(args.len())..].to_vec();
                Value::Array(Rc::new(RefCell::new(ArrayValue {
                    dims: 1,
                    component: p.ty.clone(),
                    items: rest,
                })))
            } else if i < args.len() {
                match &p.ty {
                    Some(t) => self.cast_value(args[i].clone(), t)?,
                    None => args[i].clone(),
                }
            } else {
                default_value(p.ty.as_ref())
            };
            frame.borrow_mut().vars.insert(
                p.name.clone(),
                crate::namespace::Variable {
                    value,
                    ty: p.ty.clone(),
                    is_final: false,
                    is_static: false,
                },
            );
        }
        // Constructor delegation: explicit super(...) / this(...) prologue,
        // otherwise an implicit super().
        let mut skip_first = 0usize;
        let this_value = Namespace::get_this(&frame);
        if m.is_ctor {
            if let Some(Stmt {
                kind: StmtKind::Block(stmts),
                ..
            }) = m.body.as_deref()
            {
                let mut handled = false;
                if let Some(first) = stmts.first() {
                    if let StmtKind::Expr(Expr {
                        kind: ExprKind::Call { target, args },
                        ..
                    }) = &first.kind
                    {
                        let delegation = match &target.kind {
                            ExprKind::Name(parts) if parts.len() == 1 && parts[0] == "super" => {
                                Some("super")
                            }
                            ExprKind::Name(parts) if parts.len() == 1 && parts[0] == "this" => {
                                Some("this")
                            }
                            ExprKind::Super => Some("super"),
                            ExprKind::This => Some("this"),
                            _ => None,
                        };
                        if let Some(which) = delegation {
                            {
                                let parts = [which.to_string()];
                                let mut vals = Vec::with_capacity(args.len());
                                for a in args {
                                    vals.push(self.eval(a, &frame)?);
                                }
                                if let Some(class) = &m.declaring_class {
                                    let target_class = if which == "super" {
                                        class.superclass.clone()
                                    } else {
                                        Some(class.clone())
                                    };
                                    if let Some(tc) = target_class {
                                        if Self::java_backed(&tc) {
                                            if !vals.is_empty() {
                                                return Err(Error::eval(format!(
                                                    "super(...) with arguments is not supported when {} extends the Java class {}",
                                                    m.name, tc.name
                                                )));
                                            }
                                            // The generated constructor already calls super().
                                        } else if let Some(cm) = tc.ctor(vals.len()) {
                                            self.invoke_ctor(
                                                &cm,
                                                this_value.clone().unwrap_or(Value::Null),
                                                vals,
                                            )?;
                                        }
                                    }
                                }
                                let _ = &parts;
                                skip_first = 1;
                                handled = true;
                            }
                        }
                    }
                }
                if !handled {
                    if let Some(class) = &m.declaring_class {
                        if let Some(sup) = class.superclass.clone() {
                            if let Some(cm) = sup.ctor(0) {
                                self.invoke_ctor(
                                    &cm,
                                    this_value.clone().unwrap_or(Value::Null),
                                    Vec::new(),
                                )?;
                            }
                        }
                    }
                }
            }
        }
        self.method_class_stack.push(m.declaring_class.clone());
        let result = match &m.body {
            Some(body) if skip_first > 0 => {
                let mut flow = Ok(Flow::Normal);
                if let StmtKind::Block(stmts) = &body.kind {
                    for s in &stmts[skip_first..] {
                        match self.exec_stmt(s, &frame) {
                            Ok(Flow::Normal) => {}
                            other => {
                                flow = other;
                                break;
                            }
                        }
                    }
                }
                flow
            }
            Some(body) => match &body.kind {
                // A method body block is the method's namespace: nested method
                // declarations must live in "this", not in an extra child.
                StmtKind::Block(stmts) => self.exec_block(stmts, &frame),
                _ => self.exec_stmt(body, &frame),
            },
            None => Ok(Flow::Normal),
        };
        self.method_class_stack.pop();
        self.pop_frame();
        let flow = result?;
        let value = match flow {
            Flow::Return(v) => v,
            Flow::Normal => Value::Void,
            Flow::Break(_) | Flow::Continue(_) | Flow::Yield(_) => Value::Void,
        };
        match &m.return_type {
            Some(t) if !t.is_void() => self.cast_value(value, t),
            _ => Ok(value),
        }
    }
}

pub fn literal_value(l: &Literal) -> Value {
    match l {
        Literal::Byte(v) => Value::Byte(*v),
        Literal::Short(v) => Value::Short(*v),
        Literal::Int(v) => Value::Int(*v as i32),
        Literal::BigInt(s) => Value::Double(s.parse::<f64>().unwrap_or(f64::INFINITY)),
        Literal::Long(v) => Value::Long(*v),
        Literal::Float(v) => Value::Float(*v),
        Literal::Double(v) => Value::Double(*v),
        Literal::BigDec(s) => Value::Double(s.parse::<f64>().unwrap_or(0.0)),
        Literal::Char(c) => Value::Char(*c),
        Literal::Str(s) => Value::string(s.clone()),
        Literal::Bool(b) => Value::Bool(*b),
        Literal::Null => Value::Null,
        Literal::Void => Value::Void,
    }
}

fn is_primitive_name(name: &str) -> bool {
    matches!(
        name,
        "int" | "long" | "double" | "float" | "short" | "byte" | "char" | "boolean" | "void"
    )
}

fn check_index(i: i64, len: usize) -> Result<usize> {
    if i < 0 || i as usize >= len {
        return Err(Error::eval(format!(
            "Array index out of bounds: {} (length {})",
            i, len
        )));
    }
    Ok(i as usize)
}

fn loose_equals(a: &Value, b: &Value) -> bool {
    if a.is_numeric() && b.is_numeric() {
        return a.as_f64().unwrap_or(f64::NAN) == b.as_f64().unwrap_or(f64::NAN);
    }
    a.equality(b)
}

/// Invoke a method on an object of the given class (used by host conversions).
pub fn invoke_method(
    interp: &mut Interpreter,
    class: &Rc<ClassValue>,
    this: &Value,
    name: &str,
    args: Vec<Value>,
) -> Result<Value> {
    let ns = interp.global.clone();
    interp.call_on_class(class, name, args, &ns, Some(this.clone()))
}

/// Methods of java.lang.Class callable on a class literal (Foo.class).
fn is_class_literal_method(name: &str) -> bool {
    matches!(
        name,
        "getName"
            | "getSimpleName"
            | "getCanonicalName"
            | "getTypeName"
            | "getSuperclass"
            | "getInterfaces"
            | "getModifiers"
            | "isInterface"
            | "isArray"
            | "isPrimitive"
            | "isEnum"
            | "isAnnotation"
            | "getClassLoader"
            | "getPackageName"
            | "newInstance"
            | "cast"
            | "isInstance"
            | "isAssignableFrom"
            | "getDeclaredMethods"
            | "getMethods"
            | "getDeclaredFields"
            | "getFields"
            | "getDeclaredConstructors"
            | "getConstructors"
            | "getDeclaredMethod"
            | "getMethod"
            | "getDeclaredField"
            | "getField"
            | "getAnnotation"
            | "getAnnotations"
            | "getEnumConstants"
            | "desiredAssertionStatus"
            | "toString"
    )
}

fn capitalize_name(name: &str) -> String {
    let mut chars = name.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
        None => String::new(),
    }
}

/// java.lang.Number / Boolean / Character accessors on boxed script values.
fn primitive_wrapper_method(base: &Value, name: &str) -> Option<Value> {
    let integer = match base {
        Value::Byte(v) => *v as i64,
        Value::Short(v) => *v as i64,
        Value::Int(v) => *v as i64,
        Value::Long(v) => *v,
        Value::Char(c) => *c as u32 as i64,
        Value::Float(v) => *v as i64,
        Value::Double(v) => *v as i64,
        _ => {
            return match (base, name) {
                (Value::Bool(v), "booleanValue") => Some(Value::Bool(*v)),
                (Value::Char(c), "charValue") => Some(Value::Char(*c)),
                _ => None,
            };
        }
    };
    let float = match base {
        Value::Float(v) => *v as f64,
        Value::Double(v) => *v,
        _ => integer as f64,
    };
    match name {
        "intValue" => Some(Value::Int(integer as i32)),
        "longValue" => Some(Value::Long(integer)),
        "shortValue" => Some(Value::Short(integer as i16)),
        "byteValue" => Some(Value::Byte(integer as i8)),
        "doubleValue" => Some(Value::Double(float)),
        "floatValue" => Some(Value::Float(float as f32)),
        _ => None,
    }
}

pub fn array_component(parameter: &str) -> Option<&str> {
    parameter.strip_suffix("[]")
}

pub fn expand_varargs_signature(signature: &[String], argc: usize) -> Option<Vec<String>> {
    if signature.is_empty() || argc + 1 < signature.len() {
        return None;
    }
    let last = signature.last()?;
    let component = array_component(last)?;
    let fixed = signature.len() - 1;
    let mut expanded = Vec::with_capacity(argc);
    expanded.extend_from_slice(&signature[..fixed]);
    for _ in fixed..argc {
        expanded.push(component.to_string());
    }
    Some(expanded)
}

pub fn host_arg_tag(value: &Value) -> String {
    match value {
        Value::Void => "V".to_string(),
        Value::Null => "N".to_string(),
        Value::Bool(_) => "Z".to_string(),
        Value::Char(_) => "C".to_string(),
        Value::Byte(_) => "B".to_string(),
        Value::Short(_) => "S".to_string(),
        Value::Int(_) => "I".to_string(),
        Value::Long(_) => "J".to_string(),
        Value::Float(_) => "F".to_string(),
        Value::Double(_) => "D".to_string(),
        Value::Str(_) => "T".to_string(),
        Value::Array(_) => "A".to_string(),
        Value::Object(object) => format!("O:{}", object.borrow().class.name),
        Value::Class(_) => "K".to_string(),
        Value::Java(object) => format!("J:{}", object.class_name),
        Value::Lambda(lambda) => format!("L:{}", lambda.method.params.len()),
        Value::MethodRef(_) => "R".to_string(),
        _ => "?".to_string(),
    }
}

fn is_java_primitive(name: &str) -> bool {
    matches!(
        name,
        "boolean" | "byte" | "char" | "short" | "int" | "long" | "float" | "double"
    )
}

/// Rough Java specificity score for one argument against one parameter type.
/// Higher is better; None means this overload cannot accept the argument.
fn score_host_argument(
    jvm: &bsh_jvm::Jvm,
    runtime: bsh_jvm::RuntimeId,
    value: &Value,
    parameter: &str,
) -> Option<i32> {
    let parameter = parameter.trim();
    if parameter == "java.lang.Object" {
        return Some(1);
    }
    match value {
        Value::Null => {
            if is_java_primitive(parameter) {
                None
            } else {
                Some(1)
            }
        }
        Value::Str(_) => match parameter {
            "java.lang.String" | "String" => Some(5),
            "java.lang.CharSequence" | "CharSequence" => Some(4),
            _ => None,
        },
        Value::Bool(_) => match parameter {
            "boolean" | "java.lang.Boolean" | "Boolean" => Some(5),
            _ => None,
        },
        Value::Char(_) => match parameter {
            "char" | "java.lang.Character" | "Character" => Some(5),
            _ => None,
        },
        Value::Byte(_) => match parameter {
            "byte" | "java.lang.Byte" | "Byte" => Some(5),
            "short" | "java.lang.Short" | "Short" => Some(3),
            "int" | "java.lang.Integer" | "Integer" => Some(2),
            "long" | "java.lang.Long" | "Long" => Some(1),
            _ => None,
        },
        Value::Short(_) => match parameter {
            "short" | "java.lang.Short" | "Short" => Some(5),
            "int" | "java.lang.Integer" | "Integer" => Some(3),
            "long" | "java.lang.Long" | "Long" => Some(2),
            _ => None,
        },
        Value::Int(_) => match parameter {
            "int" | "java.lang.Integer" | "Integer" => Some(5),
            "long" | "java.lang.Long" | "Long" => Some(4),
            "short" | "java.lang.Short" | "Short" => Some(2),
            "byte" | "java.lang.Byte" | "Byte" => Some(1),
            "double" | "java.lang.Double" | "Double" => Some(2),
            "float" | "java.lang.Float" | "Float" => Some(1),
            _ => None,
        },
        Value::Long(_) => match parameter {
            "long" | "java.lang.Long" | "Long" => Some(5),
            "double" | "java.lang.Double" | "Double" => Some(2),
            "float" | "java.lang.Float" | "Float" => Some(1),
            _ => None,
        },
        Value::Float(_) => match parameter {
            "float" | "java.lang.Float" | "Float" => Some(5),
            "double" | "java.lang.Double" | "Double" => Some(4),
            _ => None,
        },
        Value::Double(_) => match parameter {
            "double" | "java.lang.Double" | "Double" => Some(5),
            "float" | "java.lang.Float" | "Float" => Some(1),
            _ => None,
        },
        Value::Java(object) => {
            if object.class_name == parameter {
                Some(5)
            } else if jvm.is_instance(runtime, object, parameter) {
                Some(3)
            } else {
                None
            }
        }
        Value::Lambda(lambda) => {
            if !jvm.is_interface(runtime, parameter) {
                return None;
            }
            match jvm.sam_info(runtime, parameter) {
                Some((arity, _)) if arity == lambda.method.params.len() => Some(4),
                Some(_) => None,
                None => Some(2),
            }
        }
        Value::MethodRef(_) => {
            if jvm.is_interface(runtime, parameter) {
                Some(3)
            } else {
                None
            }
        }
        Value::Object(object) => {
            if object.borrow().java.is_none() && jvm.is_interface(runtime, parameter) {
                Some(2)
            } else {
                None
            }
        }
        Value::Array(_) => {
            if array_component(parameter).is_some() {
                Some(2)
            } else {
                None
            }
        }
        Value::Class(_) => match parameter {
            "java.lang.Class" | "Class" => Some(5),
            _ => None,
        },
        _ => None,
    }
}

/// Only lambda / script-object arguments need the parameter type list (to
/// build Java interface proxies). Plain values can skip the extra reflection
/// scan entirely.
fn args_need_param_types(args: &[Value]) -> bool {
    args.iter().any(|arg| match arg {
        Value::Lambda(_) | Value::MethodRef(_) => true,
        Value::Object(object) => object.borrow().java.is_none(),
        _ => false,
    })
}
