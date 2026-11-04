//! JNI entry points for the Rust BeanShell interpreter.
//!
//! Each bsh.Interpreter instance owns one engine (global namespace) here, so
//! multiple plugins do not share variables or script methods.

use bsh_runtime::Interpreter;
use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::ffi::{c_char, c_void};
use std::io::Write;
use std::ptr;
use std::rc::Rc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, MutexGuard, OnceLock};

pub type JObject = *mut c_void;
pub type JString = *mut c_void;
pub type JArray = *mut c_void;
pub type JClass = *mut c_void;
pub type JLong = i64;
pub type JInt = i32;
pub type JBoolean = u8;

const JNI_VERSION_1_6: JInt = 0x0001_0006;

#[repr(C)]
pub struct JniEnvRaw {
    functions: *const *const c_void,
}

pub type Env = *mut JniEnvRaw;

unsafe fn env_fn(env: Env, index: usize) -> *const c_void {
    let table = (*env).functions;
    *table.add(index)
}

unsafe fn get_string_chars(env: Env, s: JString) -> *const u16 {
    let f: unsafe extern "system" fn(Env, JString, *mut u8) -> *const u16 =
        std::mem::transmute(env_fn(env, 165));
    f(env, s, ptr::null_mut())
}

unsafe fn release_string_chars(env: Env, s: JString, chars: *const u16) {
    let f: unsafe extern "system" fn(Env, JString, *const u16) =
        std::mem::transmute(env_fn(env, 166));
    f(env, s, chars);
}

unsafe fn get_string_length(env: Env, s: JString) -> i32 {
    let f: unsafe extern "system" fn(Env, JString) -> i32 = std::mem::transmute(env_fn(env, 164));
    f(env, s)
}

unsafe fn new_string(env: Env, chars: *const u16, len: i32) -> JString {
    let f: unsafe extern "system" fn(Env, *const u16, i32) -> JString =
        std::mem::transmute(env_fn(env, 163));
    f(env, chars, len)
}

unsafe fn get_array_length(env: Env, array: JArray) -> i32 {
    let f: unsafe extern "system" fn(Env, JArray) -> i32 = std::mem::transmute(env_fn(env, 171));
    f(env, array)
}

unsafe fn get_object_array_element(env: Env, array: JArray, index: i32) -> JObject {
    let f: unsafe extern "system" fn(Env, JArray, i32) -> JObject =
        std::mem::transmute(env_fn(env, 173));
    f(env, array, index)
}

fn read_java_string(env: Env, s: JString) -> String {
    unsafe {
        if s.is_null() {
            return String::new();
        }
        let len = get_string_length(env, s);
        let chars = get_string_chars(env, s);
        if chars.is_null() {
            return String::new();
        }
        let slice = std::slice::from_raw_parts(chars, len as usize);
        let text = String::from_utf16_lossy(slice);
        release_string_chars(env, s, chars);
        text
    }
}

fn to_java_string(env: Env, text: &str) -> JString {
    let mut units: Vec<u16> = text.encode_utf16().collect();
    unsafe { new_string(env, units.as_mut_ptr(), units.len() as i32) }
}

fn log(message: &str) {
    #[cfg(target_os = "android")]
    unsafe {
        extern "C" {
            fn __android_log_write(prio: i32, tag: *const c_char, text: *const c_char) -> i32;
        }
        let tag = std::ffi::CString::new("bsh-rs").unwrap();
        let text = std::ffi::CString::new(message.replace('\u{0}', "")).unwrap();
        __android_log_write(4, tag.as_ptr(), text.as_ptr());
    }
    #[cfg(not(target_os = "android"))]
    {
        eprintln!("[bsh-rs] {}", message);
    }
}

