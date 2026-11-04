# Changelog

All notable changes to this project are documented here. Entries correspond to
repository tags; the current version is whatever the latest tag says.

The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/) and
the project uses [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

[Unreleased]: https://github.com/Lspilot/BeanShell-Rust/compare/v0.1.0...HEAD

## [0.1.0] - 2025-11-04

First public snapshot of the Rust implementation.

### Added

- `bsh-syntax`: lexer, parser and AST for the BeanShell language, including
  switch expressions and arrows, try-with-resources, lambdas, records, sealed
  types, instanceof patterns, text blocks and string templates.
- `bsh-runtime`: value model, evaluator, scopes, builtins and exception
  handling, with scripted classes that can extend Java classes.
- `bsh-jvm`: embedded JVM over JNI plus the `bshrs.Bridge` Java reflection
  bridge; bridge bytecode is generated at build time by `build.rs`.
- `bsh-cli`: the `bsh` command supporting script files, `-e`, stdin and a REPL.
- `bsh-lspilot`: Android JNI bindings built as `libbsh_lspilot.so`.
- Tests: language semantics, JVM interop, upstream BeanShell scripts and a
  differential runner against the Java implementation.

### Fixed

- Scripted subclasses of Java classes failed with "Cannot cast X to X" because
  the JVM-side class resolver cannot see Rust-defined classes.
- `catch (IllegalArgumentException)` incorrectly matched unrelated
  `RuntimeException` instances due to an over-broad `Throwable` fallback.

[0.1.0]: https://github.com/Lspilot/BeanShell-Rust/releases/tag/v0.1.0
