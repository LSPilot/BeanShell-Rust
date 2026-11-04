# BeanShell-Rust

<p align="center">
  <a href="https://github.com/Lspilot/BeanShell-Rust/actions/workflows/ci.yml"><img alt="CI" src="https://github.com/Lspilot/BeanShell-Rust/actions/workflows/ci.yml/badge.svg"></a>
  <a href="https://github.com/Lspilot/BeanShell-Rust/releases"><img alt="Release" src="https://img.shields.io/github/v/release/Lspilot/BeanShell-Rust?sort=semver&display_name=tag"></a>
  <a href="LICENSE"><img alt="License" src="https://img.shields.io/badge/license-Apache--2.0-blue.svg"></a>
  <a href="https://www.rust-lang.org"><img alt="Rust" src="https://img.shields.io/badge/rust-1.75%2B-orange.svg"></a>
  <img alt="Platforms" src="https://img.shields.io/badge/platform-linux%20%7C%20macOS%20%7C%20android-lightgrey.svg">
</p>

BeanShell-Rust is a Rust rewrite of the [BeanShell](https://github.com/beanshell/beanshell)
interpreter. The lexer, parser, value model and evaluator are native Rust; Java
interop is provided by an embedded JVM reached over JNI through a small Java
reflection bridge.

**English** | [简体中文](README.zh-CN.md)

---

## Table of contents

- [Highlights](#highlights)
- [Project layout](#project-layout)
- [Requirements](#requirements)
- [Build](#build)
- [Usage](#usage)
- [JVM discovery](#jvm-discovery)
- [Testing](#testing)
- [Android](#android)
- [Documentation](#documentation)
- [Contributing](#contributing)
- [License](#license)
- [Acknowledgements](#acknowledgements)

## Highlights

- Full BeanShell syntax: statements, expressions, method definitions, closures,
  exceptions, arrays, enhanced for, switch statements and expressions,
  try-with-resources, lambdas, records, sealed types, instanceof patterns, text
  blocks and string templates.
- Java interop: `new`, static and instance members, varargs, boxing and proxy
  objects, plus scripted classes that extend Java classes.
- Library-friendly: `bsh-runtime` embeds cleanly; `bsh-rs` exposes Android
  JNI entry points as a shared library.
- Behaviour is checked against the Java implementation with a differential
  runner.

## Project layout

| Crate | Purpose |
| --- | --- |
| `crates/bsh-syntax` | Lexer, parser and AST |
| `crates/bsh-runtime` | Value model, evaluator, scopes and builtins |
| `crates/bsh-jvm` | Embedded JVM over JNI and the Java reflection bridge |
| `crates/bsh-cli` | The `bsh` command line |
| `crates/bsh-rs` | Android JNI bindings (cdylib) |

Supporting directories:

| Path | Contents |
| --- | --- |
| `tests/corpus` | Syntax and semantics cases |
| `tests/upstream-scripts` | Scripts taken from upstream BeanShell |
| `tools/` | Bridge rebuild, Android linker wrapper and the differential runner |
| `docs/` | Project artwork |

## Requirements

- Rust 1.75 or newer
- A JDK 8 or newer for Java interop (anything that ships `libjvm`)

## Build

```sh
cargo build --release
```

The Java bridge is compiled by `crates/bsh-jvm/build.rs` with `javac` and embedded
into the Rust library, so no class files are stored in the repository.

## Usage

```sh
# Run a script file
cargo run --release --bin bsh -- script.bsh

# Evaluate one line
cargo run --release --bin bsh -- -e 'print(new java.math.BigInteger("12").add(new java.math.BigInteger("30")));'

# Read a script from standard input
echo 'print("hi");' | cargo run --release --bin bsh

# Start a REPL
cargo run --release --bin bsh
```

`run.sh` builds on demand and then runs a script with the same arguments.

## JVM discovery

`libjvm` is resolved in this order:

1. the `BSH_RS_LIBJVM` environment variable
2. `JAVA_HOME/lib/server/libjvm.so`
3. the `java.home` reported by `java -XshowSettings:properties`
4. common locations such as `/usr/lib/jvm/*`

Without a JVM the interpreter still starts and exposes only the built-in host
classes. Pass `--no-jvm` or set `BSH_RS_JVM=0` to disable the JVM explicitly.

## Testing

```sh
cargo test --workspace
```

Differential testing against the Java implementation needs a runnable BeanShell:

```sh
BSH_ORACLE=/path/to/beanshell/run.sh tools/oracle-diff.py
```

Without `BSH_ORACLE` only the Rust side runs.

## Android

Android artifacts are published as an AAR built by AGP, bundling the JNI
libraries for `arm64-v8a`, `armeabi-v7a` and `x86_64`:

```text
bsh-rs-<version>.aar
  AndroidManifest.xml
  classes.jar
  R.txt
  META-INF/com/android/build/gradle/aar-metadata.properties
  jni/arm64-v8a/libbsh_rs.so
  jni/armeabi-v7a/libbsh_rs.so
  jni/x86_64/libbsh_rs.so
```

Consume it from Gradle with `implementation(files("libs/bsh-rs.aar"))`.
The library exports JNI entry points named `Java_me_yun_lspilot_bsh_Native_*`.

The AAR is assembled from `android/`, a small AGP library module that packages
the prebuilt `libbsh_rs.so` files. To build it locally:

```sh
# 1. build the JNI library for the ABI you need
ANDROID_NDK_HOME=/path/to/ndk \
  cargo build --release --target aarch64-linux-android -p bsh-rs

# 2. stage it where AGP expects it
mkdir -p android/bsh-rs/src/main/jniLibs/arm64-v8a
cp target/aarch64-linux-android/release/libbsh_rs.so \
   android/bsh-rs/src/main/jniLibs/arm64-v8a/

# 3. assemble the AAR
cd android && ./gradlew :bsh-rs:assembleRelease
```

rustc emits AArch64 object files itself, so `tools/android-linker.sh` only links:
it translates rustc's flags and invokes the NDK `ld.lld` directly, adding the
16 KB page-size flags required by Android 15 and later. On aarch64 hosts the
x86_64 `ld.lld` is run through `qemu-x86_64-static` when available. The wrapper
is selected by `.cargo/config.toml` and needs `ANDROID_NDK_HOME`.

### Sample app

`android/app` is a small demo and smoke-test app with the package name
`me.yun.bsh`. It depends on the AAR module and runs whatever script is in the
text field.

```sh
adb install -r bsh-demo-<version>.apk

# run a script without opening the UI
adb shell am broadcast -n me.yun.bsh/.RunReceiver -a me.yun.bsh.RUN \
  --es code 'print("demo ok " + (40 + 2));'

adb logcat -s bsh-rs
```

### Java API

The AAR ships a small Java API in `classes.jar`:

```text
me.yun.bsh.rust.BshEngine   one interpreter instance per object
me.yun.bsh.rust.Native      JNI bindings (package private)
bshrs.Bridge                Java reflection bridge used by the runtime
bshrs.Native                proxy callback entry point
```

```kotlin
val engine = BshEngine()
engine.importClass("java.util.ArrayList")
val list = engine.evalValue("new java.util.ArrayList()")
engine.setVariable("answer", 42)
engine.close()
```

Consumer ProGuard rules are bundled as `proguard.txt`, so native method names
survive shrinking.

## Documentation

- [Changelog](CHANGELOG.md) — entries match the repository tags
- [Releases](https://github.com/Lspilot/BeanShell-Rust/releases) — the current
  version is whatever the latest tag says
- [License](LICENSE) and [NOTICE](NOTICE)

## Contributing

Issues and pull requests are welcome. Please run `cargo fmt` and
`cargo test --workspace` before opening a pull request; CI checks formatting,
builds in release mode and runs the full test suite on Linux and macOS.

## License

Licensed under the Apache License, Version 2.0. See [LICENSE](LICENSE) and
[NOTICE](NOTICE) for details.

## Acknowledgements

BeanShell was created by Patrick Niemeyer and later donated to the Apache
Software Foundation. This project derives its language semantics, test scripts
and parts of its design from the upstream Java implementation.
