//! Embedded JVM access for the Rust BeanShell interpreter.
//!
//! The Rust side never talks to Java classes directly: it loads a tiny Java
//! helper (bshrs.Bridge) that performs reflection, overload selection and
//! boxing. This keeps the JNI surface small and stable.

mod bindings;
pub mod classfile;

use bindings::*;
use std::collections::HashMap;
use std::fmt;
use std::path::{Path, PathBuf};
use std::ptr;
use std::sync::{Arc, OnceLock, Weak};

// ---------------------------------------------------------------- bridge code

const BRIDGE_CLASS: &[u8] = include_bytes!("../java-classes/bshrs/Bridge.class");
const BRIDGE_HANDLER_CLASS: &[u8] = include_bytes!("../java-classes/bshrs/Bridge$Handler.class");
const BRIDGE_LOADER_CLASS: &[u8] =
    include_bytes!("../java-classes/bshrs/Bridge$ScriptLoader.class");
const NATIVE_CLASS: &[u8] = include_bytes!("../java-classes/bshrs/Native.class");

const BRIDGE_METHODS: &[(&str, &str, &str)] = &[
    (
        "box",
        "box",
        "(Ljava/lang/String;Ljava/lang/String;)Ljava/lang/Object;",
    ),
    ("tag", "tag", "(Ljava/lang/Object;)I"),
    ("asBool", "asBool", "(Ljava/lang/Object;)Z"),
    ("asChar", "asChar", "(Ljava/lang/Object;)C"),
    ("asLong", "asLong", "(Ljava/lang/Object;)J"),
    ("asDouble", "asDouble", "(Ljava/lang/Object;)D"),
    (
        "asString",
        "asString",
        "(Ljava/lang/Object;)Ljava/lang/String;",
    ),
    (
        "className",
        "className",
        "(Ljava/lang/Object;)Ljava/lang/String;",
    ),
    (
        "typeName",
        "typeName",
        "(Ljava/lang/Object;)Ljava/lang/String;",
    ),
    (
        "newInstance",
        "newInstance",
        "(Ljava/lang/String;[Ljava/lang/Object;)Ljava/lang/Object;",
    ),
    (
        "call",
        "call",
        "(Ljava/lang/Object;Ljava/lang/String;[Ljava/lang/Object;)Ljava/lang/Object;",
    ),
    (
        "callStatic",
        "callStatic",
        "(Ljava/lang/String;Ljava/lang/String;[Ljava/lang/Object;)Ljava/lang/Object;",
    ),
    (
        "get",
        "get",
        "(Ljava/lang/Object;Ljava/lang/String;)Ljava/lang/Object;",
    ),
    (
        "getStatic",
        "getStatic",
        "(Ljava/lang/String;Ljava/lang/String;)Ljava/lang/Object;",
    ),
    (
        "set",
        "set",
        "(Ljava/lang/Object;Ljava/lang/String;Ljava/lang/Object;)V",
    ),
    (
        "setStatic",
        "setStatic",
        "(Ljava/lang/String;Ljava/lang/String;Ljava/lang/Object;)V",
    ),
    ("length", "length", "(Ljava/lang/Object;)I"),
    ("index", "index", "(Ljava/lang/Object;I)Ljava/lang/Object;"),
    (
        "setIndex",
        "setIndex",
        "(Ljava/lang/Object;ILjava/lang/Object;)V",
    ),
    (
        "newArray",
        "newArray",
        "(Ljava/lang/String;I)Ljava/lang/Object;",
    ),
    (
        "isInstance",
        "isInstance",
        "(Ljava/lang/Object;Ljava/lang/String;)Z",
    ),
    (
        "classFor",
        "classFor",
        "(Ljava/lang/String;)Ljava/lang/Object;",
    ),
    ("classExists", "classExists", "(Ljava/lang/String;)Z"),
    ("isInterface", "isInterface", "(Ljava/lang/String;)Z"),
    ("proxy", "proxy", "([Ljava/lang/String;J)Ljava/lang/Object;"),
    (
        "methodParams",
        "methodParams",
        "(Ljava/lang/Object;Ljava/lang/String;Ljava/lang/String;IZ)[Ljava/lang/String;",
    ),
    (
        "ctorParams",
        "ctorParams",
        "(Ljava/lang/String;I)[Ljava/lang/String;",
    ),
    (
        "defineScriptClass",
        "defineScriptClass",
        "(Ljava/lang/String;[B)Ljava/lang/Class;",
    ),
    (
        "newScriptInstance",
        "newScriptInstance",
        "(Ljava/lang/Class;J)Ljava/lang/Object;",
    ),
    ("drainErrors", "drainErrors", "()Ljava/lang/String;"),
    (
        "overridableMethods",
        "overridableMethods",
        "(Ljava/lang/String;)Ljava/lang/String;",
    ),
    (
        "classForEngine",
        "classForEngine",
        "(JLjava/lang/String;)Ljava/lang/Object;",
    ),
    (
        "classExistsEngine",
        "classExistsEngine",
        "(JLjava/lang/String;)Z",
    ),
    (
        "newInstanceEngine",
        "newInstanceEngine",
        "(JLjava/lang/String;[Ljava/lang/Object;)Ljava/lang/Object;",
    ),
    (
        "callStaticEngine",
        "callStaticEngine",
        "(JLjava/lang/String;Ljava/lang/String;[Ljava/lang/Object;)Ljava/lang/Object;",
    ),
    (
        "getStaticEngine",
        "getStaticEngine",
        "(JLjava/lang/String;Ljava/lang/String;)Ljava/lang/Object;",
    ),
    (
        "setStaticEngine",
        "setStaticEngine",
        "(JLjava/lang/String;Ljava/lang/String;Ljava/lang/Object;)V",
    ),
    (
        "newArrayEngine",
        "newArrayEngine",
        "(JLjava/lang/String;I)Ljava/lang/Object;",
    ),
    (
        "isInstanceEngine",
        "isInstanceEngine",
        "(JLjava/lang/Object;Ljava/lang/String;)Z",
    ),
    (
        "isInterfaceEngine",
        "isInterfaceEngine",
        "(JLjava/lang/String;)Z",
    ),
    (
        "proxyEngine",
        "proxyEngine",
        "(J[Ljava/lang/String;J)Ljava/lang/Object;",
    ),
    (
        "methodParamsEngine",
        "methodParamsEngine",
        "(JLjava/lang/Object;Ljava/lang/String;Ljava/lang/String;IZ)[Ljava/lang/String;",
    ),
    (
        "methodOverloadsEngine",
        "methodOverloadsEngine",
        "(JLjava/lang/Object;Ljava/lang/String;Ljava/lang/String;IZ)[[Ljava/lang/String;",
    ),
    (
        "samInfoEngine",
        "samInfoEngine",
        "(JLjava/lang/String;)[Ljava/lang/String;",
    ),
    (
        "ctorParamsEngine",
        "ctorParamsEngine",
        "(JLjava/lang/String;I)[Ljava/lang/String;",
    ),
];

