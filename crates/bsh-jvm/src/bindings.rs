//! Minimal hand written JNI bindings (no external crates).
//!
//! Only the entry points needed by the interpreter are declared. JNI function
//! tables are stable across JDK versions, so the indices used here are fixed.

#![allow(non_snake_case, dead_code)]

use std::ffi::c_void;
use std::os::raw::c_char;
use std::ptr;

pub type JObject = *mut c_void;
pub type JClass = *mut c_void;
pub type JMethodId = *mut c_void;
pub type JString = *mut c_void;
pub type JArray = *mut c_void;
pub type JInt = i32;
pub type JLong = i64;
pub type JChar = u16;
pub type JDouble = f64;
pub type JFloat = f32;
pub type JByte = i8;
pub type JShort = i16;
pub type JBoolean = u8;

pub const JNI_VERSION_1_6: JInt = 0x0001_0006;
pub const JNI_OK: JInt = 0;

#[repr(C)]
pub union JValue {
    pub z: JBoolean,
    pub b: JByte,
    pub c: JChar,
    pub s: JShort,
    pub i: JInt,
    pub j: JLong,
    pub f: JFloat,
    pub d: JDouble,
    pub l: JObject,
}

impl Clone for JValue {
    fn clone(&self) -> Self {
        unsafe { JValue { j: self.j } }
    }
}
impl Copy for JValue {}
impl Default for JValue {
    fn default() -> Self {
        JValue { j: 0 }
    }
}

#[repr(C)]
pub struct JniEnvRaw {
    pub functions: *const *const c_void,
}

#[repr(C)]
pub struct JavaVmRaw {
    pub functions: *const *const c_void,
}

pub type Env = *mut JniEnvRaw;
pub type Vm = *mut JavaVmRaw;

#[repr(C)]
pub struct JavaVmOption {
    pub option_string: *const c_char,
    pub extra_info: *mut c_void,
}

#[repr(C)]
pub struct JavaVmInitArgs {
    pub version: JInt,
    pub n_options: JInt,
    pub options: *mut JavaVmOption,
    pub ignore_unrecognized: JBoolean,
}

#[cfg(unix)]
extern "C" {
    fn dlopen(filename: *const c_char, flag: JInt) -> *mut c_void;
    fn dlsym(handle: *mut c_void, symbol: *const c_char) -> *mut c_void;
}

#[cfg(unix)]
const RTLD_NOW: JInt = 2;

#[cfg(windows)]
#[link(name = "kernel32")]
extern "system" {
    fn LoadLibraryA(name: *const c_char) -> *mut c_void;
    fn GetProcAddress(module: *mut c_void, name: *const c_char) -> *mut c_void;
}

/// Loads a dynamic library: dlopen on Unix, LoadLibraryA on Windows.
pub fn load_library(path: &str) -> Result<*mut c_void, String> {
    let c = std::ffi::CString::new(path).map_err(|e| e.to_string())?;
    #[cfg(unix)]
    let handle = unsafe { dlopen(c.as_ptr(), RTLD_NOW) };
    #[cfg(windows)]
    let handle = unsafe { LoadLibraryA(c.as_ptr()) };
    if handle.is_null() {
        return Err(format!("cannot load {}", path));
    }
    Ok(handle)
}

/// Resolves a symbol in a library returned by load_library.
pub fn lookup_symbol(handle: *mut c_void, name: &str) -> Result<*mut c_void, String> {
    let c = std::ffi::CString::new(name).map_err(|e| e.to_string())?;
    #[cfg(unix)]
    let sym = unsafe { dlsym(handle, c.as_ptr()) };
    #[cfg(windows)]
    let sym = unsafe { GetProcAddress(handle, c.as_ptr()) };
    if sym.is_null() {
        return Err(format!("cannot find symbol {}", name));
    }
    Ok(sym)
}

pub type CreateJavaVmFn =
    unsafe extern "system" fn(pvm: *mut Vm, penv: *mut Env, args: *mut JavaVmInitArgs) -> JInt;

unsafe fn env_fn(env: Env, index: usize) -> *const c_void {
    let table = (*env).functions;
    *table.add(index)
}

unsafe fn vm_fn(vm: Vm, index: usize) -> *const c_void {
    let table = (*vm).functions;
    *table.add(index)
}

// --------------------------------------------------------------- env calls

pub unsafe fn get_version(env: Env) -> JInt {
    let f: unsafe extern "system" fn(Env) -> JInt = std::mem::transmute(env_fn(env, 4));
    f(env)
}

pub unsafe fn find_class(env: Env, name: &str) -> JClass {
    let f: unsafe extern "system" fn(Env, *const c_char) -> JClass =
        std::mem::transmute(env_fn(env, 6));
    let c = std::ffi::CString::new(name).unwrap();
    f(env, c.as_ptr())
}

pub unsafe fn exception_occurred(env: Env) -> JObject {
    let f: unsafe extern "system" fn(Env) -> JObject = std::mem::transmute(env_fn(env, 15));
    f(env)
}

