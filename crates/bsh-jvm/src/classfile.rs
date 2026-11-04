//! Minimal Java class file writer used to synthesize subclasses of Java
//! classes for scripted classes.
//!
//! The generated class holds a single long `handle` field. Its overridden
//! methods forward to `bshrs.Native.invoke(handle, name, args)`, which is the
//! same JNI callback used by dynamic proxies. For every override we also emit a
//! `__super_<name>_<argc>` method that performs an invokespecial call so that
//! scripts can invoke the superclass implementation without recursion.

use std::collections::HashMap;

const ACC_PUBLIC: u16 = 0x0001;
const ACC_PRIVATE: u16 = 0x0002;
const ACC_SUPER: u16 = 0x0020;

// ------------------------------------------------------------ constant pool

enum Entry {
    Utf8(String),
    Class(u16),
    String(u16),
    FieldRef(u16, u16),
    MethodRef(u16, u16),
    NameAndType(u16, u16),
}

struct Pool {
    entries: Vec<Entry>,
    utf8: HashMap<String, u16>,
    classes: HashMap<String, u16>,
    strings: HashMap<String, u16>,
    names: HashMap<(String, String), u16>,
    members: HashMap<(&'static str, u16, u16), u16>,
}

impl Pool {
    fn new() -> Pool {
        Pool {
            entries: Vec::new(),
            utf8: HashMap::new(),
            classes: HashMap::new(),
            strings: HashMap::new(),
            names: HashMap::new(),
            members: HashMap::new(),
        }
    }

    fn push(&mut self, entry: Entry) -> u16 {
        self.entries.push(entry);
        self.entries.len() as u16
    }

    fn utf8(&mut self, s: &str) -> u16 {
        if let Some(i) = self.utf8.get(s) {
            return *i;
        }
        let i = self.push(Entry::Utf8(s.to_string()));
        self.utf8.insert(s.to_string(), i);
        i
    }

    fn class(&mut self, internal: &str) -> u16 {
        if let Some(i) = self.classes.get(internal) {
            return *i;
        }
        let name = self.utf8(internal);
        let i = self.push(Entry::Class(name));
        self.classes.insert(internal.to_string(), i);
        i
    }

    fn string(&mut self, s: &str) -> u16 {
        if let Some(i) = self.strings.get(s) {
            return *i;
        }
        let utf = self.utf8(s);
        let i = self.push(Entry::String(utf));
        self.strings.insert(s.to_string(), i);
        i
    }

    fn name_and_type(&mut self, name: &str, desc: &str) -> u16 {
        let key = (name.to_string(), desc.to_string());
        if let Some(i) = self.names.get(&key) {
            return *i;
        }
        let n = self.utf8(name);
        let d = self.utf8(desc);
        let i = self.push(Entry::NameAndType(n, d));
        self.names.insert(key, i);
        i
    }

    fn method(&mut self, class: &str, name: &str, desc: &str) -> u16 {
        let c = self.class(class);
        let nat = self.name_and_type(name, desc);
        let key = ("method", c, nat);
        if let Some(i) = self.members.get(&key) {
            return *i;
        }
        let i = self.push(Entry::MethodRef(c, nat));
        self.members.insert(key, i);
        i
    }

    fn field(&mut self, class: &str, name: &str, desc: &str) -> u16 {
        let c = self.class(class);
        let nat = self.name_and_type(name, desc);
        let key = ("field", c, nat);
        if let Some(i) = self.members.get(&key) {
            return *i;
        }
        let i = self.push(Entry::FieldRef(c, nat));
        self.members.insert(key, i);
        i
    }

