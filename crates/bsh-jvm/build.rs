//! Compiles the tiny Java reflection bridge (bshrs.Bridge / bshrs.Native) into
//! java-classes/, whose bytecode is embedded into the Rust library with
//! include_bytes!.

use std::path::{Path, PathBuf};
use std::process::Command;

const CLASSES: &[&str] = &[
    "Bridge.class",
    "Bridge$Handler.class",
    "Bridge$ScriptLoader.class",
    "Native.class",
];

fn main() {
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let src = manifest.join("java");
    let out = manifest.join("java-classes");

    println!("cargo:rerun-if-changed=java");
    println!("cargo:rerun-if-env-changed=JAVAC");
    println!("cargo:rerun-if-env-changed=JAVA_HOME");

    let classes_dir = out.join("bshrs");
    let up_to_date = CLASSES.iter().all(|name| {
        let compiled = classes_dir.join(name);
        compiled.exists()
            && [src.join("bshrs/Bridge.java"), src.join("bshrs/Native.java")]
                .iter()
                .all(|source| {
                    let (Ok(a), Ok(b)) = (compiled.metadata(), source.metadata()) else {
                        return false;
                    };
                    match (a.modified(), b.modified()) {
                        (Ok(a), Ok(b)) => a >= b,
                        _ => false,
                    }
                })
    });
    if up_to_date {
        return;
    }

    std::fs::create_dir_all(&classes_dir).expect("create java-classes/bshrs");

    let javac = std::env::var("JAVAC")
        .ok()
        .or_else(|| {
            std::env::var("JAVA_HOME")
                .ok()
                .map(|home| Path::new(&home).join("bin/javac").display().to_string())
        })
        .unwrap_or_else(|| "javac".to_string());

    let status = Command::new(&javac)
        .arg("-encoding")
        .arg("UTF-8")
        .arg("-d")
        .arg(&out)
        .arg(src.join("bshrs/Bridge.java"))
        .arg(src.join("bshrs/Native.java"))
        .status()
        .unwrap_or_else(|e| {
            panic!("failed to run {javac}: {e}; install a JDK or set JAVAC/JAVA_HOME")
        });

    assert!(status.success(), "{javac} failed");
}
