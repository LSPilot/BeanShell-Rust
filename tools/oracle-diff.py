#!/usr/bin/env python3
"""Differential test: run BeanShell scripts through the Java oracle and the
Rust interpreter, then compare the observable behaviour.

If the Java oracle reports an error (it prints a line starting with __ERROR__)
the Rust run is only required to fail as well; otherwise the standard output
must match exactly.

Usage: tools/oracle-diff.py [corpus-dir]
"""
import difflib
import glob
import os
import shutil
import subprocess
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.dirname(HERE)
if not ORACLE:
    sys.exit("Set BSH_ORACLE to the Java BeanShell launcher (or put beanshell/bsh on PATH)")

CORPUS = sys.argv[1] if len(sys.argv) > 1 else os.path.join(ROOT, "tests", "corpus")
ORACLE = os.environ.get("BSH_ORACLE") or shutil.which("beanshell") or shutil.which("bsh")
RUST = os.path.join(ROOT, "target", "release", "bsh")


ORACLE_ENV = dict(os.environ)
ORACLE_ENV.setdefault(
    "JAVA_TOOL_OPTIONS",
    " ".join(
        "--add-opens=%s=ALL-UNNAMED" % pkg
        for pkg in [
            "java.base/java.lang",
            "java.base/java.lang.reflect",
            "java.base/java.util",
            "java.base/java.math",
            "java.base/java.io",
            "java.base/java.time",
            "java.desktop/java.awt",
            "java.desktop/java.awt.geom",
            "java.desktop/java.awt.event",
            "java.base/java.util.concurrent",
            "jdk.proxy1/jdk.proxy1",
            "jdk.proxy2/jdk.proxy2",
            "jdk.proxy3/jdk.proxy3",
        ]
    ),
)


def run(cmd, src, env=None):
    p = subprocess.run(cmd, input=src, capture_output=True, text=True, timeout=120, env=env)
    return p.stdout, p.stderr, p.returncode


def main():
    files = sorted(glob.glob(os.path.join(CORPUS, "*.bsh")))
    same = err_same = diff = 0
    for path in files:
        src = open(path, encoding="utf-8").read()
        jout, jerr, _ = run([ORACLE], src, ORACLE_ENV)
        rout, rerr, rcode = run([RUST], src)
        name = os.path.basename(path)
        java_error = "__ERROR__" in jout
        rust_error = rcode != 0 or bool(rerr.strip())
        if java_error:
            if rust_error:
                err_same += 1
                print("SAME-ERROR", name)
            else:
                diff += 1
                print("DIFF-ERROR", name, "(java failed, rust succeeded)")
                print("   rust stdout:", rout.strip()[:300])
            continue
        if jout == rout:
            same += 1
            print("SAME", name)
        else:
            diff += 1
            print("DIFF", name)
            for line in list(difflib.unified_diff(
                    jout.splitlines(), rout.splitlines(),
                    fromfile="java", tofile="rust", lineterm=""))[:40]:
                print("   " + line)
            if rerr.strip():
                print("   rust stderr:", rerr.strip()[:400])
    print("----------------------------------------")
    print("same=%d same-error=%d diff=%d" % (same, err_same, diff))
    return 0 if diff == 0 else 1


if __name__ == "__main__":
    sys.exit(main())