fn throw_runtime(env: Env, message: &str) {
    unsafe {
        let find_class: unsafe extern "system" fn(Env, *const c_char) -> JClass =
            std::mem::transmute(env_fn(env, 6));
        let class = find_class(
            env,
            b"java/lang/RuntimeException\0".as_ptr() as *const c_char,
        );
        if class.is_null() {
            return;
        }
        let text = std::ffi::CString::new(message.replace('\u{0}', "")).unwrap();
        let throw_new: unsafe extern "system" fn(Env, JClass, *const c_char) -> JInt =
            std::mem::transmute(env_fn(env, 14));
        throw_new(env, class, text.as_ptr());
    }
}

struct SharedSink {
    out: Rc<RefCell<Vec<u8>>>,
    at_line_start: bool,
}

/// Local time prefix for script output: [HH:MM:SS.mmm].
#[cfg(target_os = "android")]
fn local_time_prefix() -> String {
    #[repr(C)]
    struct Tm {
        tm_sec: i32,
        tm_min: i32,
        tm_hour: i32,
        tm_mday: i32,
        tm_mon: i32,
        tm_year: i32,
        tm_wday: i32,
        tm_yday: i32,
        tm_isdst: i32,
        tm_gmtoff: i64,
        tm_zone: *const std::os::raw::c_char,
    }
    extern "C" {
        fn localtime_r(timer: *const i64, result: *mut Tm) -> *mut Tm;
    }
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default();
    let seconds = now.as_secs() as i64;
    let millis = now.subsec_millis();
    unsafe {
        let mut tm: Tm = std::mem::zeroed();
        if localtime_r(&seconds, &mut tm).is_null() {
            return String::new();
        }
        format!(
            "[{:02}:{:02}:{:02}.{:03}] ",
            tm.tm_hour, tm.tm_min, tm.tm_sec, millis
        )
    }
}

#[cfg(not(target_os = "android"))]
fn local_time_prefix() -> String {
    String::new()
}

impl Write for SharedSink {
    fn write(&mut self, data: &[u8]) -> std::io::Result<usize> {
        let mut out = self.out.borrow_mut();
        for &byte in data {
            if self.at_line_start {
                out.extend_from_slice(local_time_prefix().as_bytes());
                self.at_line_start = false;
            }
            out.push(byte);
            if byte == b'\n' {
                self.at_line_start = true;
            }
        }
        Ok(data.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

struct Runtime {
    #[allow(dead_code)]
    id: u64,
    interpreter: Interpreter,
    output: Rc<RefCell<Vec<u8>>>,
}

// Access is serialized by the engine mutex; Rc values never leave that thread.
unsafe impl Send for Runtime {}

static ENGINES: OnceLock<Mutex<HashMap<u64, Runtime>>> = OnceLock::new();
static NEXT_ENGINE: AtomicU64 = AtomicU64::new(1);

thread_local! {
    static CURRENT_INTERPRETER: Cell<*mut Interpreter> = Cell::new(ptr::null_mut());
    static CURRENT_SESSION: Cell<*mut Runtime> = Cell::new(ptr::null_mut());
    static CURRENT_ENGINE: Cell<u64> = Cell::new(0);
}

fn engines() -> &'static Mutex<HashMap<u64, Runtime>> {
    ENGINES.get_or_init(|| Mutex::new(HashMap::new()))
}

fn lock_engines() -> MutexGuard<'static, HashMap<u64, Runtime>> {
    match engines().lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    }
}

fn new_runtime(id: u64) -> Runtime {
    ensure_callback_handler();
    let output = Rc::new(RefCell::new(Vec::new()));
    let sink = SharedSink {
        out: output.clone(),
        at_line_start: true,
    };
    let mut interpreter = Interpreter::new(Box::new(sink));
    interpreter.runtime_id = bsh_jvm::RuntimeId(id);
    Runtime {
        id,
        interpreter,
        output,
    }
}

