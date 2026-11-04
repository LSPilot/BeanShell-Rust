//! The interpreter: global state, class registry, output and invocation helpers.

use crate::builtins;
use crate::class::{ClassDef, ClassValue};
use crate::error::{Error, Result};
use crate::namespace::Namespace;
use crate::value::{MethodDef, Value};
use bsh_syntax::ast::{Modifiers, Param, Program, Stmt, StmtKind, TypeRef};
use std::cell::RefCell;
use std::collections::HashMap;
use std::io::{self, Write};
use std::path::PathBuf;
use std::rc::Rc;

pub type Ns = Rc<RefCell<Namespace>>;

pub type OutSink = Rc<RefCell<Box<dyn Write>>>;

/// A Java method registered by the host (LSPilot plugin SDK).
#[derive(Clone)]
pub struct HostMethod {
    pub name: String,
    pub argc: usize,
    pub varargs: bool,
    pub class_name: String,
    pub instance: Option<std::sync::Arc<bsh_jvm::JObj>>,
}

pub struct Interpreter {
    pub global: Ns,
    pub classes: HashMap<String, Rc<ClassValue>>,
    /// The embedded JVM, when one is available (disable with BSH_RS_JVM=0).
    pub jvm: Option<std::sync::Arc<bsh_jvm::Jvm>>,
    /// Whether Java interop is allowed (BSH_RS_JVM != 0).
    pub jvm_enabled: bool,
    /// Set once the script actually used Java; drain callback errors only then.
    pub java_used: std::cell::Cell<bool>,
    /// Callbacks arriving from Java threads, drained at blocking points.
    pub callback_queue: std::sync::Arc<crate::proxy::CallbackQueue>,
    /// Declaring class of each method currently executing (for super calls).
    pub method_class_stack: Vec<Option<Rc<ClassValue>>>,
    /// Java threads started by the current script; after evaluation we wait for
    /// them (pumping callbacks) so their output is not lost.
    pub async_threads: Vec<std::sync::Arc<bsh_jvm::JObj>>,
    /// import static: (class, Some(member) or None for wildcard).
    pub static_imports: Vec<(String, Option<String>)>,
    /// Host-registered Java methods callable from scripts by name.
    pub host_methods: HashMap<String, Vec<HostMethod>>,
    /// Resolved (name, argc, argument-tag) -> (host index, signature).
    pub host_method_cache: HashMap<(String, usize, Vec<String>), (usize, Vec<String>)>,
    /// Owning plugin runtime (0 for CLI/host tests).
    pub runtime_id: bsh_jvm::RuntimeId,
    /// Class names that failed to resolve; avoids repeated Class.forName
    /// probes for things like "Integer.MAX_VALUE" on every field access.
    pub missing_classes: std::collections::HashSet<String>,
    pub out: OutSink,
    pub cwd: PathBuf,
    pub source_file: Option<String>,
    /// Source of the code currently being evaluated (for error excerpts).
    pub source_text: Option<String>,
    /// Span of the statement/expression currently being evaluated.
    pub current_span: Option<bsh_syntax::span::Span>,
    /// Names of script methods on the call stack (innermost last).
    pub method_name_stack: Vec<String>,
    pub depth: usize,
    pub last_value: Value,
    /// Set while a class body is being defined, so methods capture the class.
    pub current_class: Vec<Rc<ClassValue>>,
}

impl Interpreter {
    pub fn new(out: Box<dyn Write>) -> Interpreter {
        let global = Namespace::root();
        crate::proxy::install();
        // The JVM is started lazily on the first Java interop use; plain
        // scripts should not pay the ~50ms JNI_CreateJavaVM cost.
        let jvm_enabled = std::env::var("BSH_RS_JVM").as_deref() != Ok("0");
        let mut interp = Interpreter {
            global,
            classes: HashMap::new(),
            jvm: None,
            jvm_enabled,
            java_used: std::cell::Cell::new(false),
            callback_queue: crate::proxy::new_queue(),
            method_class_stack: Vec::new(),
            async_threads: Vec::new(),
            static_imports: Vec::new(),
            host_methods: HashMap::new(),
            host_method_cache: HashMap::new(),
            runtime_id: bsh_jvm::RuntimeId::default(),
            missing_classes: std::collections::HashSet::new(),
            out: Rc::new(RefCell::new(out)),
            cwd: std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")),
            source_file: None,
            source_text: None,
            current_span: None,
            method_name_stack: Vec::new(),
            depth: 0,
            last_value: Value::Void,
            current_class: Vec::new(),
        };
        builtins::install_classes(&mut interp);
        interp
    }

