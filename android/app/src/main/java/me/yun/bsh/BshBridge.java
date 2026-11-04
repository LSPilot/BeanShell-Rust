package me.yun.bsh;

import me.yun.bsh.rust.BshEngine;

/** Thin Java facade over the Rust interpreter shipped by the bsh-rs AAR. */
public final class BshBridge {
    private static final Object LOCK = new Object();
    private static BshEngine engine;

    private BshBridge() {
    }

    /** Evaluates a BeanShell script and returns everything it printed. */
    public static String nativeEval(String source) {
        synchronized (LOCK) {
            if (engine == null) {
                engine = new BshEngine();
            }
            String out = engine.eval(source);
            return out == null ? "" : out;
        }
    }

    /** Drops the interpreter and its global namespace. */
    public static void nativeReset() {
        synchronized (LOCK) {
            if (engine != null) {
                engine.close();
                engine = null;
            }
        }
    }
}
