# Contributing

Thanks for taking the time to improve BeanShell-Rust. This document covers the
basics: how to build, what to run before sending a change, and how to report a
problem.

[中文说明](CONTRIBUTING.zh-CN.md)

## Before you start

- Search the existing issues to avoid duplicates.
- For larger changes, open an issue first so the design can be discussed.
- Keep changes focused; unrelated refactors belong in a separate pull request.

## Build and test

Requirements: Rust 1.75+ and a JDK 8+ for Java interop.

```sh
cargo build --release
cargo test --workspace
cargo fmt --all -- --check
cargo clippy --workspace --all-targets
```

The Java reflection bridge is compiled by `crates/bsh-jvm/build.rs` with `javac`;
set `JAVA_HOME` or `JAVAC` if `javac` is not on `PATH`.

### Differential testing

To compare behaviour against the Java implementation:

```sh
BSH_ORACLE=/path/to/beanshell/run.sh tools/oracle-diff.py
```

## Android

Android artifacts are built from the `android/` Gradle project:

```sh
ANDROID_NDK_HOME=/path/to/ndk \
  cargo build --release --target aarch64-linux-android -p bsh-rs

mkdir -p android/bsh-rs/src/main/jniLibs/arm64-v8a
cp target/aarch64-linux-android/release/libbsh_rs.so \
   android/bsh-rs/src/main/jniLibs/arm64-v8a/

cd android && ./gradlew :bsh-rs:assembleRelease :app:assembleRelease
```

## Pull requests

- Describe the problem and the approach in the description.
- Reference the issue it fixes, for example `Fixes #123`.
- Add or update tests for behaviour changes.
- Keep the commit messages in the imperative mood, for example
  `fix: resolve catch matching for simple names`.
- Make sure CI is green before asking for a review.

## Reporting bugs

Use the bug report template and include:

- the platform (Linux, macOS, Windows or Android) and its version,
- the Rust and JDK versions,
- a minimal script that reproduces the problem,
- what you expected and what happened instead.

## Code style

- Rust: `cargo fmt`, no new warnings from `cargo clippy`.
- Java: 4-space indentation, keep JNI method names in sync with the Rust side.
- Markdown: wrap at roughly 80 columns.

## License

By contributing you agree that your changes are licensed under the Apache
License 2.0, the license of this repository.