    fn write(&self, out: &mut Vec<u8>) {
        write_u16(out, (self.entries.len() + 1) as u16);
        for e in &self.entries {
            match e {
                Entry::Utf8(s) => {
                    out.push(1);
                    let bytes = s.as_bytes();
                    write_u16(out, bytes.len() as u16);
                    out.extend_from_slice(bytes);
                }
                Entry::Class(i) => {
                    out.push(7);
                    write_u16(out, *i);
                }
                Entry::String(i) => {
                    out.push(8);
                    write_u16(out, *i);
                }
                Entry::FieldRef(c, n) => {
                    out.push(9);
                    write_u16(out, *c);
                    write_u16(out, *n);
                }
                Entry::MethodRef(c, n) => {
                    out.push(10);
                    write_u16(out, *c);
                    write_u16(out, *n);
                }
                Entry::NameAndType(n, d) => {
                    out.push(12);
                    write_u16(out, *n);
                    write_u16(out, *d);
                }
            }
        }
    }
}

fn write_u16(out: &mut Vec<u8>, v: u16) {
    out.extend_from_slice(&v.to_be_bytes());
}

fn write_u32(out: &mut Vec<u8>, v: u32) {
    out.extend_from_slice(&v.to_be_bytes());
}

// ------------------------------------------------------------------ builder

pub struct MethodInfo {
    pub access: u16,
    pub name: u16,
    pub descriptor: u16,
    pub max_stack: u16,
    pub max_locals: u16,
    pub code: Vec<u8>,
}

pub struct FieldInfo {
    pub access: u16,
    pub name: u16,
    pub descriptor: u16,
}

pub struct ClassBuilder {
    pool: Pool,
    this_internal: String,
    super_internal: String,
    fields: Vec<FieldInfo>,
    methods: Vec<MethodInfo>,
    handle_field: u16,
}

impl ClassBuilder {
    pub fn new(internal_name: &str, super_internal: &str) -> ClassBuilder {
        let mut builder = ClassBuilder {
            pool: Pool::new(),
            this_internal: internal_name.to_string(),
            super_internal: super_internal.to_string(),
            fields: Vec::new(),
            methods: Vec::new(),
            handle_field: 0,
        };
        let name = builder.pool.utf8("handle");
        let desc = builder.pool.utf8("J");
        builder.fields.push(FieldInfo {
            access: ACC_PRIVATE,
            name,
            descriptor: desc,
        });
        builder.handle_field = builder.pool.field(internal_name, "handle", "J");
        builder
    }

    /// Constructor (J)V: call the superclass no-argument constructor and store
    /// the callback handle.
    pub fn add_constructor(&mut self) {
        let mut code = Vec::new();
        // aload_0
        code.push(0x2a);
        let super_init = self
            .pool
            .method(&self.super_internal.clone(), "<init>", "()V");
        code.push(0xb7);
        write_u16(&mut code, super_init);
        // aload_0; lload_1; putfield handle
        code.push(0x2a);
        code.push(0x1f); // lload_1
        code.push(0xb5);
        write_u16(&mut code, self.handle_field);
        code.push(0xb1); // return
        let name = self.pool.utf8("<init>");
        let desc = self.pool.utf8("(J)V");
        self.methods.push(MethodInfo {
            access: ACC_PUBLIC,
            name,
            descriptor: desc,
            max_stack: 4,
            max_locals: 3,
            code,
        });
    }

    /// Add an override that forwards to the script through Native.invoke.
    pub fn add_dispatch_method(&mut self, name: &str, desc: &str, params: &[String], ret: &str) {
        let slots = param_slots(params);
        let mut code = Vec::new();
        // this.handle
        code.push(0x2a);
        code.push(0xb4);
        write_u16(&mut code, self.handle_field);
        // ldc_w "name"
        let name_idx = self.pool.string(name);
        code.push(0x13);
        write_u16(&mut code, name_idx);
        // new Object[n]
        emit_iconst(&mut code, params.len() as i32);
        let object_class = self.pool.class("java/lang/Object");
        code.push(0xbd);
        write_u16(&mut code, object_class);
        let mut slot = 1u16;
        for p in params {
            code.push(0x59); // dup
            emit_load(&mut code, p, slot);
            self.emit_box(&mut code, p);
            code.push(0x53); // aastore
            slot += slot_width(p);
        }
        let invoke = self.pool.method(
            "bshrs/Native",
            "invoke",
            "(JLjava/lang/String;[Ljava/lang/Object;)Ljava/lang/Object;",
        );
        code.push(0xb8);
        write_u16(&mut code, invoke);
        emit_return_conversion(&mut code, ret, &mut self.pool);
        let name_idx = self.pool.utf8(name);
        let desc_idx = self.pool.utf8(desc);
        self.methods.push(MethodInfo {
            access: ACC_PUBLIC,
            name: name_idx,
            descriptor: desc_idx,
            max_stack: slots + 8,
            max_locals: slots + 1,
            code,
        });
    }