/// Proxy callbacks may arrive on app threads: run them synchronously under the
/// owning engine lock, fall back to the callback queue while an eval holds it.
fn ensure_callback_handler() {
    bsh_jvm::set_proxy_handler(Box::new(|handle, method, args| {
        let current = CURRENT_INTERPRETER.with(|cell| cell.get());
        if !current.is_null() {
            let interpreter = unsafe { &mut *current };
            return bsh_runtime::proxy::dispatch_current(interpreter, handle, method, args);
        }
        if let Some(owner) = bsh_runtime::proxy::owner_of(handle) {
            let mut engines = match engines().try_lock() {
                Ok(guard) => guard,
                Err(_) => return bsh_runtime::proxy::queue_dispatch(handle, method, args),
            };
            if !engines.contains_key(&owner) {
                engines.insert(owner, new_runtime(owner));
            }
            let session = engines.get_mut(&owner).unwrap();
            // This callback path bypasses with_engine; mark the interpreter
            // current for nested callbacks (a hook callback calling a hooked
            // function) so they do not try to lock the engine again.
            let pointer = &mut session.interpreter as *mut Interpreter;
            let previous = CURRENT_INTERPRETER.with(|cell| cell.replace(pointer));
            let result = bsh_runtime::proxy::dispatch_current(
                unsafe { &mut *pointer },
                handle,
                method,
                args,
            );
            CURRENT_INTERPRETER.with(|cell| cell.set(previous));
            return result;
        }
        bsh_runtime::proxy::queue_dispatch(handle, method, args)
    }));
}

fn with_engine<T>(engine: u64, f: impl FnOnce(&mut Runtime) -> T) -> T {
    // Nested calls happen when a script host API calls interpreter.source()
    // (loadJava). The outer call already holds the engine mutex; reuse its
    // session instead of deadlocking on a non-reentrant Mutex.
    let nested = CURRENT_SESSION.with(|cell| cell.get());
    if !nested.is_null() && CURRENT_ENGINE.with(|cell| cell.get()) == engine {
        return f(unsafe { &mut *nested });
    }

    let mut engines = lock_engines();
    if !engines.contains_key(&engine) {
        engines.insert(engine, new_runtime(engine));
    }
    let session = engines.get_mut(&engine).unwrap();
    let interpreter_pointer = &mut session.interpreter as *mut Interpreter;
    let session_pointer = session as *mut Runtime;
    let previous_pointer = CURRENT_INTERPRETER.with(|cell| cell.replace(interpreter_pointer));
    let previous_session = CURRENT_SESSION.with(|cell| cell.replace(session_pointer));
    let previous_engine = CURRENT_ENGINE.with(|cell| cell.replace(engine));
    bsh_runtime::proxy::set_current_owner(engine);
    let result = f(session);
    CURRENT_ENGINE.with(|cell| cell.set(previous_engine));
    CURRENT_SESSION.with(|cell| cell.set(previous_session));
    CURRENT_INTERPRETER.with(|cell| cell.set(previous_pointer));
    result
}

fn eval_session(
    engine: u64,
    source: &str,
    file: Option<&str>,
) -> (String, Result<bsh_runtime::value::Value, String>) {
    with_engine(engine, |session| {
        session.output.borrow_mut().clear();
        session.interpreter.async_threads.clear();
        let result = match file {
            Some(path) => session.interpreter.eval_file(path),
            None => session.interpreter.eval_source(source),
        };
        session.interpreter.await_started_threads(2000);
        // A hook may have queued a main-thread callback while the engine was
        // loading; drain it before releasing the engine.
        session.interpreter.settle_callbacks(2, 100);
        session.interpreter.drain_java_errors();
        let text = String::from_utf8_lossy(&session.output.borrow()).to_string();
        if !text.is_empty() {
            log(text.trim_end());
        }
        (text, result.map_err(|error| format!("{}", error)))
    })
}

fn export_value(engine: u64, env: Env, value: &bsh_runtime::value::Value) -> JObject {
    let Some(jvm) = bsh_jvm::global() else {
        return ptr::null_mut();
    };
    let jval = with_engine(engine, |session| {
        bsh_runtime::java::to_jval(&mut session.interpreter, value)
    });
    match jval {
        Ok(jval) => jvm
            .export_object_raw(env as *mut c_void, &jval)
            .unwrap_or(ptr::null_mut()),
        Err(error) => {
            throw_runtime(env, &format!("{}", error));
            ptr::null_mut()
        }
    }
}

