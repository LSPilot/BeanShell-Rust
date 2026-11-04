package me.yun.bsh.rust;

import java.io.Closeable;

/**
 * A BeanShell interpreter instance backed by the Rust implementation.
 *
 * <p>Instances are not thread safe; create one per thread or guard access.
 */
public final class BshEngine implements Closeable {
    private long handle;

    public BshEngine() {
        if (!Native.isLoaded()) {
            throw new IllegalStateException("libbsh_rs.so is not available for this ABI");
        }
        handle = Native.createEngine();
        if (handle == 0L) {
            throw new IllegalStateException("cannot create a BeanShell engine");
        }
    }

    /** Evaluates a script and returns everything it printed. */
    public String eval(String source) {
        return Native.eval(handle(), source);
    }

    /** Evaluates a script and returns its value. */
    public Object evalValue(String source) {
        return Native.evalValue(handle(), source);
    }

    /** Evaluates a script file and returns its value. */
    public Object evalFile(String path) {
        return Native.evalFile(handle(), path);
    }

    public void setVariable(String name, Object value) {
        Native.setVariable(handle(), name, value);
    }

    public Object getVariable(String name) {
        return Native.getVariable(handle(), name);
    }

    public void importClass(String name) {
        Native.importClass(handle(), name);
    }

    /**
     * Registers a script method so Java code can call it later.
     *
     * @param argc number of parameters the method takes
     * @param varargs whether the last parameter is variadic
     */
    public void defineMethod(String name, Object instance, String className, int argc, boolean varargs) {
        Native.defineMethod(handle(), name, instance, className, argc, varargs);
    }

    /** Returns the script methods as a newline separated report. */
    public String scriptMethods() {
        return Native.scriptMethods(handle());
    }

    public Object invokeMethod(String name, Object... args) {
        return Native.invokeMethod(handle(), name, args);
    }

    /** Clears the global namespace of this engine. */
    public void reset() {
        Native.reset(handle());
    }

    @Override
    public void close() {
        long h = handle;
        handle = 0L;
        if (h != 0L) {
            Native.destroyEngine(h);
        }
    }

    private long handle() {
        long h = handle;
        if (h == 0L) {
            throw new IllegalStateException("engine is closed");
        }
        return h;
    }
}