    /// Add a non-virtual superclass bridge: __super_name_argc(...)
    pub fn add_super_forwarder(
        &mut self,
        name: &str,
        desc: &str,
        params: &[String],
        ret: &str,
        argc: usize,
    ) {
        let slots = param_slots(params);
        let mut code = Vec::new();
        code.push(0x2a); // aload_0
        let mut slot = 1u16;
        for p in params {
            emit_load(&mut code, p, slot);
            slot += slot_width(p);
        }
        let target = self.pool.method(&self.super_internal.clone(), name, desc);
        code.push(0xb7); // invokespecial
        write_u16(&mut code, target);
        emit_xreturn(&mut code, ret, &mut self.pool);
        let bridge_name = format!("__super_{}_{}", name, argc);
        let name_idx = self.pool.utf8(&bridge_name);
        let desc_idx = self.pool.utf8(desc);
        self.methods.push(MethodInfo {
            access: ACC_PUBLIC,
            name: name_idx,
            descriptor: desc_idx,
            max_stack: slots + 2,
            max_locals: slots + 1,
            code,
        });
    }

    fn emit_box(&mut self, code: &mut Vec<u8>, ty: &str) {
        let (wrapper, method_name, desc) = match ty {
            "I" => ("java/lang/Integer", "valueOf", "(I)Ljava/lang/Integer;"),
            "Z" => ("java/lang/Boolean", "valueOf", "(Z)Ljava/lang/Boolean;"),
            "B" => ("java/lang/Byte", "valueOf", "(B)Ljava/lang/Byte;"),
            "C" => ("java/lang/Character", "valueOf", "(C)Ljava/lang/Character;"),
            "S" => ("java/lang/Short", "valueOf", "(S)Ljava/lang/Short;"),
            "J" => ("java/lang/Long", "valueOf", "(J)Ljava/lang/Long;"),
            "F" => ("java/lang/Float", "valueOf", "(F)Ljava/lang/Float;"),
            "D" => ("java/lang/Double", "valueOf", "(D)Ljava/lang/Double;"),
            _ => return, // reference type: no boxing
        };
        let idx = self.pool.method(wrapper, method_name, desc);
        code.push(0xb8);
        write_u16(code, idx);
    }

    /// Test helper: a method (I)I that returns its argument plus delta.
    pub fn add_test_int_plus_one(&mut self, name: &str, delta: i32) {
        let mut code = Vec::new();
        code.push(0x1b); // iload_1
        emit_iconst(&mut code, delta);
        code.push(0x60); // iadd
        code.push(0xac); // ireturn
        let name_idx = self.pool.utf8(name);
        let desc_idx = self.pool.utf8("(I)I");
        self.methods.push(MethodInfo {
            access: ACC_PUBLIC,
            name: name_idx,
            descriptor: desc_idx,
            max_stack: 3,
            max_locals: 2,
            code,
        });
    }

