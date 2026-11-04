//! Every script of the upstream BeanShell test suite must at least parse.

use std::fs;
use std::path::PathBuf;

#[test]
fn parses_upstream_suite() {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/upstream-scripts");
    let mut checked = 0usize;
    let mut failures = Vec::new();
    let entries = match fs::read_dir(&dir) {
        Ok(e) => e,
        Err(_) => return, // suite not vendored: nothing to check
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("bsh") {
            continue;
        }
        let src = fs::read_to_string(&path).unwrap();
        if let Err(e) = bsh_syntax::parse(&src) {
            failures.push(format!("{}: {}", path.display(), e));
        }
        checked += 1;
    }
    assert!(
        checked > 100,
        "expected the vendored suite, found {} files",
        checked
    );
    assert!(
        failures.is_empty(),
        "parse failures:\n{}",
        failures.join("\n")
    );
}