// --------------------------------------------------------------------- types

/// Identifies the owning interpreter/plugin runtime. Phase 1 only plumbs the
/// id through the API; it is not used for lookup yet.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct RuntimeId(pub u64);

fn runtime_arg(runtime: RuntimeId) -> JValue {
    JValue {
        j: runtime.0 as i64,
    }
}

#[derive(Clone)]
pub enum JVal {
    Null,
    Bool(bool),
    Char(char),
    Byte(i8),
    Short(i16),
    Int(i32),
    Long(i64),
    Float(f32),
    Double(f64),
    Str(String),
    Obj(Arc<JObj>),
}

impl fmt::Debug for JVal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            JVal::Str(s) => write!(f, "Str({:?})", s),
            JVal::Obj(o) => write!(f, "Obj({})", o.class_name),
            other => write!(f, "{:?}", other),
        }
    }
}

pub struct JObj {
    obj: JObject,
    /// Fully qualified Java class name.
    pub class_name: String,
    jvm: Arc<Jvm>,
}

impl fmt::Debug for JObj {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "JObj({}@{:?})", self.class_name, self.obj)
    }
}

impl JObj {
    pub fn jvm(&self) -> &Arc<Jvm> {
        &self.jvm
    }

    pub fn class_name(&self) -> &str {
        &self.class_name
    }
}

// SAFETY: a JObj wraps a JNI global reference. JNI global references stay valid
// across threads and every JNI call attaches the calling thread first.
unsafe impl Send for JObj {}
unsafe impl Sync for JObj {}

impl Drop for JObj {
    fn drop(&mut self) {
        if self.obj.is_null() {
            return;
        }
        if let Ok(env) = self.jvm.env() {
            unsafe { delete_global_ref(env, self.obj) };
        }
    }
}

#[derive(Debug)]
pub enum JError {
    Api(String),
    Thrown(Arc<JObj>),
}

impl fmt::Display for JError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            JError::Api(m) => write!(f, "JNI error: {}", m),
            JError::Thrown(o) => write!(f, "Java exception: {}", o.class_name),
        }
    }
}

// ----------------------------------------------------------------------- Jvm

pub struct Jvm {
    vm: Vm,
    bridge: JClass,
    methods: HashMap<&'static str, JMethodId>,
    self_ref: OnceLock<Weak<Jvm>>,
    /// Whether the runtime can define Java bytecode (false on Android/ART).
    script_classes: bool,
}

// The raw JVM handles are thread safe; every call attaches the current thread.
unsafe impl Send for Jvm {}
unsafe impl Sync for Jvm {}

impl Jvm {
    fn arc(&self) -> Arc<Jvm> {
        self.self_ref
            .get()
            .and_then(|w| w.upgrade())
            .expect("Jvm self reference")
    }

    fn env(&self) -> Result<Env, JError> {
        match unsafe { get_env(self.vm, JNI_VERSION_1_6) } {
            Ok(env) => Ok(env),
            Err(_) => unsafe { attach_current_thread(self.vm) }
                .map_err(|rc| JError::Api(format!("cannot attach thread ({})", rc))),
        }
    }

    fn frame<T>(&self, f: impl FnOnce(Env) -> Result<T, JError>) -> Result<T, JError> {
        let env = self.env()?;
        unsafe {
            push_local_frame(env, 64);
        }
        let result = f(env);
        unsafe {
            pop_local_frame(env, ptr::null_mut());
        }
        result
    }

    fn check(&self, env: Env) -> Result<(), JError> {
        if !unsafe { exception_check(env) } {
            return Ok(());
        }
        let throwable = unsafe { exception_occurred(env) };
        let global = unsafe { new_global_ref(env, throwable) };
        unsafe {
            delete_local_ref(env, throwable);
            exception_clear(env);
        }
        let class_name = self
            .bridge_string(env, "className", &[JValue { l: global }])
            .unwrap_or_else(|_| "java.lang.Throwable".to_string());
        Err(JError::Thrown(Arc::new(JObj {
            obj: global,
            class_name,
            jvm: self.arc(),
        })))
    }

    fn method(&self, name: &str) -> JMethodId {
        *self.methods.get(name).expect("bridge method")
    }

    fn bridge_string(&self, env: Env, name: &str, args: &[JValue]) -> Result<String, JError> {
        let m = self.method(name);
        let s = unsafe { call_static_object_method_a(env, self.bridge, m, args.as_ptr()) };
        self.check(env)?;
        if s.is_null() {
            return Ok(String::new());
        }
        Ok(read_string(env, s))
    }