    /// An interpreter used only for host driven conversions (toString).
    pub fn detached() -> Interpreter {
        Interpreter::new(Box::new(io::sink()))
    }

    pub fn write_out(&mut self, text: &str) {
        let mut sink = self.out.borrow_mut();
        let _ = sink.write_all(text.as_bytes());
        let _ = sink.flush();
    }

    pub fn write_line(&mut self, text: &str) {
        self.write_out(text);
        self.write_out("\n");
    }

    pub fn jvm(&self) -> Option<std::sync::Arc<bsh_jvm::Jvm>> {
        if !self.jvm_enabled {
            return None;
        }
        let jvm = if let Some(jvm) = &self.jvm {
            Some(jvm.clone())
        } else {
            bsh_jvm::global()
        };
        if jvm.is_some() {
            self.java_used.set(true);
        }
        jvm
    }

    /// Convert interpreter values to JNI values (for host calls).
    pub fn jval_args(&mut self, args: &[Value]) -> crate::error::Result<Vec<bsh_jvm::JVal>> {
        let mut out = Vec::with_capacity(args.len());
        for a in args {
            out.push(crate::java::to_jval(self, a)?);
        }
        Ok(out)
    }

    /// Resolve a class name, falling back to the embedded JVM for classes that
    /// are not modelled in Rust.
    pub fn resolve_class(&mut self, name: &str) -> Option<Rc<ClassValue>> {
        if let Some(c) = self.find_class(name) {
            return Some(c);
        }
        if self.missing_classes.contains(name) {
            return None;
        }
        let jvm = self.jvm()?;
        let mut candidates: Vec<String> = vec![name.to_string()];
        for prefix in [
            "java.lang.",
            "java.util.",
            "java.util.function.",
            "java.util.concurrent.",
            "java.io.",
            "java.math.",
            "java.time.",
        ] {
            candidates.push(format!("{}{}", prefix, name));
        }
        // Wildcard imports in scope.
        let imports: Vec<String> = self.global.borrow().imports.clone();
        for import in imports {
            if let Some(package) = import.strip_suffix(".*") {
                candidates.push(format!("{}.{}", package, name));
            }
        }
        if name.ends_with("[]") {
            candidates.push(name.to_string());
        }
        for candidate in candidates {
            // Try nested-class forms too: a.b.Outer.Inner -> a.b.Outer$Inner.
            let mut variants = vec![candidate.clone()];
            let mut current = candidate.clone();
            while let Some(position) = current.rfind('.') {
                current.replace_range(position..position + 1, "$");
                variants.push(current.clone());
            }
            for variant in variants {
                if self.missing_classes.contains(&variant) {
                    continue;
                }
                if !jvm.class_exists(self.runtime_id, &variant) {
                    self.missing_classes.insert(variant);
                    continue;
                }
                let object = self.find_class("java.lang.Object");
                let class = Rc::new(ClassValue::new_builtin(variant.clone(), "java", object));
                let simple = variant
                    .rsplit(['.', '$'])
                    .next()
                    .unwrap_or(&variant)
                    .to_string();
                self.classes.insert(variant.clone(), class.clone());
                self.classes.insert(simple.clone(), class.clone());
                self.global
                    .borrow_mut()
                    .classes
                    .insert(simple, class.clone());
                self.global
                    .borrow_mut()
                    .classes
                    .insert(variant, class.clone());
                return Some(class.clone());
            }
        }
        None
    }

    pub fn register_class(&mut self, name: &str, class: Rc<ClassValue>) {
        self.classes.insert(name.to_string(), class.clone());
        let simple = name.rsplit('.').next().unwrap_or(name).to_string();
        self.global.borrow_mut().classes.insert(simple, class);
    }

