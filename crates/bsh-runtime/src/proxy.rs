//! Script values exposed to Java as dynamic proxies.
//!
//! A script lambda, method reference or scripted object can be handed to a Java
//! API that expects an interface. We create a `java.lang.reflect.Proxy` whose
//! handler calls back into this module through a JNI native method, then back
//! into the evaluator.
//!
//! Callbacks normally happen on the interpreter thread and are dispatched
//! directly. When Java invokes the proxy from another thread the request is
//! queued and the interpreter drains the queue at cooperative blocking points
//! (Thread.join, Thread.sleep), so the script still runs single threaded.

use crate::error::{Error, Result};
use crate::interp::Interpreter;
use crate::value::{LambdaValue, Value};
use bsh_jvm::JVal;
use std::cell::{Cell, RefCell};
use std::collections::{HashMap, VecDeque};
use std::rc::Rc;
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::mpsc::{self, Sender};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

thread_local! {
    static CURRENT_OWNER: Cell<u64> = Cell::new(0);
}

static PROXY_OWNERS: OnceLock<Mutex<HashMap<i64, u64>>> = OnceLock::new();

/// Tag proxies created from now on with the owning engine id.
pub fn set_current_owner(engine: u64) {
    CURRENT_OWNER.with(|cell| cell.set(engine));
}

pub fn owner_of(handle: i64) -> Option<u64> {
    PROXY_OWNERS.get()?.lock().ok()?.get(&handle).copied()
}

/// Drop every proxy/target/queue owned by an engine that is being unloaded.
pub fn release_owner(engine: u64) {
    let handles: Vec<i64> = {
        let Some(owners) = PROXY_OWNERS.get() else {
            return;
        };
        let Ok(owners) = owners.lock() else { return };
        owners
            .iter()
            .filter(|(_, owner)| **owner == engine)
            .map(|(handle, _)| *handle)
            .collect()
    };
    if handles.is_empty() {
        return;
    }
    if let Some(owners) = PROXY_OWNERS.get() {
        if let Ok(mut owners) = owners.lock() {
            for handle in &handles {
                owners.remove(handle);
            }
        }
    }
    if let Ok(mut proxies) = proxies().lock() {
        for handle in &handles {
            proxies.remove(handle);
        }
    }
    if let Ok(mut queues) = queues().lock() {
        for handle in &handles {
            queues.remove(handle);
        }
    }
}

fn record_owner(handle: i64) {
    let engine = CURRENT_OWNER.with(|cell| cell.get());
    if engine == 0 {
        return;
    }
    let owners = PROXY_OWNERS.get_or_init(|| Mutex::new(HashMap::new()));
    if let Ok(mut owners) = owners.lock() {
        owners.insert(handle, engine);
    }
}

#[derive(Clone)]
pub enum ProxyTarget {
    Lambda(Rc<LambdaValue>),
    /// A scripted object; calls are dispatched by method name.
    Scripted(Value),
    /// A method reference invoked with the callback arguments.
    MethodRef(Rc<crate::value::MethodRefValue>),
    /// A script namespace (bsh "this"); bsh.This implements Runnable.
    Namespace(Rc<RefCell<crate::namespace::Namespace>>),
}

/// A callback received from a Java thread.
struct CallbackRequest {
    handle: i64,
    method: String,
    args: Vec<JVal>,
    reply: Sender<std::result::Result<JVal, String>>,
}

/// Queue of cross-thread callbacks owned by one interpreter.
pub struct CallbackQueue {
    pending: Mutex<VecDeque<CallbackRequest>>,
}

impl CallbackQueue {
    fn new() -> CallbackQueue {
        CallbackQueue {
            pending: Mutex::new(VecDeque::new()),
        }
    }

    fn push(&self, request: CallbackRequest) {
        if let Ok(mut queue) = self.pending.lock() {
            queue.push_back(request);
        }
    }

    fn pop(&self) -> Option<CallbackRequest> {
        self.pending.lock().ok().and_then(|mut q| q.pop_front())
    }
}

thread_local! {
    /// The interpreter that is currently evaluating, used to re-enter it from a
    /// Java callback. Single threaded by construction.
    static CURRENT: Cell<*mut Interpreter> = const { Cell::new(std::ptr::null_mut()) };
}

#[derive(Clone)]
struct SendProxyTarget(ProxyTarget);

// ProxyTarget contains Rc values. Every access happens under the owning
// engine's mutex (or on the interpreter thread itself), so moving the handle
// table to the callback thread is serialized.
unsafe impl Send for SendProxyTarget {}