    pub fn finish(&mut self) -> Vec<u8> {
        // Intern everything used below before the pool is serialized.
        let this_class = self.pool.class(&self.this_internal);
        let super_class = self.pool.class(&self.super_internal);
        let code_name = self.pool.utf8("Code");
        let mut out = Vec::new();
        write_u32(&mut out, 0xCAFE_BABE);
        write_u16(&mut out, 0);
        write_u16(&mut out, 52); // Java 8
        self.pool.write(&mut out);
        write_u16(&mut out, ACC_PUBLIC | ACC_SUPER);
        write_u16(&mut out, this_class);
        write_u16(&mut out, super_class);
        write_u16(&mut out, 0); // interfaces
        write_u16(&mut out, self.fields.len() as u16);
        for f in &self.fields {
            write_u16(&mut out, f.access);
            write_u16(&mut out, f.name);
            write_u16(&mut out, f.descriptor);
            write_u16(&mut out, 0); // attributes
        }
        write_u16(&mut out, self.methods.len() as u16);
        for m in &self.methods {
            write_u16(&mut out, m.access);
            write_u16(&mut out, m.name);
            write_u16(&mut out, m.descriptor);
            write_u16(&mut out, 1); // one attribute: Code
            write_u16(&mut out, code_name);
            let attr_len = 12 + m.code.len() as u32;
            write_u32(&mut out, attr_len);
            write_u16(&mut out, m.max_stack);
            write_u16(&mut out, m.max_locals);
            write_u32(&mut out, m.code.len() as u32);
            out.extend_from_slice(&m.code);
            write_u16(&mut out, 0); // exception table
            write_u16(&mut out, 0); // code attributes
        }
        write_u16(&mut out, 0); // class attributes
        out
    }
}

fn param_slots(params: &[String]) -> u16 {
    params.iter().map(|p| slot_width(p)).sum()
}

fn slot_width(desc: &str) -> u16 {
    if desc == "J" || desc == "D" {
        2
    } else {
        1
    }
}

fn emit_iconst(code: &mut Vec<u8>, value: i32) {
    match value {
        -1 => code.push(0x02),
        0..=5 => code.push(0x03 + value as u8),
        _ if value <= 127 => {
            code.push(0x10);
            code.push(value as u8);
        }
        _ => {
            code.push(0x11);
            write_u16(code, value as u16);
        }
    }
}

fn emit_load(code: &mut Vec<u8>, desc: &str, slot: u16) {
    let first = desc.as_bytes()[0];
    let (short0, generic) = match first {
        b'J' => (0x1e, 0x16),
        b'F' => (0x22, 0x17),
        b'D' => (0x26, 0x18),
        b'L' | b'[' => (0x2a, 0x19),
        _ => (0x1a, 0x15), // int-like
    };
    if slot <= 3 {
        code.push(short0 + slot as u8);
    } else {
        code.push(generic);
        code.push(slot as u8);
    }
}

fn emit_xreturn(code: &mut Vec<u8>, ret: &str, pool: &mut Pool) {
    if ret == "V" {
        code.push(0xb1);
        return;
    }
    let opcode = match ret.as_bytes()[0] {
        b'I' | b'Z' | b'B' | b'C' | b'S' => 0xac,
        b'J' => 0xad,
        b'F' => 0xae,
        b'D' => 0xaf,
        _ => {
            let _ = pool;
            0xb0
        }
    };
    code.push(opcode);
}

fn emit_return_conversion(code: &mut Vec<u8>, ret: &str, pool: &mut Pool) {
    if ret == "V" {
        code.push(0x57); // pop
        code.push(0xb1);
        return;
    }
    if ret.as_bytes()[0] == b'L' || ret.as_bytes()[0] == b'[' {
        let class = pool.class(&internal_of(ret));
        code.push(0xc0); // checkcast
        write_u16(code, class);
        code.push(0xb0);
        return;
    }
    let (wrapper, method_name, desc) = match ret {
        "I" => ("java/lang/Integer", "intValue", "()I"),
        "Z" => ("java/lang/Boolean", "booleanValue", "()Z"),
        "B" => ("java/lang/Byte", "byteValue", "()B"),
        "C" => ("java/lang/Character", "charValue", "()C"),
        "S" => ("java/lang/Short", "shortValue", "()S"),
        "J" => ("java/lang/Long", "longValue", "()J"),
        "F" => ("java/lang/Float", "floatValue", "()F"),
        "D" => ("java/lang/Double", "doubleValue", "()D"),
        _ => {
            code.push(0x57);
            code.push(0xb1);
            return;
        }
    };
    let class = pool.class(wrapper);
    code.push(0xc0); // checkcast
    write_u16(code, class);
    let call = pool.method(wrapper, method_name, desc);
    code.push(0xb6); // invokevirtual
    write_u16(code, call);
    emit_xreturn(code, ret, pool);
}

fn internal_of(desc: &str) -> String {
    if desc.starts_with('[') {
        // Array descriptors are used directly by checkcast.
        return desc.to_string();
    }
    desc.trim_start_matches('L')
        .trim_end_matches(';')
        .to_string()
}

// ------------------------------------------------------- descriptor helpers

/// Convert a Java type name (as returned by Class.getTypeName) to a JVM
/// descriptor.
pub fn type_descriptor(name: &str) -> String {
    match name {
        "void" => "V".to_string(),
        "boolean" => "Z".to_string(),
        "byte" => "B".to_string(),
        "char" => "C".to_string(),
        "short" => "S".to_string(),
        "int" => "I".to_string(),
        "long" => "J".to_string(),
        "float" => "F".to_string(),
        "double" => "D".to_string(),
        _ => {
            if let Some(base) = name.strip_suffix("[]") {
                format!("[{}", type_descriptor(base))
            } else {
                format!("L{};", name.replace('.', "/"))
            }
        }
    }
}

pub fn method_descriptor(params: &[String], ret: &str) -> String {
    let mut out = String::from("(");
    for p in params {
        out.push_str(&type_descriptor(p));
    }
    out.push(')');
    out.push_str(&type_descriptor(ret));
    out
}