pub unsafe fn exception_describe(env: Env) {
    let f: unsafe extern "system" fn(Env) = std::mem::transmute(env_fn(env, 16));
    f(env)
}

pub unsafe fn exception_clear(env: Env) {
    let f: unsafe extern "system" fn(Env) = std::mem::transmute(env_fn(env, 17));
    f(env)
}

pub unsafe fn exception_check(env: Env) -> bool {
    let f: unsafe extern "system" fn(Env) -> JBoolean = std::mem::transmute(env_fn(env, 228));
    f(env) != 0
}

pub unsafe fn push_local_frame(env: Env, capacity: JInt) -> JInt {
    let f: unsafe extern "system" fn(Env, JInt) -> JInt = std::mem::transmute(env_fn(env, 19));
    f(env, capacity)
}

pub unsafe fn pop_local_frame(env: Env, result: JObject) -> JObject {
    let f: unsafe extern "system" fn(Env, JObject) -> JObject =
        std::mem::transmute(env_fn(env, 20));
    f(env, result)
}

pub unsafe fn new_global_ref(env: Env, obj: JObject) -> JObject {
    let f: unsafe extern "system" fn(Env, JObject) -> JObject =
        std::mem::transmute(env_fn(env, 21));
    f(env, obj)
}

pub unsafe fn delete_global_ref(env: Env, obj: JObject) {
    let f: unsafe extern "system" fn(Env, JObject) = std::mem::transmute(env_fn(env, 22));
    f(env, obj)
}

pub unsafe fn delete_local_ref(env: Env, obj: JObject) {
    let f: unsafe extern "system" fn(Env, JObject) = std::mem::transmute(env_fn(env, 23));
    f(env, obj)
}

pub unsafe fn is_same_object(env: Env, a: JObject, b: JObject) -> bool {
    let f: unsafe extern "system" fn(Env, JObject, JObject) -> JBoolean =
        std::mem::transmute(env_fn(env, 24));
    f(env, a, b) != 0
}

pub unsafe fn get_object_class(env: Env, obj: JObject) -> JClass {
    let f: unsafe extern "system" fn(Env, JObject) -> JClass = std::mem::transmute(env_fn(env, 31));
    f(env, obj)
}

pub unsafe fn is_instance_of(env: Env, obj: JObject, class: JClass) -> bool {
    let f: unsafe extern "system" fn(Env, JObject, JClass) -> JBoolean =
        std::mem::transmute(env_fn(env, 32));
    f(env, obj, class) != 0
}

pub unsafe fn get_method_id(env: Env, class: JClass, name: &str, sig: &str) -> JMethodId {
    let f: unsafe extern "system" fn(Env, JClass, *const c_char, *const c_char) -> JMethodId =
        std::mem::transmute(env_fn(env, 33));
    let n = std::ffi::CString::new(name).unwrap();
    let s = std::ffi::CString::new(sig).unwrap();
    f(env, class, n.as_ptr(), s.as_ptr())
}

pub unsafe fn new_object_a(
    env: Env,
    class: JClass,
    ctor: JMethodId,
    args: *const JValue,
) -> JObject {
    let f: unsafe extern "system" fn(Env, JClass, JMethodId, *const JValue) -> JObject =
        std::mem::transmute(env_fn(env, 30));
    f(env, class, ctor, args)
}

macro_rules! call_variant {
    ($name:ident, $idx:expr, $ret:ty) => {
        pub unsafe fn $name(env: Env, obj: JObject, m: JMethodId, args: *const JValue) -> $ret {
            let f: unsafe extern "system" fn(Env, JObject, JMethodId, *const JValue) -> $ret =
                std::mem::transmute(env_fn(env, $idx));
            f(env, obj, m, args)
        }
    };
}

call_variant!(call_object_method_a, 36, JObject);
call_variant!(call_boolean_method_a, 39, JBoolean);
call_variant!(call_char_method_a, 45, JChar);
call_variant!(call_int_method_a, 51, JInt);
call_variant!(call_long_method_a, 54, JLong);
call_variant!(call_double_method_a, 60, JDouble);
call_variant!(call_void_method_a, 63, ());

pub unsafe fn get_static_method_id(env: Env, class: JClass, name: &str, sig: &str) -> JMethodId {
    let f: unsafe extern "system" fn(Env, JClass, *const c_char, *const c_char) -> JMethodId =
        std::mem::transmute(env_fn(env, 113));
    let n = std::ffi::CString::new(name).unwrap();
    let s = std::ffi::CString::new(sig).unwrap();
    f(env, class, n.as_ptr(), s.as_ptr())
}

macro_rules! static_call_variant {
    ($name:ident, $idx:expr, $ret:ty) => {
        pub unsafe fn $name(env: Env, class: JClass, m: JMethodId, args: *const JValue) -> $ret {
            let f: unsafe extern "system" fn(Env, JClass, JMethodId, *const JValue) -> $ret =
                std::mem::transmute(env_fn(env, $idx));
            f(env, class, m, args)
        }
    };
}