    fn bridge_int(&self, env: Env, name: &str, args: &[JValue]) -> Result<i32, JError> {
        let m = self.method(name);
        let v = unsafe { call_static_int_method_a(env, self.bridge, m, args.as_ptr()) };
        self.check(env)?;
        Ok(v)
    }

    fn bridge_bool(&self, env: Env, name: &str, args: &[JValue]) -> Result<bool, JError> {
        let m = self.method(name);
        let v = unsafe { call_static_boolean_method_a(env, self.bridge, m, args.as_ptr()) };
        self.check(env)?;
        Ok(v != 0)
    }

    fn bridge_long(&self, env: Env, name: &str, args: &[JValue]) -> Result<i64, JError> {
        let m = self.method(name);
        let v = unsafe { call_static_long_method_a(env, self.bridge, m, args.as_ptr()) };
        self.check(env)?;
        Ok(v)
    }

    fn bridge_double(&self, env: Env, name: &str, args: &[JValue]) -> Result<f64, JError> {
        let m = self.method(name);
        let v = unsafe { call_static_double_method_a(env, self.bridge, m, args.as_ptr()) };
        self.check(env)?;
        Ok(v)
    }

    fn bridge_char(&self, env: Env, name: &str, args: &[JValue]) -> Result<char, JError> {
        let m = self.method(name);
        let v = unsafe { call_static_char_method_a(env, self.bridge, m, args.as_ptr()) };
        self.check(env)?;
        Ok(char::from_u32(v as u32).unwrap_or('\u{0}'))
    }

    fn call_bridge_obj(&self, env: Env, name: &str, args: &[JValue]) -> Result<JObject, JError> {
        let m = self.method(name);
        let v = unsafe { call_static_object_method_a(env, self.bridge, m, args.as_ptr()) };
        self.check(env)?;
        Ok(v)
    }

    fn call_bridge_void(&self, env: Env, name: &str, args: &[JValue]) -> Result<(), JError> {
        let m = self.method(name);
        unsafe { call_static_void_method_a(env, self.bridge, m, args.as_ptr()) };
        self.check(env)
    }

    fn wrap(&self, env: Env, obj: JObject, class_name: Option<String>) -> Result<JVal, JError> {
        if obj.is_null() {
            return Ok(JVal::Null);
        }
        let name = match class_name {
            Some(n) => n,
            None => self
                .bridge_string(env, "className", &[JValue { l: obj }])
                .unwrap_or_else(|_| "java.lang.Object".to_string()),
        };
        let global = unsafe { new_global_ref(env, obj) };
        Ok(JVal::Obj(Arc::new(JObj {
            obj: global,
            class_name: name,
            jvm: self.arc(),
        })))
    }

    fn convert(&self, env: Env, obj: JObject) -> Result<JVal, JError> {
        if obj.is_null() {
            return Ok(JVal::Null);
        }
        let tag = self.bridge_int(env, "tag", &[JValue { l: obj }])?;
        Ok(match tag {
            1 => JVal::Bool(self.bridge_bool(env, "asBool", &[JValue { l: obj }])?),
            2 => JVal::Char(self.bridge_char(env, "asChar", &[JValue { l: obj }])?),
            3 => JVal::Byte(self.bridge_long(env, "asLong", &[JValue { l: obj }])? as i8),
            4 => JVal::Short(self.bridge_long(env, "asLong", &[JValue { l: obj }])? as i16),
            5 => JVal::Int(self.bridge_long(env, "asLong", &[JValue { l: obj }])? as i32),
            6 => JVal::Long(self.bridge_long(env, "asLong", &[JValue { l: obj }])?),
            7 => JVal::Float(self.bridge_double(env, "asDouble", &[JValue { l: obj }])? as f32),
            8 => JVal::Double(self.bridge_double(env, "asDouble", &[JValue { l: obj }])?),
            9 => JVal::Str(self.bridge_string(env, "asString", &[JValue { l: obj }])?),
            _ => self.wrap(env, obj, None)?,
        })
    }

    fn box_value(&self, env: Env, kind: &str, text: &str) -> Result<JObject, JError> {
        let k = new_string(env, kind);
        let t = new_string(env, text);
        let args = [JValue { l: k }, JValue { l: t }];
        self.call_bridge_obj(env, "box", &args)
    }

    fn to_java(&self, env: Env, v: &JVal) -> Result<JObject, JError> {
        Ok(match v {
            JVal::Null => ptr::null_mut(),
            JVal::Bool(b) => self.box_value(env, "boolean", if *b { "true" } else { "false" })?,
            JVal::Char(c) => self.box_value(env, "char", &c.to_string())?,
            JVal::Byte(b) => self.box_value(env, "byte", &b.to_string())?,
            JVal::Short(s) => self.box_value(env, "short", &s.to_string())?,
            JVal::Int(i) => self.box_value(env, "int", &i.to_string())?,
            JVal::Long(l) => self.box_value(env, "long", &l.to_string())?,
            JVal::Float(f) => self.box_value(env, "float", &f.to_string())?,
            JVal::Double(d) => self.box_value(env, "double", &d.to_string())?,
            JVal::Str(s) => new_string(env, s),
            JVal::Obj(o) => o.obj,
        })
    }

    fn make_args(&self, env: Env, args: &[JVal]) -> Result<JArray, JError> {
        let object_class = unsafe { find_class(env, "java/lang/Object") };
        let arr =
            unsafe { new_object_array(env, args.len() as i32, object_class, ptr::null_mut()) };
        for (i, a) in args.iter().enumerate() {
            let obj = self.to_java(env, a)?;
            unsafe { set_object_array_element(env, arr, i as i32, obj) };
        }
        Ok(arr)
    }

