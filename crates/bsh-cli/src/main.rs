//! Command line front end for the Rust BeanShell implementation.
//!
//! Usage:
//!   bsh [options] [script.bsh [args...]]
//!   bsh -e "expression"
//!
//! Options:
//!   -e CODE     evaluate CODE instead of a file
//!   -q          quiet (no banner in interactive mode)
//!   --cwd DIR   working directory used for relative file access

use bsh_runtime::Interpreter;
use std::io::{self, BufRead, Write};
use std::path::PathBuf;
use std::process::ExitCode;

fn main() -> ExitCode {
    let mut args = std::env::args().skip(1);
    let mut eval_code: Option<String> = None;
    let mut script: Option<PathBuf> = None;
    let mut cwd: Option<PathBuf> = None;
    let mut quiet = false;

    while let Some(arg) = args.next() {
        match arg.as_str() {
            "-e" => {
                eval_code = args.next();
            }
            "-q" => quiet = true,
            "-cp" | "-classpath" => {
                if let Some(path) = args.next() {
                    let mut classpath = std::env::var("CLASSPATH").unwrap_or_default();
                    if !classpath.is_empty() {
                        classpath.push(':');
                    }
                    classpath.push_str(&path);
                    std::env::set_var("CLASSPATH", classpath);
                }
            }
            "--no-jvm" => std::env::set_var("BSH_RS_JVM", "0"),
            "--cwd" => cwd = args.next().map(PathBuf::from),
            "-h" | "--help" => {
                print_help();
                return ExitCode::SUCCESS;
            }
            other => {
                script = Some(PathBuf::from(other));
                break;
            }
        }
    }

    let stdout: Box<dyn Write> = Box::new(io::stdout());
    let mut interp = Interpreter::new(stdout);
    if let Some(dir) = cwd {
        interp.cwd = dir;
    }
    // The JVM starts on demand; only report an explicitly disabled one.
    let _ = &interp;

    if let Some(code) = eval_code {
        return run_code(&mut interp, &code);
    }
    if let Some(path) = script {
        if let Some(parent) = path.parent() {
            if !parent.as_os_str().is_empty() {
                if let Ok(abs) = std::fs::canonicalize(parent) {
                    interp.cwd = abs;
                }
            }
        }
        let src = match std::fs::read_to_string(&path) {
            Ok(s) => s,
            Err(e) => {
                eprintln!("bsh: cannot read {}: {}", path.display(), e);
                return ExitCode::from(1);
            }
        };
        interp.source_file = Some(path.display().to_string());
        match interp.eval_source(&src) {
            Ok(_) => return ExitCode::SUCCESS,
            Err(e) => {
                eprintln!("{}", e);
                return ExitCode::from(1);
            }
        }
    }

    // Non interactive stdin: evaluate the whole stream (differential testing mode).
    use std::io::IsTerminal;
    if !io::stdin().is_terminal() {
        let mut src = String::new();
        if io::Read::read_to_string(&mut io::stdin(), &mut src).is_ok() {
            return run_code(&mut interp, &src);
        }
    }
    if !quiet {
        println!("BeanShell (Rust) 0.1.0 -- type 'exit' or Ctrl-D to quit");
    }
    repl(&mut interp)
}

fn run_code(interp: &mut Interpreter, code: &str) -> ExitCode {
    match interp.eval_source(code) {
        Ok(_) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("{}", e);
            ExitCode::from(1)
        }
    }
}

fn repl(interp: &mut Interpreter) -> ExitCode {
    let stdin = io::stdin();
    let mut line = String::new();
    loop {
        print!("bsh> ");
        let _ = io::stdout().flush();
        line.clear();
        match stdin.lock().read_line(&mut line) {
            Ok(0) => break,
            Ok(_) => {}
            Err(e) => {
                eprintln!("bsh: {}", e);
                return ExitCode::from(1);
            }
        }
        let trimmed = line.trim();
        if trimmed == "exit" || trimmed == "exit();" {
            break;
        }
        if trimmed.is_empty() {
            continue;
        }
        match interp.eval_source(&line) {
            Ok(v) => {
                if !matches!(v, bsh_runtime::Value::Void | bsh_runtime::Value::Null) {
                    println!("{}", v.to_display_string());
                }
            }
            Err(e) => eprintln!("{}", e),
        }
    }
    ExitCode::SUCCESS
}

fn print_help() {
    println!("Usage: bsh [options] [script.bsh]");
    println!("  -e CODE     evaluate CODE");
    println!("  -q          quiet mode");
    println!("  --cwd DIR   working directory");
    println!("  -cp PATH    append to the JVM classpath");
    println!("  --no-jvm    do not start the embedded JVM");
    println!();
    println!("Environment: BSH_RS_JVM=0 disables the JVM, BSH_RS_LIBJVM overrides libjvm path.");
}