fn import_value(env: Env, object: JObject) -> Result<bsh_runtime::value::Value, String> {
    if object.is_null() {
        return Ok(bsh_runtime::value::Value::Null);
    }
    let jvm = bsh_jvm::global().ok_or_else(|| "No JVM available".to_string())?;
    let jval = jvm
        .import_object_raw(env as *mut c_void, object as *mut c_void)
        .map_err(|error| format!("{}", error))?;
    Ok(bsh_runtime::java::from_jval(jval))
}

#[no_mangle]
pub unsafe extern "system" fn JNI_OnLoad(vm: *mut c_void, _reserved: *mut c_void) -> JInt {
    match bsh_jvm::install_external(vm) {
        Ok(_) => {}
        Err(error) => log(&format!("JVM attach failed: {}", error)),
    }
    JNI_VERSION_1_6
}

#[no_mangle]
pub unsafe extern "system" fn Java_me_yun_bsh_rust_Native_createEngine(
    _env: Env,
    _this: JClass,
) -> JLong {
    let engine = NEXT_ENGINE.fetch_add(1, Ordering::Relaxed);
    lock_engines().insert(engine, new_runtime(engine));
    engine as i64
}

#[no_mangle]
pub unsafe extern "system" fn Java_me_yun_bsh_rust_Native_destroyEngine(
    _env: Env,
    _this: JClass,
    engine: JLong,
) {
    let id = engine as u64;
    bsh_runtime::proxy::release_owner(id);
    lock_engines().remove(&id);
}

#[no_mangle]
pub unsafe extern "system" fn Java_me_yun_bsh_rust_Native_eval(
    env: Env,
    _this: JClass,
    engine: JLong,
    source: JString,
) -> JString {
    let code = read_java_string(env, source);
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        eval_session(engine as u64, &code, None)
    }))
    .unwrap_or_else(|_| ("".to_string(), Err("interpreter panicked".to_string())));
    match result {
        (text, Ok(_)) => to_java_string(env, &text),
        (text, Err(error)) => {
            let mut out = text;
            if !out.is_empty() && !out.ends_with('\n') {
                out.push('\n');
            }
            out.push_str(&error);
            out.push('\n');
            to_java_string(env, &out)
        }
    }
}

#[no_mangle]
pub unsafe extern "system" fn Java_me_yun_bsh_rust_Native_evalValue(
    env: Env,
    _this: JClass,
    engine: JLong,
    source: JString,
) -> JObject {
    let code = read_java_string(env, source);
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        eval_session(engine as u64, &code, None)
    }));
    match result {
        Ok((_, Ok(value))) => export_value(engine as u64, env, &value),
        Ok((_, Err(error))) => {
            throw_runtime(env, &error);
            ptr::null_mut()
        }
        Err(_) => {
            throw_runtime(env, "interpreter panicked");
            ptr::null_mut()
        }
    }
}

#[no_mangle]
pub unsafe extern "system" fn Java_me_yun_bsh_rust_Native_evalFile(
    env: Env,
    _this: JClass,
    engine: JLong,
    path: JString,
) -> JObject {
    let file = read_java_string(env, path);
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        eval_session(engine as u64, "", Some(&file))
    }));
    match result {
        Ok((_, Ok(value))) => export_value(engine as u64, env, &value),
        Ok((_, Err(error))) => {
            throw_runtime(env, &error);
            ptr::null_mut()
        }
        Err(_) => {
            throw_runtime(env, "interpreter panicked");
            ptr::null_mut()
        }
    }
}

#[no_mangle]
pub unsafe extern "system" fn Java_me_yun_bsh_rust_Native_setVariable(
    env: Env,
    _this: JClass,
    engine: JLong,
    name: JString,
    value: JObject,
) {
    let key = read_java_string(env, name);
    let outcome =
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| -> Result<(), String> {
            let value = import_value(env, value)?;
            with_engine(engine as u64, |session| {
                session.interpreter.set_variable_value(&key, value)
            });
            Ok(())
        }));
    match outcome {
        Ok(Ok(())) => {}
        Ok(Err(error)) => throw_runtime(env, &error),
        Err(_) => throw_runtime(env, "interpreter panicked"),
    }
}