    // ------------------------------------------------------------ public API

    pub fn class_exists(&self, _runtime: RuntimeId, name: &str) -> bool {
        self.frame(|env| {
            let s = new_string(env, name);
            self.bridge_bool(
                env,
                "classExistsEngine",
                &[runtime_arg(_runtime), JValue { l: s }],
            )
        })
        .unwrap_or(false)
    }

    pub fn class_ref(&self, _runtime: RuntimeId, name: &str) -> Result<Arc<JObj>, JError> {
        self.frame(|env| {
            let s = new_string(env, name);
            let obj = self.call_bridge_obj(
                env,
                "classForEngine",
                &[runtime_arg(_runtime), JValue { l: s }],
            )?;
            match self.wrap(env, obj, Some(name.to_string()))? {
                JVal::Obj(o) => Ok(o),
                _ => Err(JError::Api("classFor returned null".to_string())),
            }
        })
    }

    pub fn new_instance(
        &self,
        _runtime: RuntimeId,
        class: &str,
        args: &[JVal],
    ) -> Result<JVal, JError> {
        self.frame(|env| {
            let cname = new_string(env, class);
            let arr = self.make_args(env, args)?;
            let obj = self.call_bridge_obj(
                env,
                "newInstanceEngine",
                &[
                    runtime_arg(_runtime),
                    JValue { l: cname },
                    JValue { l: arr },
                ],
            )?;
            self.convert(env, obj)
        })
    }

    pub fn call(&self, target: &JObj, name: &str, args: &[JVal]) -> Result<JVal, JError> {
        self.frame(|env| {
            let nm = new_string(env, name);
            let arr = self.make_args(env, args)?;
            let obj = self.call_bridge_obj(
                env,
                "call",
                &[
                    JValue { l: target.obj },
                    JValue { l: nm },
                    JValue { l: arr },
                ],
            )?;
            self.convert(env, obj)
        })
    }

    pub fn call_static(
        &self,
        _runtime: RuntimeId,
        class: &str,
        name: &str,
        args: &[JVal],
    ) -> Result<JVal, JError> {
        self.frame(|env| {
            let cn = new_string(env, class);
            let nm = new_string(env, name);
            let arr = self.make_args(env, args)?;
            let obj = self.call_bridge_obj(
                env,
                "callStaticEngine",
                &[
                    runtime_arg(_runtime),
                    JValue { l: cn },
                    JValue { l: nm },
                    JValue { l: arr },
                ],
            )?;
            self.convert(env, obj)
        })
    }

    pub fn get_field(&self, target: &JObj, name: &str) -> Result<JVal, JError> {
        self.frame(|env| {
            let nm = new_string(env, name);
            let obj =
                self.call_bridge_obj(env, "get", &[JValue { l: target.obj }, JValue { l: nm }])?;
            self.convert(env, obj)
        })
    }

    pub fn set_field(&self, target: &JObj, name: &str, value: &JVal) -> Result<(), JError> {
        self.frame(|env| {
            let nm = new_string(env, name);
            let val = self.to_java(env, value)?;
            self.call_bridge_void(
                env,
                "set",
                &[
                    JValue { l: target.obj },
                    JValue { l: nm },
                    JValue { l: val },
                ],
            )
        })
    }

    pub fn get_static(&self, _runtime: RuntimeId, class: &str, name: &str) -> Result<JVal, JError> {
        self.frame(|env| {
            let cn = new_string(env, class);
            let nm = new_string(env, name);
            let obj = self.call_bridge_obj(
                env,
                "getStaticEngine",
                &[runtime_arg(_runtime), JValue { l: cn }, JValue { l: nm }],
            )?;
            self.convert(env, obj)
        })
    }

    pub fn set_static(
        &self,
        _runtime: RuntimeId,
        class: &str,
        name: &str,
        value: &JVal,
    ) -> Result<(), JError> {
        self.frame(|env| {
            let cn = new_string(env, class);
            let nm = new_string(env, name);
            let val = self.to_java(env, value)?;
            self.call_bridge_void(
                env,
                "setStaticEngine",
                &[
                    runtime_arg(_runtime),
                    JValue { l: cn },
                    JValue { l: nm },
                    JValue { l: val },
                ],
            )
        })
    }

    pub fn array_len(&self, obj: &JObj) -> Result<usize, JError> {
        self.frame(|env| Ok(unsafe { get_array_length(env, obj.obj) } as usize))
    }

    pub fn array_get(&self, obj: &JObj, i: usize) -> Result<JVal, JError> {
        self.frame(|env| {
            let v = self.call_bridge_obj(
                env,
                "index",
                &[JValue { l: obj.obj }, JValue { i: i as i32 }],
            )?;
            self.convert(env, v)
        })
    }

    pub fn array_set(&self, obj: &JObj, i: usize, v: &JVal) -> Result<(), JError> {
        self.frame(|env| {
            let val = self.to_java(env, v)?;
            self.call_bridge_void(
                env,
                "setIndex",
                &[
                    JValue { l: obj.obj },
                    JValue { i: i as i32 },
                    JValue { l: val },
                ],
            )
        })
    }

    pub fn new_array(
        &self,
        _runtime: RuntimeId,
        component: &str,
        len: usize,
    ) -> Result<JVal, JError> {
        self.frame(|env| {
            let cn = new_string(env, component);
            let v = self.call_bridge_obj(
                env,
                "newArrayEngine",
                &[
                    runtime_arg(_runtime),
                    JValue { l: cn },
                    JValue { i: len as i32 },
                ],
            )?;
            self.wrap(env, v, None)
        })
    }

