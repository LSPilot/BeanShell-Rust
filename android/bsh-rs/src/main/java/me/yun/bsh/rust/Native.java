package me.yun.bsh.rust;

/**
 * JNI entry points for the Rust BeanShell interpreter.
 *
 * <p>Each engine corresponds to one interpreter instance with its own global
 * namespace. Use {@link BshEngine} rather than calling these methods directly.
 */
final class Native {
    private static final boolean LOADED;

    static {
        boolean loaded;
        try {
            System.loadLibrary("bsh_rs");
            loaded = true;
        } catch (UnsatisfiedLinkError e) {
            loaded = false;
        }
        LOADED = loaded;
    }

    private Native() {
    }

    static boolean isLoaded() {
        return LOADED;
    }

    static native long createEngine();

    static native void destroyEngine(long engine);

    static native String eval(long engine, String source);

    static native Object evalValue(long engine, String source);

    static native Object evalFile(long engine, String path);

    static native void setVariable(long engine, String name, Object value);

    static native Object getVariable(long engine, String name);

    static native void importClass(long engine, String name);

    static native void defineMethod(
            long engine,
            String name,
            Object instance,
            String className,
            int argc,
            boolean varargs);

    static native String scriptMethods(long engine);

    static native Object invokeMethod(long engine, String name, Object[] args);

    static native void reset(long engine);
}
