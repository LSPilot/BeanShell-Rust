//! Smoke tests for the embedded JVM bridge.

use bsh_jvm::{JVal, Jvm, RuntimeId};

const RT: RuntimeId = RuntimeId(1);

fn jvm() -> Option<std::sync::Arc<Jvm>> {
    match bsh_jvm::start() {
        Ok(j) => Some(j),
        Err(e) => {
            eprintln!("JVM unavailable, skipping: {}", e);
            None
        }
    }
}

fn obj(v: JVal) -> std::sync::Arc<bsh_jvm::JObj> {
    match v {
        JVal::Obj(o) => o,
        other => panic!("expected object, got {:?}", other),
    }
}

#[test]
fn big_integer_arithmetic() {
    let Some(jvm) = jvm() else { return };
    let a = obj(jvm
        .new_instance(RT, "java.math.BigInteger", &[JVal::Str("12".into())])
        .unwrap());
    let b = obj(jvm
        .new_instance(RT, "java.math.BigInteger", &[JVal::Str("30".into())])
        .unwrap());
    let sum = jvm.call(&a, "add", &[JVal::Obj(b)]).unwrap();
    let sum = obj(sum);
    assert_eq!(jvm.to_java_string(&sum), "42");
    assert_eq!(sum.class_name(), "java.math.BigInteger");
    let pow = jvm.call(&sum, "pow", &[JVal::Int(2)]).unwrap();
    assert_eq!(jvm.to_java_string(&obj(pow)), "1764");
}

#[test]
fn static_calls_and_properties() {
    let Some(jvm) = jvm() else { return };
    let version = jvm
        .call_static(
            RT,
            "java.lang.System",
            "getProperty",
            &[JVal::Str("java.version".into())],
        )
        .unwrap();
    match version {
        JVal::Str(s) => assert!(!s.is_empty()),
        other => panic!("unexpected {:?}", other),
    }
    assert!(jvm.class_exists(RT, "java.util.Date"));
    assert!(!jvm.class_exists(RT, "no.such.ClassHere"));
}

#[test]
fn strings_and_arrays() {
    let Some(jvm) = jvm() else { return };
    // java.lang.String crosses the bridge as a Rust string.
    match jvm
        .new_instance(RT, "java.lang.String", &[JVal::Str("hello".into())])
        .unwrap()
    {
        JVal::Str(s) => assert_eq!(s, "hello"),
        other => panic!("unexpected {:?}", other),
    }
    // StringBuilder stays an object and supports chained calls.
    let sb = obj(jvm
        .new_instance(RT, "java.lang.StringBuilder", &[JVal::Str("ab".into())])
        .unwrap());
    jvm.call(&sb, "append", &[JVal::Str("cd".into())]).unwrap();
    assert_eq!(jvm.to_java_string(&sb), "abcd");
    let arr = jvm.new_array(RT, "int", 3).unwrap();
    let arr = obj(arr);
    jvm.array_set(&arr, 1, &JVal::Int(7)).unwrap();
    assert_eq!(jvm.array_len(&arr).unwrap(), 3);
    match jvm.array_get(&arr, 1).unwrap() {
        JVal::Int(v) => assert_eq!(v, 7),
        other => panic!("unexpected {:?}", other),
    }
}

#[test]
fn java_exceptions_are_reported() {
    let Some(jvm) = jvm() else { return };
    let err = jvm.new_instance(RT, "java.lang.Integer", &[JVal::Str("nope".into())]);
    match err {
        Err(bsh_jvm::JError::Thrown(t)) => {
            assert_eq!(t.class_name(), "java.lang.NumberFormatException");
            assert!(!jvm.throwable_message(&t).is_empty());
        }
        other => panic!("expected thrown exception, got {:?}", other),
    }
}

#[test]
fn generated_class_roundtrip() {
    let Some(jvm) = jvm() else { return };
    let mut builder = bsh_jvm::classfile::ClassBuilder::new("bshrs/gen/Probe1", "java/lang/Object");
    builder.add_constructor();
    builder.add_test_int_plus_one("plusOne", 1);
    let bytes = builder.finish();
    std::fs::write("/tmp/probe1.class", &bytes).ok();
    let class = match jvm.define_script_class("bshrs.gen.Probe1", &bytes) {
        Ok(c) => c,
        Err(bsh_jvm::JError::Thrown(t)) => {
            panic!("ClassFormatError: {}", jvm.throwable_message(&t))
        }
        Err(e) => panic!("define class: {}", e),
    };
    let instance = jvm.new_script_instance(&class, 7).expect("instantiate");
    let object = match instance {
        JVal::Obj(o) => o,
        other => panic!("unexpected {:?}", other),
    };
    match jvm.call(&object, "plusOne", &[JVal::Int(41)]).unwrap() {
        JVal::Int(v) => assert_eq!(v, 42),
        other => panic!("unexpected {:?}", other),
    }
}

#[test]
fn fields_and_instances() {
    let Some(jvm) = jvm() else { return };
    let point = obj(jvm
        .new_instance(RT, "java.awt.Point", &[JVal::Int(3), JVal::Int(4)])
        .unwrap());
    match jvm.get_field(&point, "x").unwrap() {
        JVal::Int(v) => assert_eq!(v, 3),
        other => panic!("unexpected {:?}", other),
    }
    jvm.set_field(&point, "x", &JVal::Int(9)).unwrap();
    assert!(jvm.is_instance(RT, &point, "java.awt.Point"));
    assert!(jvm.is_instance(RT, &point, "java.lang.Object"));
}
