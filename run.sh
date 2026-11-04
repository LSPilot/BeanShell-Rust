#!/bin/sh
# Run a BeanShell script with the Rust interpreter.
# Accepts the same shape as ../run.sh (the Java oracle) for differential use.
DIR="$(cd "$(dirname "$0")" && pwd)"
BIN="$DIR/target/release/bsh"
if [ ! -x "$BIN" ]; then
    ( cd "$DIR" && cargo build --release -q ) || exit 1
fi
if [ $# -gt 0 ]; then
    exec "$BIN" "$@"
fi
exec "$BIN"