    pub fn is_instance(&self, _runtime: RuntimeId, obj: &JObj, class: &str) -> bool {
        self.frame(|env| {
            let cn = new_string(env, class);
            self.bridge_bool(
                env,
                "isInstanceEngine",
                &[
                    runtime_arg(_runtime),
                    JValue { l: obj.obj },
                    JValue { l: cn },
                ],
            )
        })
        .unwrap_or(false)
    }

    pub fn class_of(&self, obj: &JObj) -> Result<Arc<JObj>, JError> {
        self.frame(|env| {
            let class = unsafe { get_object_class(env, obj.obj) };
            let global = unsafe { new_global_ref(env, class) };
            Ok(Arc::new(JObj {
                obj: global,
                class_name: "java.lang.Class".to_string(),
                jvm: self.arc(),
            }))
        })
    }

    pub fn same_object(&self, a: &JObj, b: &JObj) -> bool {
        match self.env() {
            Ok(env) => unsafe { is_same_object(env, a.obj, b.obj) },
            Err(_) => false,
        }
    }

    pub fn to_java_string(&self, obj: &JObj) -> String {
        match self.call(obj, "toString", &[]) {
            Ok(JVal::Str(s)) => s,
            Ok(JVal::Null) => "null".to_string(),
            Ok(other) => format!("{:?}", other),
            Err(_) => obj.class_name.clone(),
        }
    }

    pub fn type_name(&self, obj: &JObj) -> String {
        self.frame(|env| {
            let s = self.call_bridge_obj(env, "typeName", &[JValue { l: obj.obj }])?;
            Ok(read_string(env, s))
        })
        .unwrap_or_else(|_| obj.class_name.clone())
    }

    pub fn throwable_message(&self, obj: &JObj) -> String {
        match self.call(obj, "getMessage", &[]) {
            Ok(JVal::Str(s)) => s,
            Ok(JVal::Null) => String::new(),
            Ok(other) => format!("{:?}", other),
            Err(_) => String::new(),
        }
    }

    // ------------------------------------------------------------- proxies

    pub fn is_interface(&self, _runtime: RuntimeId, name: &str) -> bool {
        self.frame(|env| {
            let s = new_string(env, name);
            self.bridge_bool(
                env,
                "isInterfaceEngine",
                &[runtime_arg(_runtime), JValue { l: s }],
            )
        })
        .unwrap_or(false)
    }

    pub fn proxy(
        &self,
        _runtime: RuntimeId,
        interfaces: &[String],
        handle: i64,
    ) -> Result<JVal, JError> {
        self.frame(|env| {
            let string_class = unsafe { find_class(env, "java/lang/String") };
            let arr = unsafe {
                new_object_array(env, interfaces.len() as i32, string_class, ptr::null_mut())
            };
            for (i, name) in interfaces.iter().enumerate() {
                let s = new_string(env, name);
                unsafe { set_object_array_element(env, arr, i as i32, s) };
            }
            let v = self.call_bridge_obj(
                env,
                "proxyEngine",
                &[
                    runtime_arg(_runtime),
                    JValue { l: arr },
                    JValue { j: handle },
                ],
            )?;
            self.convert(env, v)
        })
    }

    fn string_array(&self, env: Env, arr: JArray) -> Result<Vec<String>, JError> {
        let len = unsafe { get_array_length(env, arr) };
        let mut out = Vec::with_capacity(len.max(0) as usize);
        for i in 0..len {
            let elem = unsafe { get_object_array_element(env, arr, i) };
            if !elem.is_null() {
                out.push(read_string(env, elem));
            }
        }
        Ok(out)
    }

    /// Parameter type names of the method that would be selected for a call.
    pub fn method_params(
        &self,
        _runtime: RuntimeId,
        target: Option<&JObj>,
        class: &str,
        name: &str,
        argc: usize,
        is_static: bool,
    ) -> Option<Vec<String>> {
        self.frame(|env| {
            let target_obj = target.map(|o| o.obj).unwrap_or(ptr::null_mut());
            let cn = new_string(env, class);
            let nm = new_string(env, name);
            let v = self.call_bridge_obj(
                env,
                "methodParamsEngine",
                &[
                    runtime_arg(_runtime),
                    JValue { l: target_obj },
                    JValue { l: cn },
                    JValue { l: nm },
                    JValue { i: argc as i32 },
                    JValue {
                        z: if is_static { 1 } else { 0 },
                    },
                ],
            )?;
            if v.is_null() {
                return Ok(None);
            }
            Ok(Some(self.string_array(env, v)?))
        })
        .ok()
        .flatten()
    }

    /// All overload signatures for a method name and argument count.
    pub fn method_overloads(
        &self,
        _runtime: RuntimeId,
        target: Option<&JObj>,
        class: &str,
        name: &str,
        argc: usize,
        is_static: bool,
    ) -> Option<Vec<Vec<String>>> {
        self.frame(|env| {
            let target_obj = target.map(|o| o.obj).unwrap_or(ptr::null_mut());
            let cn = new_string(env, class);
            let nm = new_string(env, name);
            let v = self.call_bridge_obj(
                env,
                "methodOverloadsEngine",
                &[
                    runtime_arg(_runtime),
                    JValue { l: target_obj },
                    JValue { l: cn },
                    JValue { l: nm },
                    JValue { i: argc as i32 },
                    JValue {
                        z: if is_static { 1 } else { 0 },
                    },
                ],
            )?;
            if v.is_null() {
                return Ok(None);
            }
            let count = unsafe { get_array_length(env, v) };
            let mut out = Vec::new();
            for i in 0..count {
                let element = unsafe { get_object_array_element(env, v, i) };
                if element.is_null() {
                    continue;
                }
                out.push(self.string_array(env, element)?);
            }
            Ok(Some(out))
        })
        .ok()
        .flatten()
    }