static PROXIES: OnceLock<Mutex<HashMap<i64, SendProxyTarget>>> = OnceLock::new();

fn proxies() -> &'static Mutex<HashMap<i64, SendProxyTarget>> {
    PROXIES.get_or_init(|| Mutex::new(HashMap::new()))
}

static NEXT_HANDLE: AtomicI64 = AtomicI64::new(1);

/// Maps a proxy handle to the queue of the interpreter that owns it.
static QUEUES: OnceLock<Mutex<HashMap<i64, Arc<CallbackQueue>>>> = OnceLock::new();

fn queues() -> &'static Mutex<HashMap<i64, Arc<CallbackQueue>>> {
    QUEUES.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Installs the callback and makes the current interpreter reachable from it.
pub struct InterpGuard(*mut Interpreter);

impl InterpGuard {
    pub fn new(interp: &mut Interpreter) -> InterpGuard {
        let ptr = interp as *mut Interpreter;
        let previous = CURRENT.with(|c| c.replace(ptr));
        InterpGuard(previous)
    }
}

impl Drop for InterpGuard {
    fn drop(&mut self) {
        CURRENT.with(|c| c.set(self.0));
    }
}

pub fn new_queue() -> Arc<CallbackQueue> {
    Arc::new(CallbackQueue::new())
}

/// Register the proxy dispatcher with the JVM bridge (idempotent).
pub fn install() {
    bsh_jvm::set_proxy_handler(Box::new(dispatch));
}

fn dispatch(handle: i64, method: &str, args: Vec<JVal>) -> std::result::Result<JVal, String> {
    let pointer = CURRENT.with(|c| c.get());
    if !pointer.is_null() {
        // SAFETY: single threaded interpreter; the callback happens while the
        // outer frame is blocked inside the JNI call.
        return unsafe { dispatch_on(pointer, handle, method, args) };
    }
    forward_to_owner(handle, method, args)
}

/// Called on a foreign Java thread: hand the work to the interpreter thread and
/// wait for the result.
fn forward_to_owner(
    handle: i64,
    method: &str,
    args: Vec<JVal>,
) -> std::result::Result<JVal, String> {
    let queue = queues()
        .lock()
        .ok()
        .and_then(|map| map.get(&handle).cloned())
        .ok_or_else(|| format!("Unknown proxy handle {}", handle))?;
    let (sender, receiver) = mpsc::channel();
    queue.push(CallbackRequest {
        handle,
        method: method.to_string(),
        args,
        reply: sender,
    });
    let timeout_ms = std::env::var("BSH_RS_CALLBACK_TIMEOUT_MS")
        .ok()
        .and_then(|v| v.parse::<u64>().ok())
        .unwrap_or(30_000);
    match receiver.recv_timeout(Duration::from_millis(timeout_ms)) {
        Ok(result) => result,
        Err(_) => Err(format!(
            "Script callback {}() timed out (the interpreter never pumped its queue)",
            method
        )),
    }
}

/// Dispatch a proxy callback on the current thread (same interpreter lock).
pub fn dispatch_current(
    interp: &mut Interpreter,
    handle: i64,
    method: &str,
    args: Vec<JVal>,
) -> std::result::Result<JVal, String> {
    // This path is used when a Java proxy callback acquires an otherwise idle
    // engine. Mark the interpreter current so a nested hook callback (e.g. a
    // script calling the function it just hooked) can re-enter it instead of
    // trying to lock the engine again.
    let _guard = InterpGuard::new(interp);
    let pointer = interp as *mut Interpreter;
    unsafe { dispatch_on(pointer, handle, method, args) }
}

/// Hand a callback to the interpreter thread via the callback queue.
pub fn queue_dispatch(
    handle: i64,
    method: &str,
    args: Vec<JVal>,
) -> std::result::Result<JVal, String> {
    forward_to_owner(handle, method, args)
}