    /// Return (creating on demand) a class handle for a primitive or array type.
    pub fn class_for_name(&mut self, name: &str) -> Rc<ClassValue> {
        if let Some(c) = self.classes.get(name) {
            return c.clone();
        }
        let object = self.find_class("java.lang.Object");
        let c = Rc::new(ClassValue::new_builtin(name, "", object));
        self.classes.insert(name.to_string(), c.clone());
        c
    }

    pub fn find_class(&self, name: &str) -> Option<Rc<ClassValue>> {
        if let Some(c) = self.classes.get(name) {
            return Some(c.clone());
        }
        if let Some(c) = Namespace::find_class(&self.global, name) {
            return Some(c);
        }
        for prefix in [
            "java.lang.",
            "java.util.",
            "java.util.function.",
            "java.util.concurrent.",
            "java.io.",
            "bsh.",
            "java.math.",
            "java.time.",
        ] {
            let full = format!("{}{}", prefix, name);
            if let Some(c) = self.classes.get(&full) {
                return Some(c.clone());
            }
        }
        None
    }

    pub fn global_ns(&self) -> Ns {
        self.global.clone()
    }

    /// Parse and evaluate a complete source string in the global namespace.
    pub fn eval_source(&mut self, src: &str) -> Result<Value> {
        let _guard = crate::proxy::InterpGuard::new(self);
        let previous_text = self.source_text.clone();
        self.source_text = Some(src.to_string());
        if self.source_file.is_none() {
            self.source_file = Some("eval string".to_string());
        }
        let result = match bsh_syntax::parse(src) {
            Ok(program) => {
                let value = self.eval_program(&program);
                let value = self.decorate_result(value);
                self.drain_java_errors();
                value
            }
            Err(error) => Err(Error::eval(Self::parse_error_message(src, &error))),
        };
        self.source_text = previous_text;
        result
    }

    /// Parse and evaluate in an explicit namespace.
    pub fn eval_source_in(&mut self, src: &str, ns: &Ns) -> Result<Value> {
        let _guard = crate::proxy::InterpGuard::new(self);
        let previous_text = self.source_text.clone();
        self.source_text = Some(src.to_string());
        if self.source_file.is_none() {
            self.source_file = Some("eval string".to_string());
        }
        let result = match bsh_syntax::parse(src) {
            Ok(program) => {
                let value = self.eval_program_in(&program, ns);
                let value = self.decorate_result(value);
                self.drain_java_errors();
                value
            }
            Err(error) => Err(Error::eval(Self::parse_error_message(src, &error))),
        };
        self.source_text = previous_text;
        result
    }

    /// Print callback errors captured on Android worker threads.
    pub fn drain_java_errors(&mut self) {
        if !self.java_used.get() {
            return;
        }
        let Some(jvm) = self.jvm() else {
            return;
        };
        if let Ok(bsh_jvm::JVal::Str(text)) =
            jvm.call_static(self.runtime_id, "bshrs.Bridge", "drainErrors", &[])
        {
            if !text.is_empty() {
                self.write_line(&text);
            }
        }
    }

    pub fn eval_file(&mut self, path: &str) -> Result<Value> {
        let full = if PathBuf::from(path).is_absolute() {
            PathBuf::from(path)
        } else {
            self.cwd.join(path)
        };
        let src = std::fs::read_to_string(&full)
            .map_err(|e| Error::eval(format!("Cannot read file: {}: {}", full.display(), e)))?;
        let previous = self.source_file.clone();
        self.source_file = Some(path.to_string());
        let r = self.eval_source(&src);
        self.source_file = previous;
        r
    }

    /// Append "at file:line:col" plus the source line and a caret to an error.
    pub fn decoration(&self, message: &str) -> String {
        if message.contains("\n    at ") {
            return message.to_string();
        }
        let Some(span) = self.current_span else {
            return message.to_string();
        };
        let file = self
            .source_file
            .clone()
            .unwrap_or_else(|| "eval string".to_string());
        let mut out = format!("{}\n    at {}:{}:{}", message, file, span.line, span.col);
        if let Some(text) = &self.source_text {
            if let Some(line) = text.lines().nth(span.line.saturating_sub(1) as usize) {
                out.push_str(&format!("\n    {}", line.trim_end()));
                let column = span.col.saturating_sub(1) as usize;
                out.push_str(&format!("\n    {}^", " ".repeat(column)));
            }
        }
        if let Some(method) = self.method_name_stack.last() {
            out.push_str(&format!("\n    in method {}", method));
        }
        out
    }