    /// (parameter count, return type) of a functional interface's SAM.
    pub fn sam_info(&self, _runtime: RuntimeId, class: &str) -> Option<(usize, String)> {
        self.frame(|env| {
            let cn = new_string(env, class);
            let v = self.call_bridge_obj(
                env,
                "samInfoEngine",
                &[runtime_arg(_runtime), JValue { l: cn }],
            )?;
            if v.is_null() {
                return Ok(None);
            }
            let values = self.string_array(env, v)?;
            if values.len() < 2 {
                return Ok(None);
            }
            let arity = match values[0].parse::<usize>() {
                Ok(value) => value,
                Err(_) => return Ok(None),
            };
            Ok(Some((arity, values[1].clone())))
        })
        .ok()
        .flatten()
    }

    pub fn ctor_params(
        &self,
        _runtime: RuntimeId,
        class: &str,
        argc: usize,
    ) -> Option<Vec<String>> {
        self.frame(|env| {
            let cn = new_string(env, class);
            let v = self.call_bridge_obj(
                env,
                "ctorParamsEngine",
                &[
                    runtime_arg(_runtime),
                    JValue { l: cn },
                    JValue { i: argc as i32 },
                ],
            )?;
            if v.is_null() {
                return Ok(None);
            }
            Ok(Some(self.string_array(env, v)?))
        })
        .ok()
        .flatten()
    }

    pub(crate) fn convert_env(&self, env: Env, obj: JObject) -> Result<JVal, JError> {
        self.convert(env, obj)
    }

    /// Import a local Java reference as a script value (raw pointer API for
    /// external JNI entry points).
    pub fn import_object_raw(
        &self,
        env: *mut std::ffi::c_void,
        obj: *mut std::ffi::c_void,
    ) -> Result<JVal, JError> {
        self.convert(env as Env, obj as JObject)
    }

    /// Export a script value as a local Java reference (raw pointer API).
    pub fn export_object_raw(
        &self,
        env: *mut std::ffi::c_void,
        value: &JVal,
    ) -> Result<*mut std::ffi::c_void, JError> {
        self.export_value(env as Env, value)
            .map(|object| object as *mut std::ffi::c_void)
    }

    /// Prepare a value for returning from a native method: the result is a
    /// local reference in the caller's frame.
    fn export_value(&self, env: Env, value: &JVal) -> Result<JObject, JError> {
        unsafe { push_local_frame(env, 16) };
        match self.to_java(env, value) {
            Ok(obj) => {
                let local = if obj.is_null() {
                    obj
                } else {
                    unsafe { new_local_ref(env, obj) }
                };
                Ok(unsafe { pop_local_frame(env, local) })
            }
            Err(e) => {
                unsafe { pop_local_frame(env, ptr::null_mut()) };
                Err(e)
            }
        }
    }

    // ------------------------------------------------- generated subclasses

    /// Define a class whose bytecode was generated on the Rust side.
    pub fn define_script_class(&self, name: &str, bytes: &[u8]) -> Result<Arc<JObj>, JError> {
        self.frame(|env| {
            let signed: Vec<i8> = bytes.iter().map(|b| *b as i8).collect();
            let arr = unsafe { new_byte_array(env, signed.len() as i32) };
            unsafe {
                set_byte_array_region(env, arr, 0, signed.len() as i32, signed.as_ptr());
            }
            let n = new_string(env, name);
            let obj = self.call_bridge_obj(
                env,
                "defineScriptClass",
                &[JValue { l: n }, JValue { l: arr }],
            )?;
            match self.wrap(env, obj, Some("java.lang.Class".to_string()))? {
                JVal::Obj(o) => Ok(o),
                _ => Err(JError::Api("defineClass returned null".to_string())),
            }
        })
    }

    /// A real java.lang.String object (for Java 11+ string methods).
    pub fn new_string_object(&self, text: &str) -> Result<Arc<JObj>, JError> {
        self.frame(|env| {
            let local = new_string(env, text);
            let global = unsafe { new_global_ref(env, local) };
            unsafe { delete_local_ref(env, local) };
            Ok(Arc::new(JObj {
                obj: global,
                class_name: "java.lang.String".to_string(),
                jvm: self.arc(),
            }))
        })
    }

    pub fn new_script_instance(&self, class: &JObj, handle: i64) -> Result<JVal, JError> {
        self.frame(|env| {
            let obj = self.call_bridge_obj(
                env,
                "newScriptInstance",
                &[JValue { l: class.obj }, JValue { j: handle }],
            )?;
            self.convert(env, obj)
        })
    }

    /// Lines of name|returnType|paramType|... for overridable methods.
    pub fn overridable_methods(&self, class: &str) -> Result<Vec<String>, JError> {
        self.frame(|env| {
            let n = new_string(env, class);
            let s = self.call_bridge_obj(env, "overridableMethods", &[JValue { l: n }])?;
            if s.is_null() {
                return Ok(Vec::new());
            }
            let text = read_string(env, s);
            Ok(text
                .lines()
                .filter(|l| !l.is_empty())
                .map(|l| l.to_string())
                .collect())
        })
    }