static_call_variant!(call_static_object_method_a, 116, JObject);
static_call_variant!(call_static_boolean_method_a, 119, JBoolean);
static_call_variant!(call_static_char_method_a, 125, JChar);
static_call_variant!(call_static_int_method_a, 131, JInt);
static_call_variant!(call_static_long_method_a, 134, JLong);
static_call_variant!(call_static_double_method_a, 140, JDouble);
static_call_variant!(call_static_void_method_a, 143, ());

pub unsafe fn new_string_utf(env: Env, s: &str) -> JString {
    let f: unsafe extern "system" fn(Env, *const c_char) -> JString =
        std::mem::transmute(env_fn(env, 167));
    let c = std::ffi::CString::new(s.replace('\u{0}', "")).unwrap();
    f(env, c.as_ptr())
}

pub unsafe fn get_string_utf_chars(env: Env, s: JString) -> *const c_char {
    let f: unsafe extern "system" fn(Env, JString, *mut JBoolean) -> *const c_char =
        std::mem::transmute(env_fn(env, 169));
    f(env, s, ptr::null_mut())
}

pub unsafe fn release_string_utf_chars(env: Env, s: JString, chars: *const c_char) {
    let f: unsafe extern "system" fn(Env, JString, *const c_char) =
        std::mem::transmute(env_fn(env, 170));
    f(env, s, chars);
}

pub unsafe fn get_array_length(env: Env, arr: JArray) -> JInt {
    let f: unsafe extern "system" fn(Env, JArray) -> JInt = std::mem::transmute(env_fn(env, 171));
    f(env, arr)
}

pub unsafe fn new_object_array(
    env: Env,
    len: JInt,
    element_class: JClass,
    initial: JObject,
) -> JArray {
    let f: unsafe extern "system" fn(Env, JInt, JClass, JObject) -> JArray =
        std::mem::transmute(env_fn(env, 172));
    f(env, len, element_class, initial)
}

pub unsafe fn get_object_array_element(env: Env, arr: JArray, i: JInt) -> JObject {
    let f: unsafe extern "system" fn(Env, JArray, JInt) -> JObject =
        std::mem::transmute(env_fn(env, 173));
    f(env, arr, i)
}

pub unsafe fn set_object_array_element(env: Env, arr: JArray, i: JInt, v: JObject) {
    let f: unsafe extern "system" fn(Env, JArray, JInt, JObject) =
        std::mem::transmute(env_fn(env, 174));
    f(env, arr, i, v);
}

// ---------------------------------------------------------------- vm calls

pub unsafe fn get_env(vm: Vm, version: JInt) -> Result<Env, JInt> {
    let f: unsafe extern "system" fn(Vm, *mut Env, JInt) -> JInt =
        std::mem::transmute(vm_fn(vm, 6));
    let mut env: Env = ptr::null_mut();
    let rc = f(vm, &mut env, version);
    if rc == JNI_OK {
        Ok(env)
    } else {
        Err(rc)
    }
}

pub unsafe fn attach_current_thread(vm: Vm) -> Result<Env, JInt> {
    let f: unsafe extern "system" fn(Vm, *mut Env, *mut c_void) -> JInt =
        std::mem::transmute(vm_fn(vm, 4));
    let mut env: Env = ptr::null_mut();
    let rc = f(vm, &mut env, ptr::null_mut());
    if rc == JNI_OK {
        Ok(env)
    } else {
        Err(rc)
    }
}

#[repr(C)]
pub struct JNINativeMethod {
    pub name: *const c_char,
    pub signature: *const c_char,
    pub fn_ptr: *mut c_void,
}

pub unsafe fn register_natives(
    env: Env,
    class: JClass,
    methods: *const JNINativeMethod,
    count: JInt,
) -> JInt {
    let f: unsafe extern "system" fn(Env, JClass, *const JNINativeMethod, JInt) -> JInt =
        std::mem::transmute(env_fn(env, 215));
    f(env, class, methods, count)
}

/// ThrowNew takes a modified-UTF-8 C string, not a jstring.
pub unsafe fn throw_new(env: Env, class: JClass, message: *const c_char) -> JInt {
    let f: unsafe extern "system" fn(Env, JClass, *const c_char) -> JInt =
        std::mem::transmute(env_fn(env, 14));
    f(env, class, message)
}

pub unsafe fn new_local_ref(env: Env, obj: JObject) -> JObject {
    let f: unsafe extern "system" fn(Env, JObject) -> JObject =
        std::mem::transmute(env_fn(env, 25));
    f(env, obj)
}

pub type JByteArray = *mut c_void;

pub unsafe fn new_byte_array(env: Env, len: JInt) -> JByteArray {
    let f: unsafe extern "system" fn(Env, JInt) -> JByteArray =
        std::mem::transmute(env_fn(env, 176));
    f(env, len)
}

pub unsafe fn set_byte_array_region(
    env: Env,
    arr: JByteArray,
    start: JInt,
    len: JInt,
    buf: *const JByte,
) {
    let f: unsafe extern "system" fn(Env, JByteArray, JInt, JInt, *const JByte) =
        std::mem::transmute(env_fn(env, 208));
    f(env, arr, start, len, buf)
}
