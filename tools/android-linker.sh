#!/bin/sh
# Linker for aarch64-linux-android.
#
# rustc already emits AArch64 object files, so only linking is needed. Calling
# the NDK clang driver is both unnecessary and awkward on non-x86_64 hosts, so
# this script translates rustc's arguments and invokes ld.lld directly.
#
# Set ANDROID_NDK_HOME (or ANDROID_NDK_ROOT). On aarch64 hosts the NDK's
# x86_64 ld.lld is executed through qemu-x86_64-static.
set -e

NDK="${ANDROID_NDK_HOME:-${ANDROID_NDK_ROOT:-}}"
if [ -z "$NDK" ]; then
    for candidate in /opt/android-sdk/ndk/*; do
        [ -d "$candidate" ] && NDK="$candidate"
    done
fi
if [ -z "$NDK" ]; then
    echo "android-linker: set ANDROID_NDK_HOME to your NDK" >&2
    exit 1
fi

HOST_TAG=linux-x86_64
case "$(uname -s)" in
    Darwin) HOST_TAG=darwin-x86_64 ;;
esac

LLD="$NDK/toolchains/llvm/prebuilt/$HOST_TAG/bin/ld.lld"
[ -x "$LLD" ] || { echo "android-linker: ld.lld not found in $NDK" >&2; exit 1; }

QEMU=""
case "$(uname -m)" in
    aarch64|arm64)
        if [ -x /usr/bin/qemu-x86_64-static ]; then
            QEMU="/usr/bin/qemu-x86_64-static -L /usr/x86_64-linux-gnu"
        fi
        ;;
esac

API=28
SYSROOT="$NDK/toolchains/llvm/prebuilt/$HOST_TAG/sysroot"
# libunwind and the sanitizer runtimes live next to the clang resource dir.
CLANG_LIB="$(ls -d "$NDK"/toolchains/llvm/prebuilt/"$HOST_TAG"/lib/clang/*/lib/linux/aarch64 2>/dev/null | sort -V | tail -n 1)"

set -- -m aarch64elf --build-id=none --eh-frame-hdr \
    -L "$SYSROOT/usr/lib/aarch64-linux-android/$API" \
    -L "$SYSROOT/usr/lib/aarch64-linux-android" \
    ${CLANG_LIB:+-L "$CLANG_LIB"} "$@"

# Translate -Wl,a,b into individual linker flags.
ARGS=""
while [ $# -gt 0 ]; do
    case "$1" in
        -Wl,*)
            for a in $(echo "${1#-Wl,}" | tr ',' ' '); do
                ARGS="$ARGS $a"
            done
            shift
            ;;
        -nodefaultlibs|-nostartfiles)
            shift
            ;;
        *)
            ARGS="$ARGS \"$1\""
            shift
            ;;
    esac
done

eval exec $QEMU "\"$LLD\"" $ARGS