    pub fn start() -> Result<Arc<Jvm>, String> {
        let classes_dir = extract_bridge()?;
        let lib =
            find_libjvm().ok_or_else(|| "libjvm not found (set BSH_RS_LIBJVM)".to_string())?;
        let handle = load_library(&lib)?;
        let create: CreateJavaVmFn =
            unsafe { std::mem::transmute(lookup_symbol(handle, "JNI_CreateJavaVM")?) };
        let mut classpath = classes_dir.to_string_lossy().to_string();
        if let Ok(extra) = std::env::var("CLASSPATH") {
            if !extra.is_empty() {
                classpath.push(':');
                classpath.push_str(&extra);
            }
        }
        classpath.push_str(":.");
        let mut opt_strings: Vec<std::ffi::CString> = vec![
            std::ffi::CString::new(format!("-Djava.class.path={}", classpath)).unwrap(),
            std::ffi::CString::new("-Dfile.encoding=UTF-8").unwrap(),
            std::ffi::CString::new("-Xrs").unwrap(),
        ];
        let mut options: Vec<JavaVmOption> = opt_strings
            .iter_mut()
            .map(|s| JavaVmOption {
                option_string: s.as_ptr(),
                extra_info: ptr::null_mut(),
            })
            .collect();
        let mut args = JavaVmInitArgs {
            version: JNI_VERSION_1_6,
            n_options: options.len() as i32,
            options: options.as_mut_ptr(),
            ignore_unrecognized: 1,
        };
        let mut vm: Vm = ptr::null_mut();
        let mut env: Env = ptr::null_mut();
        let rc = unsafe { create(&mut vm, &mut env, &mut args) };
        if rc != JNI_OK {
            return Err(format!("JNI_CreateJavaVM failed ({})", rc));
        }
        Self::from_vm(vm, env, true)
    }

    /// Build the bridge on top of an existing VM (used on Android, where the
    /// application's ART runtime is the JVM and libjvm does not exist).
    fn from_vm(vm: Vm, env: Env, script_classes: bool) -> Result<Arc<Jvm>, String> {
        let bridge = unsafe { find_class(env, "bshrs/Bridge") };
        if bridge.is_null() {
            unsafe { exception_describe(env) };
            return Err("cannot load bshrs.Bridge (is it in the app classpath?)".to_string());
        }
        let bridge_global = unsafe { new_global_ref(env, bridge) };
        let mut methods = HashMap::new();
        for (key, name, sig) in BRIDGE_METHODS {
            let id = unsafe { get_static_method_id(env, bridge_global, name, sig) };
            if id.is_null() {
                unsafe { exception_describe(env) };
                return Err(format!("cannot resolve Bridge.{}", name));
            }
            methods.insert(*key, id);
        }
        // Register the native callback used by dynamic proxies.
        let native_class = unsafe { find_class(env, "bshrs/Native") };
        if native_class.is_null() {
            unsafe { exception_describe(env) };
            return Err("cannot load bshrs.Native".to_string());
        }
        let name = std::ffi::CString::new("invoke").unwrap();
        let sig =
            std::ffi::CString::new("(JLjava/lang/String;[Ljava/lang/Object;)Ljava/lang/Object;")
                .unwrap();
        let mut method = JNINativeMethod {
            name: name.as_ptr(),
            signature: sig.as_ptr(),
            fn_ptr: native_proxy_invoke as *mut std::ffi::c_void,
        };
        let rc = unsafe { register_natives(env, native_class, &mut method, 1) };
        if rc != 0 {
            unsafe { exception_describe(env) };
            return Err("RegisterNatives failed".to_string());
        }
        let jvm = Arc::new(Jvm {
            vm,
            bridge: bridge_global,
            methods,
            self_ref: OnceLock::new(),
            script_classes,
        });
        let _ = jvm.self_ref.set(Arc::downgrade(&jvm));
        if !script_classes {
            // Android/ART: worker-thread callback errors must not propagate out
            // of Thread.run(), which would kill the application process.
            let _ = jvm.set_static(
                RuntimeId::default(),
                "bshrs.Bridge",
                "catchCallbackErrors",
                &JVal::Bool(true),
            );
        }
        Ok(jvm)
    }

    /// ART cannot define raw Java class files, so generated subclasses are off.
    pub fn supports_script_classes(&self) -> bool {
        self.script_classes
    }
}

/// Attach to an already running VM (Android). The bridge classes must be part
/// of the application class path.
static EXTERNAL_VM: OnceLock<usize> = OnceLock::new();
static EXTERNAL: OnceLock<Arc<Jvm>> = OnceLock::new();

pub fn install_external(vm: *mut std::ffi::c_void) -> Result<(), String> {
    if vm.is_null() {
        return Err("null JavaVM".to_string());
    }
    let _ = EXTERNAL_VM.set(vm as usize);
    Ok(())
}

/// Build the bridge for an installed external VM on first use. Keeping this
/// lazy avoids loading and verifying every bridge method on process startup.
fn external_jvm() -> Option<Arc<Jvm>> {
    if let Some(jvm) = EXTERNAL.get() {
        return Some(jvm.clone());
    }
    let vm = *EXTERNAL_VM.get()? as Vm;
    let env = match unsafe { get_env(vm, JNI_VERSION_1_6) } {
        Ok(env) => env,
        Err(_) => unsafe { attach_current_thread(vm) }.ok()?,
    };
    let jvm = Jvm::from_vm(vm, env, false).ok()?;
    let _ = EXTERNAL.set(jvm.clone());
    Some(jvm)
}

// -------------------------------------------------------------------- helpers

fn new_string(env: Env, s: &str) -> JObject {
    unsafe { new_string_utf(env, s) }
}

fn read_string(env: Env, s: JString) -> String {
    unsafe {
        let chars = get_string_utf_chars(env, s);
        if chars.is_null() {
            return String::new();
        }
        let out = std::ffi::CStr::from_ptr(chars)
            .to_string_lossy()
            .to_string();
        release_string_utf_chars(env, s, chars);
        out
    }
}

