package bshrs;

/**
 * Native entry points called from the Java reflection bridge. The Rust side
 * registers implementations with JNI RegisterNatives at JVM startup.
 */
public final class Native {
    private Native() {}

    /** Dispatch a proxy method call back into the Rust interpreter. */
    public static native Object invoke(long handle, String method, Object[] args);
}