    fn decorate_result(&self, result: Result<Value>) -> Result<Value> {
        match result {
            Err(Error::Eval(message)) => Err(Error::Eval(self.decoration(&message))),
            Err(Error::Thrown(value)) => {
                let text = value.to_display_string();
                Err(Error::Eval(
                    self.decoration(&format!("TargetError: {}", text)),
                ))
            }
            other => other,
        }
    }

    fn parse_error_message(src: &str, error: &bsh_syntax::SyntaxError) -> String {
        let mut out = format!(
            "{}: line {}:{}",
            error.message, error.span.line, error.span.col
        );
        if let Some(line) = src.lines().nth(error.span.line.saturating_sub(1) as usize) {
            out.push_str(&format!("\n    {}", line.trim_end()));
            out.push_str(&format!(
                "\n    {}^",
                " ".repeat(error.span.col.saturating_sub(1) as usize)
            ));
        }
        out
    }

    pub fn set_variable_value(&mut self, name: &str, value: Value) {
        let global = self.global.clone();
        self.define_var(&global, name, value, None);
    }

    pub fn get_variable_value(&self, name: &str) -> Option<Value> {
        Namespace::lookup_value(&self.global, name)
    }

    pub fn add_import_name(&mut self, name: &str) {
        self.missing_classes.clear();
        self.global.borrow_mut().imports.push(name.to_string());
        if !name.ends_with(".*") {
            // Import a concrete class so its simple name resolves.
            let _ = self.resolve_class(name);
        }
    }

    pub fn define_host_method(
        &mut self,
        name: &str,
        argc: usize,
        class_name: &str,
        instance: Option<std::sync::Arc<bsh_jvm::JObj>>,
        varargs: bool,
    ) {
        self.host_method_cache.clear();
        self.host_methods
            .entry(name.to_string())
            .or_default()
            .push(HostMethod {
                name: name.to_string(),
                argc,
                varargs,
                class_name: class_name.to_string(),
                instance,
            });
    }

    /// (name, parameter count) for every top-level script method.
    pub fn script_methods(&self) -> Vec<(String, usize)> {
        let mut out: Vec<(String, usize)> = Vec::new();
        {
            let global = self.global.borrow();
            for (name, methods) in &global.methods {
                for method in methods {
                    let entry = (name.clone(), method.params.len());
                    if !out.contains(&entry) {
                        out.push(entry);
                    }
                }
            }
        }
        out.sort();
        out
    }

    pub fn invoke_script_method(&mut self, name: &str, args: Vec<Value>) -> Result<Value> {
        let argc = args.len();
        let method = {
            let global = self.global.borrow();
            global.methods.get(name).and_then(|methods| {
                methods
                    .iter()
                    .find(|m| m.params.len() == argc || crate::value::has_varargs(&m.params, argc))
                    .cloned()
            })
        };
        if let Some(method) = method {
            return self.invoke_scripted(&method, None, args);
        }
        if let Some(Value::Lambda(lambda)) = self.get_variable_value(name) {
            return self.invoke_scripted(&lambda.method, None, args);
        }
        Err(Error::eval(format!(
            "No script method {}() with {} arguments",
            name, argc
        )))
    }

    pub fn define_var(&mut self, ns: &Ns, name: &str, value: Value, ty: Option<TypeRef>) {
        ns.borrow_mut().vars.insert(
            name.to_string(),
            crate::namespace::Variable {
                value,
                ty,
                is_final: false,
                is_static: false,
            },
        );
    }

    pub fn define_method(&mut self, ns: &Ns, m: Rc<MethodDef>) {
        ns.borrow_mut()
            .methods
            .entry(m.name.clone())
            .or_default()
            .push(m);
    }