unsafe fn dispatch_on(
    pointer: *mut Interpreter,
    handle: i64,
    method: &str,
    args: Vec<JVal>,
) -> std::result::Result<JVal, String> {
    let target = {
        let table = proxies()
            .lock()
            .map_err(|_| "proxy table poisoned".to_string())?;
        table.get(&handle).map(|entry| entry.0.clone())
    }
    .ok_or_else(|| format!("Unknown proxy handle {}", handle))?;
    let interp = &mut *pointer;
    let values: Vec<Value> = args.into_iter().map(crate::java::from_jval).collect();
    let result = match target {
        ProxyTarget::Lambda(lambda) => {
            let previous_file = interp.source_file.clone();
            let previous_text = interp.source_text.clone();
            if lambda.source_file.is_some() {
                interp.source_file = lambda.source_file.clone();
            }
            if lambda.source_text.is_some() {
                interp.source_text = lambda.source_text.clone();
            }
            if let Some(body) = &lambda.method.body {
                interp.current_span = Some(body.span);
            }
            let result = interp.invoke_scripted(&lambda.method, None, values);
            interp.source_file = previous_file;
            interp.source_text = previous_text;
            result
        }
        ProxyTarget::Scripted(object) => {
            let ns = interp.global.clone();
            interp.call_method_on(&object, method, values, &ns)
        }
        ProxyTarget::MethodRef(method_ref) => {
            let ns = interp.global.clone();
            interp.invoke_method_ref(&method_ref, values, &ns)
        }
        ProxyTarget::Namespace(ns) => {
            let target_ns = ns.clone();
            let argc = values.len();
            let declared = {
                let namespace = target_ns.borrow();
                namespace.methods.get(method).and_then(|methods| {
                    methods
                        .iter()
                        .find(|m| {
                            m.params.len() == argc || crate::value::has_varargs(&m.params, argc)
                        })
                        .cloned()
                })
            };
            if let Some(method_def) = declared {
                interp.invoke_scripted(&method_def, None, values)
            } else {
                match crate::namespace::Namespace::lookup_value(&target_ns, method) {
                    Some(Value::Method(method_def)) => {
                        interp.invoke_scripted(&method_def, None, values)
                    }
                    Some(Value::Lambda(lambda)) => {
                        interp.invoke_scripted(&lambda.method, None, values)
                    }
                    Some(Value::MethodRef(method_ref)) => {
                        interp.invoke_method_ref(&method_ref, values, &target_ns)
                    }
                    _ => Err(crate::error::Error::eval(format!(
                        "No method {}() in the script namespace",
                        method
                    ))),
                }
            }
        }
    };
    match result {
        Ok(value) => crate::java::to_jval(interp, &value).map_err(|e| {
            if std::env::var("BSH_RS_DEBUG").is_ok() {
                eprintln!("[proxy] {} -> {}", method, e);
            }
            e.to_string()
        }),
        Err(e) => {
            if std::env::var("BSH_RS_DEBUG").is_ok() {
                eprintln!("[proxy] {} failed: {}", method, e);
            }
            match e {
                crate::error::Error::Eval(message) => Err(interp.decoration(&message)),
                other => Err(other.to_string()),
            }
        }
    }
}

/// Run pending cross-thread callbacks on the interpreter thread.
pub fn pump(interp: &mut Interpreter) -> usize {
    let pointer = interp as *mut Interpreter;
    let mut processed = 0;
    loop {
        let request = match interp.callback_queue.pop() {
            Some(r) => r,
            None => break,
        };
        // SAFETY: we are on the interpreter thread; the queue entry outlives.
        let result = unsafe { dispatch_on(pointer, request.handle, &request.method, request.args) };
        let _ = request.reply.send(result);
        processed += 1;
    }
    processed
}

/// Register a scripted instance so generated Java overrides can call back.
pub fn register_instance(interp: &mut Interpreter, target: ProxyTarget) -> i64 {
    let handle = NEXT_HANDLE.fetch_add(1, Ordering::Relaxed);
    if let Ok(mut table) = proxies().lock() {
        table.insert(handle, SendProxyTarget(target));
    }
    if let Ok(mut map) = queues().lock() {
        map.insert(handle, interp.callback_queue.clone());
    }
    record_owner(handle);
    handle
}

pub fn make_proxy(
    interp: &mut Interpreter,
    interfaces: &[String],
    target: ProxyTarget,
) -> Result<Value> {
    let jvm = interp
        .jvm()
        .ok_or_else(|| Error::eval("No JVM available for a Java interface proxy"))?;
    let handle = NEXT_HANDLE.fetch_add(1, Ordering::Relaxed);
    if let Ok(mut table) = proxies().lock() {
        table.insert(handle, SendProxyTarget(target));
    }
    if let Ok(mut map) = queues().lock() {
        map.insert(handle, interp.callback_queue.clone());
    }
    record_owner(handle);
    match jvm.proxy(interp.runtime_id, interfaces, handle) {
        Ok(JVal::Obj(obj)) => Ok(Value::Java(obj)),
        Ok(_) => {
            if let Ok(mut table) = proxies().lock() {
                table.remove(&handle);
            }
            Err(Error::eval("Proxy creation returned null"))
        }
        Err(e) => {
            if let Ok(mut table) = proxies().lock() {
                table.remove(&handle);
            }
            Err(crate::java::error(e))
        }
    }
}