fn extract_bridge() -> Result<PathBuf, String> {
    let dir = std::env::temp_dir().join(format!("bsh-rs-{}", std::process::id()));
    let pkg = dir.join("bshrs");
    std::fs::create_dir_all(&pkg).map_err(|e| format!("cannot create {}: {}", pkg.display(), e))?;
    for (name, bytes) in [
        ("Bridge.class", BRIDGE_CLASS),
        ("Bridge$Handler.class", BRIDGE_HANDLER_CLASS),
        ("Bridge$ScriptLoader.class", BRIDGE_LOADER_CLASS),
        ("Native.class", NATIVE_CLASS),
    ] {
        let path = pkg.join(name);
        std::fs::write(&path, bytes)
            .map_err(|e| format!("cannot write {}: {}", path.display(), e))?;
    }
    Ok(dir)
}

fn find_libjvm() -> Option<String> {
    if let Ok(p) = std::env::var("BSH_RS_LIBJVM") {
        if Path::new(&p).exists() {
            return Some(p);
        }
    }
    let names = [
        "lib/server/libjvm.so",
        "lib/server/libjvm.dylib",
        "jre/lib/server/libjvm.so",
        "bin/server/jvm.dll",
        "jre/bin/server/jvm.dll",
        "bin/client/jvm.dll",
    ];
    if let Ok(home) = std::env::var("JAVA_HOME") {
        for n in names.iter() {
            let p = Path::new(&home).join(n);
            if p.exists() {
                return Some(p.to_string_lossy().to_string());
            }
        }
    }
    if let Ok(out) = std::process::Command::new("java")
        .args(["-XshowSettings:properties", "-version"])
        .output()
    {
        let text = String::from_utf8_lossy(&out.stderr);
        for line in text.lines() {
            let line = line.trim();
            if let Some(rest) = line.strip_prefix("java.home = ") {
                for n in names.iter() {
                    let p = Path::new(rest.trim()).join(n);
                    if p.exists() {
                        return Some(p.to_string_lossy().to_string());
                    }
                }
            }
        }
    }
    for root in ["/usr/lib/jvm", "/usr/java", "/opt/java"] {
        if let Ok(entries) = std::fs::read_dir(root) {
            for entry in entries.flatten() {
                for n in names.iter() {
                    let p = entry.path().join(n);
                    if p.exists() {
                        return Some(p.to_string_lossy().to_string());
                    }
                }
            }
        }
    }
    None
}

/// Callback used to dispatch Java proxy calls back into the interpreter.
pub type ProxyHandler = Box<dyn Fn(i64, &str, Vec<JVal>) -> Result<JVal, String> + Send + Sync>;

static PROXY_HANDLER: OnceLock<ProxyHandler> = OnceLock::new();

pub fn set_proxy_handler(handler: ProxyHandler) {
    let _ = PROXY_HANDLER.set(handler);
}

unsafe extern "system" fn native_proxy_invoke(
    env: Env,
    _class: JClass,
    handle: i64,
    method: JString,
    args: JArray,
) -> JObject {
    let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(
        || -> Result<JObject, String> {
            let jvm = global().ok_or_else(|| "No JVM running".to_string())?;
            let method_name = read_string(env, method);
            let len = if args.is_null() {
                0
            } else {
                (unsafe { get_array_length(env, args) }) as usize
            };
            let mut values = Vec::with_capacity(len);
            for i in 0..len {
                let elem = unsafe { get_object_array_element(env, args, i as i32) };
                let value = jvm.convert_env(env, elem).map_err(|e| e.to_string())?;
                values.push(value);
            }
            let handler = PROXY_HANDLER
                .get()
                .ok_or_else(|| "No proxy handler installed".to_string())?;
            let result = handler(handle, &method_name, values)?;
            jvm.export_value(env, &result).map_err(|e| e.to_string())
        },
    ));
    match outcome {
        Ok(Ok(obj)) => obj,
        Ok(Err(msg)) => {
            if std::env::var("BSH_RS_DEBUG").is_ok() {
                eprintln!("[jni-native] callback error: {}", msg);
            }
            throw_runtime(env, &msg);
            ptr::null_mut()
        }
        Err(_) => {
            throw_runtime(env, "panic inside native proxy callback");
            ptr::null_mut()
        }
    }
}

fn throw_runtime(env: Env, msg: &str) {
    unsafe {
        let class = find_class(env, "java/lang/RuntimeException");
        let message = std::ffi::CString::new(msg.replace('\u{0}', "")).unwrap();
        throw_new(env, class, message.as_ptr());
    }
}

static STARTED: OnceLock<Result<Arc<Jvm>, String>> = OnceLock::new();

/// Start (or reuse) the process wide JVM.
pub fn start() -> Result<Arc<Jvm>, String> {
    if EXTERNAL_VM.get().is_some() {
        return external_jvm().ok_or_else(|| "external JVM bridge unavailable".to_string());
    }
    STARTED.get_or_init(Jvm::start).clone()
}

/// True when an external VM (Android ART) was installed.
pub fn has_external_vm() -> bool {
    EXTERNAL_VM.get().is_some()
}

/// The process wide JVM, if one could be started.
pub fn global() -> Option<Arc<Jvm>> {
    if EXTERNAL_VM.get().is_some() {
        return external_jvm();
    }
    start().ok()
}

/// Explain why the JVM is unavailable (empty when it works).
pub fn unavailable_reason() -> String {
    if EXTERNAL_VM.get().is_some() {
        return if external_jvm().is_some() {
            String::new()
        } else {
            "external JVM bridge unavailable".to_string()
        };
    }
    match STARTED.get_or_init(Jvm::start) {
        Ok(_) => String::new(),
        Err(e) => e.clone(),
    }
}

pub use bindings::JNI_VERSION_1_6 as VERSION;