    pub fn make_method(
        &self,
        ns: &Ns,
        name: &str,
        params: Vec<Param>,
        body: Option<Box<Stmt>>,
        return_type: Option<TypeRef>,
        modifiers: Modifiers,
        is_ctor: bool,
        declaring_class: Option<Rc<ClassValue>>,
    ) -> Rc<MethodDef> {
        Rc::new(MethodDef {
            name: name.to_string(),
            params,
            body,
            return_type,
            ns: Some(ns.clone()),
            declaring_class,
            is_ctor,
            modifiers,
            source_file: self.source_file.clone(),
            source_text: self.source_text.clone(),
        })
    }

    /// Build a scripted class from its AST declaration.
    pub fn define_class(
        &mut self,
        ns: &Ns,
        decl: &bsh_syntax::ast::ClassDecl,
    ) -> Result<Rc<ClassValue>> {
        let superclass = match &decl.extends {
            Some(t) => self.resolve_class(&t.base_name()),
            None => None,
        };
        let def = Rc::new(ClassDef {
            name: decl.name.clone(),
            is_interface: decl.kind == bsh_syntax::ast::ClassKind::Interface,
            methods: Vec::new(),
            fields: Vec::new(),
            instance_init: Vec::new(),
            static_init: Vec::new(),
            enum_constants: Vec::new(),
        });
        let class = Rc::new(ClassValue::new_scripted(decl.name.clone(), def, superclass));
        // Members must be collected before methods are created so that
        // recursive references resolve.
        self.current_class.push(class.clone());
        ns.borrow_mut()
            .classes
            .insert(decl.name.clone(), class.clone());
        self.register_class(&decl.name, class.clone());

        // A namespace shared by all members of the class body.
        let class_ns = Namespace::child(ns);
        {
            let mut b = class_ns.borrow_mut();
            b.this_obj = Some(Value::Class(class.clone()));
            b.name = Some(decl.name.clone());
        }
        let mut methods: Vec<Rc<MethodDef>> = Vec::new();
        let mut fields = Vec::new();
        let mut instance_init = Vec::new();
        let mut static_init = Vec::new();
        for member in &decl.members {
            match &member.kind {
                StmtKind::Method(md) => {
                    let m = self.make_method(
                        &class_ns,
                        &md.name,
                        md.params.clone(),
                        md.body.clone(),
                        md.return_type.clone(),
                        md.modifiers,
                        md.is_ctor,
                        Some(class.clone()),
                    );
                    methods.push(m);
                }
                StmtKind::VarDecl(vd) => {
                    for var in &vd.vars {
                        let ty = vd.ty.clone();
                        if vd.modifiers.is_static {
                            // Static fields are evaluated lazily below.
                            let value = match &var.init {
                                Some(e) => self.eval(e, &class_ns).unwrap_or(Value::Void),
                                None => default_value(ty.as_ref()),
                            };
                            class.statics.borrow_mut().insert(var.name.clone(), value);
                        } else {
                            fields.push(crate::value::FieldDef {
                                name: var.name.clone(),
                                ty,
                                modifiers: vd.modifiers,
                                init: var.init.clone(),
                            });
                        }
                    }
                }
                StmtKind::Class(_) | StmtKind::Block(_) | StmtKind::Expr(_) => {
                    if member_is_static(member) {
                        static_init.push(member.clone());
                    } else {
                        instance_init.push(member.clone());
                    }
                }
                _ => {
                    instance_init.push(member.clone());
                }
            }
        }
        // Replace the placeholder definition with the populated one.
        let def = Rc::new(ClassDef {
            name: decl.name.clone(),
            is_interface: decl.kind == bsh_syntax::ast::ClassKind::Interface,
            methods,
            fields,
            instance_init,
            static_init,
            enum_constants: Vec::new(),
        });
        let final_class = Rc::new(ClassValue {
            name: decl.name.clone(),
            superclass: class.superclass.clone(),
            interfaces: decl.implements.iter().map(|t| t.base_name()).collect(),
            def: Some(def),
            statics: RefCell::new(class.statics.borrow().clone()),
            is_interface: decl.kind == bsh_syntax::ast::ClassKind::Interface,
            builtin: None,
        });
        // Enum constants become static fields of the class.
        self.current_class.pop();
        ns.borrow_mut()
            .classes
            .insert(decl.name.clone(), final_class.clone());
        self.register_class(&decl.name, final_class.clone());
        let _ = class_ns;
        Ok(final_class)
    }