#[no_mangle]
pub unsafe extern "system" fn Java_me_yun_bsh_rust_Native_getVariable(
    env: Env,
    _this: JClass,
    engine: JLong,
    name: JString,
) -> JObject {
    let key = read_java_string(env, name);
    let value = with_engine(engine as u64, |session| {
        session.interpreter.get_variable_value(&key)
    });
    match value {
        Some(value) => export_value(engine as u64, env, &value),
        None => ptr::null_mut(),
    }
}

#[no_mangle]
pub unsafe extern "system" fn Java_me_yun_bsh_rust_Native_importClass(
    env: Env,
    _this: JClass,
    engine: JLong,
    name: JString,
) {
    let class_name = read_java_string(env, name);
    with_engine(engine as u64, |session| {
        session.interpreter.add_import_name(&class_name)
    });
}

#[no_mangle]
pub unsafe extern "system" fn Java_me_yun_bsh_rust_Native_defineMethod(
    env: Env,
    _this: JClass,
    engine: JLong,
    name: JString,
    instance: JObject,
    class_name: JString,
    argc: JInt,
    varargs: JBoolean,
) {
    let method = read_java_string(env, name);
    let class_name = read_java_string(env, class_name);
    let instance = if instance.is_null() {
        None
    } else {
        match import_value(env, instance) {
            Ok(bsh_runtime::value::Value::Java(object)) => Some(object),
            _ => None,
        }
    };
    with_engine(engine as u64, |session| {
        session.interpreter.define_host_method(
            &method,
            argc as usize,
            &class_name,
            instance,
            varargs != 0,
        )
    });
}

#[no_mangle]
pub unsafe extern "system" fn Java_me_yun_bsh_rust_Native_scriptMethods(
    env: Env,
    _this: JClass,
    engine: JLong,
) -> JString {
    let methods = with_engine(engine as u64, |session| {
        session.interpreter.script_methods()
    });
    let text = methods
        .iter()
        .map(|(name, argc)| format!("{}|{}", name, argc))
        .collect::<Vec<_>>()
        .join("\n");
    to_java_string(env, &text)
}

#[no_mangle]
pub unsafe extern "system" fn Java_me_yun_bsh_rust_Native_invokeMethod(
    env: Env,
    _this: JClass,
    engine: JLong,
    name: JString,
    args: JArray,
) -> JObject {
    let method = read_java_string(env, name);
    let argc = if args.is_null() {
        0
    } else {
        get_array_length(env, args) as usize
    };
    let mut values = Vec::with_capacity(argc);
    for index in 0..argc {
        let argument = get_object_array_element(env, args, index as i32);
        match import_value(env, argument) {
            Ok(value) => values.push(value),
            Err(error) => {
                throw_runtime(env, &error);
                return ptr::null_mut();
            }
        }
    }
    let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        with_engine(engine as u64, |session| {
            let value = session.interpreter.invoke_script_method(&method, values);
            session.interpreter.settle_callbacks(2, 100);
            value
        })
    }));
    match outcome {
        Ok(Ok(value)) => export_value(engine as u64, env, &value),
        Ok(Err(error)) => {
            throw_runtime(env, &format!("{}", error));
            ptr::null_mut()
        }
        Err(_) => {
            throw_runtime(env, "interpreter panicked");
            ptr::null_mut()
        }
    }
}

#[no_mangle]
pub unsafe extern "system" fn Java_me_yun_bsh_rust_Native_reset(
    _env: Env,
    _this: JClass,
    engine: JLong,
) {
    bsh_runtime::proxy::release_owner(engine as u64);
    let mut engines = lock_engines();
    engines.insert(engine as u64, new_runtime(engine as u64));
}
