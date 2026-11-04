use bsh_runtime::eval::{array_component, expand_varargs_signature, host_arg_tag};
use bsh_runtime::Value;

fn signature(parts: &[&str]) -> Vec<String> {
    parts.iter().map(|part| part.to_string()).collect()
}

#[test]
fn array_component_uses_type_names() {
    assert_eq!(array_component("int[]"), Some("int"));
    assert_eq!(
        array_component("java.lang.String[]"),
        Some("java.lang.String")
    );
    assert_eq!(array_component("java.lang.String"), None);
}

#[test]
fn varargs_expands_fixed_and_component_types() {
    let sig = signature(&["int", "java.lang.String[]"]);
    assert_eq!(
        expand_varargs_signature(&sig, 3),
        Some(signature(&["int", "java.lang.String", "java.lang.String"]))
    );
    assert_eq!(expand_varargs_signature(&sig, 1), Some(signature(&["int"])));
    assert_eq!(expand_varargs_signature(&sig, 0), None);
}

#[test]
fn varargs_with_only_component_expands_to_arity() {
    let sig = signature(&["java.lang.String[]"]);
    assert_eq!(
        expand_varargs_signature(&sig, 2),
        Some(signature(&["java.lang.String", "java.lang.String"]))
    );
    assert_eq!(
        expand_varargs_signature(&sig, 1),
        Some(signature(&["java.lang.String"]))
    );
}

#[test]
fn host_tags_distinguish_primitives() {
    assert_eq!(host_arg_tag(&Value::Null), "N");
    assert_eq!(host_arg_tag(&Value::Bool(true)), "Z");
    assert_eq!(host_arg_tag(&Value::Int(1)), "I");
    assert_eq!(host_arg_tag(&Value::Long(1)), "J");
    assert_eq!(host_arg_tag(&Value::Double(1.0)), "D");
    assert_eq!(host_arg_tag(&Value::string("x")), "T");
}
