//! Parse every file given on the command line and report the failures.
//!
//! Usage: parse-check <file.bsh>...

use std::fs;

fn main() {
    let mut ok = 0usize;
    let mut bad = 0usize;
    for path in std::env::args().skip(1) {
        let src = match fs::read_to_string(&path) {
            Ok(s) => s,
            Err(e) => {
                println!("READ-ERROR {}: {}", path, e);
                bad += 1;
                continue;
            }
        };
        match bsh_syntax::parse(&src) {
            Ok(_) => ok += 1,
            Err(e) => {
                bad += 1;
                println!(
                    "PARSE-ERROR {}:{}:{}: {}",
                    path, e.span.line, e.span.col, e.message
                );
            }
        }
    }
    println!("parse-ok={} parse-fail={}", ok, bad);
    if bad > 0 {
        std::process::exit(1);
    }
}