    pub fn push_frame(&mut self) -> Result<()> {
        self.depth += 1;
        if self.depth > 2000 {
            return Err(Error::eval("Method call stack overflow"));
        }
        Ok(())
    }

    pub fn pop_frame(&mut self) {
        self.depth = self.depth.saturating_sub(1);
    }

    /// Keep servicing callbacks queued by Java threads after the script has
    /// finished, until the queue is quiet for `quiet_ms` (bounded by `max_ms`).
    pub fn settle_callbacks(&mut self, quiet_ms: u64, max_ms: u64) -> usize {
        let _guard = crate::proxy::InterpGuard::new(self);
        let start = std::time::Instant::now();
        let mut last_activity = start;
        let mut total = 0usize;
        loop {
            let processed = crate::proxy::pump(self);
            if processed > 0 {
                total += processed;
                last_activity = std::time::Instant::now();
            }
            if start.elapsed() >= std::time::Duration::from_millis(max_ms) {
                break;
            }
            if last_activity.elapsed() >= std::time::Duration::from_millis(quiet_ms) {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
        total
    }

    /// Wait for the threads started by the script, servicing their callbacks.
    /// Bounded so a long-lived thread cannot block the caller forever.
    pub fn await_started_threads(&mut self, max_ms: u64) {
        let handles = std::mem::take(&mut self.async_threads);
        if handles.is_empty() {
            return;
        }
        let _guard = crate::proxy::InterpGuard::new(self);
        let mut threads = handles;
        let start = std::time::Instant::now();
        loop {
            crate::proxy::pump(self);
            while let Some(extra) = self.async_threads.pop() {
                threads.push(extra);
            }
            let mut alive = false;
            for thread in &threads {
                if let Ok(bsh_jvm::JVal::Bool(true)) = thread.jvm().call(thread, "isAlive", &[]) {
                    alive = true;
                    break;
                }
            }
            if !alive {
                break;
            }
            if start.elapsed() >= std::time::Duration::from_millis(max_ms) {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
        crate::proxy::pump(self);
    }

    pub fn eval_program(&mut self, program: &Program) -> Result<Value> {
        let ns = self.global.clone();
        self.eval_program_in(program, &ns)
    }

    pub fn eval_program_in(&mut self, program: &Program, ns: &Ns) -> Result<Value> {
        let mut last = Value::Void;
        for stmt in &program.stmts {
            match self.exec_stmt(stmt, ns)? {
                crate::eval::Flow::Normal => {}
                crate::eval::Flow::Return(v) => {
                    last = v;
                    break;
                }
                crate::eval::Flow::Break(_)
                | crate::eval::Flow::Continue(_)
                | crate::eval::Flow::Yield(_) => break,
            }
            last = self.last_value.clone();
        }
        Ok(last)
    }
}

pub fn is_static_member(m: &Stmt) -> bool {
    member_is_static(m)
}

fn member_is_static(m: &Stmt) -> bool {
    match &m.kind {
        StmtKind::Method(md) => md.modifiers.is_static,
        StmtKind::VarDecl(vd) => vd.modifiers.is_static,
        _ => false,
    }
}

pub fn default_value(ty: Option<&TypeRef>) -> Value {
    let ty = match ty {
        Some(t) => t,
        None => return Value::Void,
    };
    if ty.dims > 0 {
        return Value::Null;
    }
    match ty.primitive {
        Some(bsh_syntax::ast::PrimKind::Boolean) => Value::Bool(false),
        Some(bsh_syntax::ast::PrimKind::Char) => Value::Char('\0'),
        Some(bsh_syntax::ast::PrimKind::Byte) => Value::Byte(0),
        Some(bsh_syntax::ast::PrimKind::Short) => Value::Short(0),
        Some(bsh_syntax::ast::PrimKind::Int) => Value::Int(0),
        Some(bsh_syntax::ast::PrimKind::Long) => Value::Long(0),
        Some(bsh_syntax::ast::PrimKind::Float) => Value::Float(0.0),
        Some(bsh_syntax::ast::PrimKind::Double) => Value::Double(0.0),
        Some(bsh_syntax::ast::PrimKind::Void) => Value::Void,
        None => Value::Null,
    }
}
