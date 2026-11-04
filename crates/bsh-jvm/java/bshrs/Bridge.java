package bshrs;

import java.lang.reflect.Array;
import java.lang.reflect.Constructor;
import java.lang.reflect.Field;
import java.lang.reflect.InvocationHandler;
import java.lang.reflect.InvocationTargetException;
import java.lang.reflect.Method;
import java.lang.reflect.Modifier;
import java.lang.reflect.Proxy;
import java.util.ArrayList;
import java.util.HashMap;
import java.util.HashSet;
import java.util.List;
import java.util.Map;
import java.util.Set;

/**
 * Reflection bridge used by the Rust BeanShell interpreter.
 *
 * Every entry point takes and returns plain Objects so that the Rust side only
 * needs the JNI calls for Object arguments and Object results.
 */
public final class Bridge {
    private Bridge() {}

    /** Android: capture callback errors from worker threads instead of
     *  letting an uncaught exception kill the process. */
    public static volatile boolean catchCallbackErrors = false;
    private static final Map<Long, Throwable> CALLBACK_ERRORS =
            new java.util.concurrent.ConcurrentHashMap<Long, Throwable>();

    /** Extra class loaders registered by the host (LSPilot plugins). */
    private static volatile ClassLoader[] scriptLoaders = new ClassLoader[0];

    public static synchronized void addScriptLoader(ClassLoader loader) {
        if (loader == null) {
            return;
        }
        ClassLoader[] current = scriptLoaders;
        for (ClassLoader existing : current) {
            if (existing == loader) {
                return;
            }
        }
        ClassLoader[] next = new ClassLoader[current.length + 1];
        System.arraycopy(current, 0, next, 0, current.length);
        next[current.length] = loader;
        scriptLoaders = next;
    }

    /** Drain callback errors collected on worker threads as one message. */
    public static String drainErrors() {
        if (CALLBACK_ERRORS.isEmpty()) {
            return "";
        }
        StringBuilder out = new StringBuilder();
        for (Map.Entry<Long, Throwable> entry : CALLBACK_ERRORS.entrySet()) {
            Throwable error = entry.getValue();
            String message = error.getMessage();
            if (message == null || message.isEmpty()) {
                message = error.toString();
            }
            if (out.length() > 0) {
                out.append('\n');
            }
            out.append("Exception in thread: ").append(message);
        }
        CALLBACK_ERRORS.clear();
        return out.toString();
    }

    // ------------------------------------------------------------- classes

    static Class<?> resolve(String name) throws ClassNotFoundException {
        switch (name) {
            case "boolean": return boolean.class;
            case "byte": return byte.class;
            case "char": return char.class;
            case "short": return short.class;
            case "int": return int.class;
            case "long": return long.class;
            case "float": return float.class;
            case "double": return double.class;
            case "void": return void.class;
            default: break;
        }
        if (name.endsWith("[]")) {
            Class<?> base = resolve(name.substring(0, name.length() - 2));
            return Array.newInstance(base, 0).getClass();
        }
        for (ClassLoader scriptLoader : scriptLoaders) {
            Class<?> found = tryResolve(name, scriptLoader);
            if (found != null) {
                return found;
            }
        }
        ClassLoader loader = Thread.currentThread().getContextClassLoader();
        Class<?> found = tryResolve(name, loader);
        if (found != null) {
            return found;
        }
        found = tryResolve(name, Bridge.class.getClassLoader());
        if (found != null) {
            return found;
        }
        return Class.forName(name);
    }

    /** Also accepts nested classes written as a.b.Outer.Inner. */
    private static Class<?> tryResolve(String name, ClassLoader loader) {
        String candidate = name;
        while (true) {
            try {
                return Class.forName(candidate, false, loader);
            } catch (ClassNotFoundException ignored) {
                // try the nested form
            } catch (LinkageError ignored) {
                // try the nested form
            }
            int separator = candidate.lastIndexOf('.');
            if (separator < 0) {
                return null;
            }
            candidate = candidate.substring(0, separator) + '$' + candidate.substring(separator + 1);
        }
    }

    static Object classFor(String name) throws ClassNotFoundException {
        return resolve(name);
    }

    static boolean classExists(String name) {
        try {
            resolve(name);
            return true;
        } catch (Throwable t) {
            return false;
        }
    }

    static String className(Object o) {
        return o.getClass().getName();
    }

    // ---------------------------------------------------------- box/unbox

    static Object box(String kind, String text) {
        switch (kind) {
            case "string": return text;
            case "null": return null;
            case "boolean": return Boolean.valueOf(Boolean.parseBoolean(text));
            case "char": return Character.valueOf(text.isEmpty() ? (char) 0 : text.charAt(0));
            case "byte": return Byte.valueOf(text);
            case "short": return Short.valueOf(text);
            case "int": return Integer.valueOf(text);
            case "long": return Long.valueOf(text);
            case "float": return Float.valueOf(text);
            case "double": return Double.valueOf(text);
            default: throw new IllegalArgumentException("unknown kind: " + kind);
        }
    }

    static int tag(Object o) {
        if (o == null) return 0;
        if (o instanceof Boolean) return 1;
        if (o instanceof Character) return 2;
        if (o instanceof Byte) return 3;
        if (o instanceof Short) return 4;
        if (o instanceof Integer) return 5;
        if (o instanceof Long) return 6;
        if (o instanceof Float) return 7;
        if (o instanceof Double) return 8;
        if (o instanceof String) return 9;
        return 10;
    }

    static boolean asBool(Object o) { return ((Boolean) o).booleanValue(); }
    static char asChar(Object o) { return ((Character) o).charValue(); }
    static long asLong(Object o) { return ((Number) o).longValue(); }
    static double asDouble(Object o) { return ((Number) o).doubleValue(); }
    static String asString(Object o) { return (String) o; }

    // -------------------------------------------------------------- calls

    static Object newInstance(String cn, Object[] args) throws Throwable {
        Class<?> c = resolve(cn);
        if (c.isInterface() || Modifier.isAbstract(c.getModifiers())) {
            throw new InstantiationException("Cannot instantiate " + cn);
        }
        Constructor<?> ctor = selectCtor(c, args);
        Object[] callArgs = normalizeVarargs(ctor.isVarArgs(), ctor.getParameterTypes(), args);
        try {
            return ctor.newInstance(callArgs);
        } catch (InvocationTargetException e) {
            throw e.getCause();
        }
    }

    static Object call(Object target, String name, Object[] args) throws Throwable {
        if (target == null) {
            throw new NullPointerException("Cannot invoke " + name + "() on null");
        }
        Class<?> c = target.getClass();
        Method m = selectMethod(c, name, args, false);
        if (m == null) {
            // Bean style property read fallback.
            String cap = Character.toUpperCase(name.charAt(0)) + name.substring(1);
            m = selectMethod(c, "get" + cap, args, false);
            if (m == null) m = selectMethod(c, "is" + cap, args, false);
        }
        if (m == null) {
            throw new NoSuchMethodException(c.getName() + "." + name + "/" + args.length);
        }
        Object[] callArgs = normalizeVarargs(m.isVarArgs(), m.getParameterTypes(), args);
        try {
            return m.invoke(target, callArgs);
        } catch (InvocationTargetException e) {
            throw e.getCause();
        } catch (IllegalAccessException e) {
            // Hidden implementation classes (e.g. lambdas) are not accessible
            // reflectively; retry through a public interface declaration.
            Method viaInterface = interfaceMethod(c, name, args.length);
            if (viaInterface == null) throw e;
            try {
                return viaInterface.invoke(target, args);
            } catch (InvocationTargetException ite) {
                throw ite.getCause();
            }
        }
    }

    static Method interfaceMethod(Class<?> c, String name, int argc) {
        for (Class<?> iface : c.getInterfaces()) {
            Method m = publicMethod(iface, name, argc);
            if (m != null) return m;
            Method nested = interfaceMethod(iface, name, argc);
            if (nested != null) return nested;
        }
        if (c.getSuperclass() != null) {
            return interfaceMethod(c.getSuperclass(), name, argc);
        }
        return null;
    }

    static Method publicMethod(Class<?> c, String name, int argc) {
        if (!Modifier.isPublic(c.getModifiers())) return null;
        for (Method m : c.getDeclaredMethods()) {
            if (m.getName().equals(name) && m.getParameterCount() == argc
                    && Modifier.isPublic(m.getModifiers())) {
                return m;
            }
        }
        return null;
    }

    static Object callStatic(String cn, String name, Object[] args) throws Throwable {
        Class<?> c = resolve(cn);
        Method m = selectMethod(c, name, args, true);
        if (m == null) {
            throw new NoSuchMethodException(cn + "." + name + "/" + args.length);
        }
        Object[] callArgs = normalizeVarargs(m.isVarArgs(), m.getParameterTypes(), args);
        try {
            return m.invoke(null, callArgs);
        } catch (InvocationTargetException e) {
            throw e.getCause();
        }
    }

    static Object get(Object target, String name) throws Throwable {
        if (target == null) throw new NullPointerException("field " + name + " of null");
        return getFieldValue(target.getClass(), target, name);
    }

    static void set(Object target, String name, Object value) throws Throwable {
        if (target == null) throw new NullPointerException("field " + name + " of null");
        setFieldValue(target.getClass(), target, name, value);
    }

    static Object getStatic(String cn, String name) throws Throwable {
        Class<?> c = resolve(cn);
        Field f = findField(c, name);
        if (f != null) {
            f.setAccessible(true);
            return f.get(null);
        }
        Method m = selectMethod(c, name, new Object[0], true);
        if (m != null) {
            try { return m.invoke(null); }
            catch (InvocationTargetException e) { throw e.getCause(); }
        }
        throw new NoSuchFieldException(cn + "." + name);
    }

    static void setStatic(String cn, String name, Object value) throws Throwable {
        Class<?> c = resolve(cn);
        Field f = findField(c, name);
        if (f == null) throw new NoSuchFieldException(cn + "." + name);
        f.setAccessible(true);
        f.set(null, value);
    }

    static Field findField(Class<?> c, String name) {
        for (Class<?> k = c; k != null; k = k.getSuperclass()) {
            try {
                return k.getDeclaredField(name);
            } catch (NoSuchFieldException ignored) {
                // continue with the superclass
            }
        }
        return null;
    }

    static Object getFieldValue(Class<?> c, Object target, String name) throws Throwable {
        Field f = findField(c, name);
        if (f != null) {
            f.setAccessible(true);
            return f.get(target);
        }
        String cap = Character.toUpperCase(name.charAt(0)) + name.substring(1);
        Method m = selectMethod(c, "get" + cap, new Object[0], false);
        if (m == null) m = selectMethod(c, "is" + cap, new Object[0], false);
        if (m == null) throw new NoSuchFieldException(c.getName() + "." + name);
        try { return m.invoke(target); }
        catch (InvocationTargetException e) { throw e.getCause(); }
    }

    static void setFieldValue(Class<?> c, Object target, String name, Object value) throws Throwable {
        Field f = findField(c, name);
        if (f != null) {
            f.setAccessible(true);
            f.set(target, value);
            return;
        }
        String cap = Character.toUpperCase(name.charAt(0)) + name.substring(1);
        Method m = selectMethod(c, "set" + cap, new Object[] { value }, false);
        if (m == null) throw new NoSuchFieldException(c.getName() + "." + name);
        try { m.invoke(target, value); }
        catch (InvocationTargetException e) { throw e.getCause(); }
    }

    // ------------------------------------------------------------- arrays

    static int length(Object array) {
        return Array.getLength(array);
    }

    static Object index(Object array, int i) {
        return Array.get(array, i);
    }

    static void setIndex(Object array, int i, Object value) {
        Array.set(array, i, value);
    }

    static Object newArray(String component, int length) throws Throwable {
        return Array.newInstance(resolve(component), length);
    }

    static boolean isInstance(Object o, String cn) {
        try {
            return resolve(cn).isInstance(o);
        } catch (Throwable t) {
            return false;
        }
    }

    static String describe(Throwable t) {
        String msg = t.getMessage();
        return t.getClass().getName() + (msg == null ? "" : ": " + msg);
    }

    // ------------------------------------------------------------- proxies

    private static Object invokeDefault(Object proxy, Method method, Object[] args) throws Throwable {
        try {
            Method invokeDefault = InvocationHandler.class.getMethod(
                    "invokeDefault", Object.class, Method.class, Object[].class);
            return invokeDefault.invoke(null, proxy, method, args);
        } catch (InvocationTargetException e) {
            throw e.getCause();
        } catch (NoSuchMethodException e) {
            throw new UnsupportedOperationException(
                    "interface default methods require Android 13 (API 33)");
        }
    }

    static boolean isInterface(String name) {
        try {
            return resolve(name).isInterface();
        } catch (Throwable t) {
            return false;
        }
    }

    /**
     * Create a dynamic proxy implementing the given interfaces; every call is
     * forwarded to the Rust callback registered for the handle.
     */
    static Object proxy(String[] ifaces, long handle) throws Throwable {
        Class<?>[] types = new Class<?>[ifaces.length];
        for (int i = 0; i < ifaces.length; i++) {
            types[i] = resolve(ifaces[i]);
        }
        ClassLoader loader = proxyLoader(types);
        return Proxy.newProxyInstance(loader, types, new Handler(handle));
    }

    /** Interfaces may come from LSPosed module loaders; the thread context
     *  loader often cannot see them, so prefer the interface's own loader. */
    private static ClassLoader proxyLoader(Class<?>[] types) {
        List<ClassLoader> candidates = new ArrayList<ClassLoader>();
        for (Class<?> type : types) {
            ClassLoader loader = type.getClassLoader();
            if (loader != null && !candidates.contains(loader)) {
                candidates.add(loader);
            }
        }
        if (candidates.isEmpty()) {
            ClassLoader context = Thread.currentThread().getContextClassLoader();
            return context != null ? context : Bridge.class.getClassLoader();
        }
        outer:
        for (ClassLoader candidate : candidates) {
            for (Class<?> type : types) {
                try {
                    if (Class.forName(type.getName(), false, candidate) != type) {
                        continue outer;
                    }
                } catch (Throwable ignored) {
                    continue outer;
                }
            }
            return candidate;
        }
        return candidates.get(0);
    }

    static final class Handler implements InvocationHandler {
        private final long handle;
        private final Thread owner;

        Handler(long handle) {
            this.handle = handle;
            this.owner = Thread.currentThread();
        }

        public Object invoke(Object proxy, Method method, Object[] args) throws Throwable {
            String name = method.getName();
            int argc = args == null ? 0 : args.length;
            if (argc == 0 && "toString".equals(name)) return "bsh-proxy-" + handle;
            if (argc == 0 && "hashCode".equals(name)) return (int) handle;
            if (argc == 1 && "equals".equals(name)) return proxy == args[0];
            if (method.isDefault()) {
                return invokeDefault(proxy, method, args);
            }
            try {
                return bshrs.Native.invoke(handle, name, args);
            } catch (Throwable error) {
                if (Thread.currentThread() == owner || !catchCallbackErrors) {
                    throw error;
                }
                CALLBACK_ERRORS.put(handle, error);
                return defaultValue(method.getReturnType());
            }
        }

        private static Object defaultValue(Class<?> type) {
            if (!type.isPrimitive() || type == void.class) return null;
            if (type == boolean.class) return false;
            if (type == char.class) return (char) 0;
            if (type == byte.class) return (byte) 0;
            if (type == short.class) return (short) 0;
            if (type == int.class) return 0;
            if (type == long.class) return 0L;
            if (type == float.class) return 0f;
            return 0d;
        }
    }

    /** Parameter type names of the method that would be selected. */
    static String[] methodParams(Object target, String className, String name, int argc, boolean isStatic)
            throws Throwable {
        Class<?> c = target == null ? resolve(className) : target.getClass();
        Method m = selectMethod(c, name, new Object[argc], isStatic);
        if (m == null) return null;
        return typeNames(m.getParameterTypes());
    }

    static String[] ctorParams(String className, int argc) throws Throwable {
        Class<?> c = resolve(className);
        Constructor<?> ctor = selectCtor(c, new Object[argc]);
        return typeNames(ctor.getParameterTypes());
    }

    static String[] typeNames(Class<?>[] types) {
        String[] out = new String[types.length];
        for (int i = 0; i < types.length; i++) {
            out[i] = types[i].getTypeName();
        }
        return out;
    }

    /** Invoked from Rust through JNI. */
    public static native Object invoke(long handle, String method, Object[] args);

    // -------------------------------------------------- generated subclasses

    /** Loader for classes whose bytecode is generated by the Rust side. */
    static final class ScriptLoader extends ClassLoader {
        ScriptLoader(ClassLoader parent) {
            super(parent);
        }

        Class<?> define(String name, byte[] bytes) {
            return defineClass(name, bytes, 0, bytes.length);
        }
    }

    private static final ScriptLoader SCRIPT_LOADER =
            new ScriptLoader(Bridge.class.getClassLoader());

    static Class<?> defineScriptClass(String name, byte[] bytes) {
        return SCRIPT_LOADER.define(name, bytes);
    }

    static Object newScriptInstance(Class<?> c, long handle) throws Throwable {
        Constructor<?> ctor = c.getDeclaredConstructor(long.class);
        ctor.setAccessible(true);
        try {
            return ctor.newInstance(handle);
        } catch (InvocationTargetException e) {
            throw e.getCause();
        }
    }

    /**
     * Encode the overridable methods of a class as one line per method:
     * name|returnType|paramType|...
     */
    static String overridableMethods(String className) throws Throwable {
        Class<?> c = resolve(className);
        StringBuilder sb = new StringBuilder();
        Set<String> seen = new HashSet<String>();
        for (Class<?> k = c; k != null; k = k.getSuperclass()) {
            for (Method m : k.getDeclaredMethods()) {
                int mod = m.getModifiers();
                if (Modifier.isStatic(mod) || Modifier.isPrivate(mod) || Modifier.isFinal(mod)) continue;
                if (!Modifier.isPublic(mod) && !Modifier.isProtected(mod)) continue;
                if (m.isSynthetic() || m.isBridge()) continue;
                String key = m.getName() + "/" + m.getParameterCount();
                if (!seen.add(key)) continue;
                sb.append(m.getName()).append('|').append(m.getReturnType().getTypeName());
                for (Class<?> p : m.getParameterTypes()) {
                    sb.append('|').append(p.getTypeName());
                }
                sb.append('\n');
            }
        }
        return sb.toString();
    }

    static String typeName(Object o) {
        if (o == null) return "null";
        Class<?> c = o.getClass();
        if (c.isArray()) {
            return c.getComponentType().getTypeName() + "[]";
        }
        return c.getTypeName();
    }

    // ------------------------------------------------------------ overloads

    static Constructor<?> selectCtor(Class<?> c, Object[] args) throws NoSuchMethodException {
        Constructor<?> best = null;
        int bestScore = -1;
        for (Constructor<?> ctor : c.getDeclaredConstructors()) {
            if (!matchesArity(ctor.isVarArgs(), ctor.getParameterCount(), args.length)) continue;
            int score = ctor.isVarArgs()
                    ? scoreVarargs(ctor.getParameterTypes(), args)
                    : score(ctor.getParameterTypes(), args);
            if (score > bestScore) {
                bestScore = score;
                best = ctor;
            }
        }
        if (best == null) {
            throw new NoSuchMethodException(c.getName() + "." + "<init>" + "/" + args.length);
        }
        try {
            best.setAccessible(true);
        } catch (Throwable ignored) {
            // public members of exported packages do not need it
        }
        return best;
    }

    static Method selectMethod(Class<?> c, String name, Object[] args, boolean staticOnly) {
        Method best = null;
        int bestScore = -1;
        List<Method> candidates = new ArrayList<Method>();
        for (Method m : c.getMethods()) {
            if (!m.getName().equals(name)) continue;
            if (staticOnly && !Modifier.isStatic(m.getModifiers())) continue;
            if (!staticOnly && Modifier.isStatic(m.getModifiers()) && !m.getDeclaringClass().isInterface()) {
                // allow static methods through an instance only when nothing else matches
            }
            if (!matchesArity(m.isVarArgs(), m.getParameterCount(), args.length)) continue;
            candidates.add(m);
        }
        for (Class<?> k = c; k != null && k != Object.class; k = k.getSuperclass()) {
            for (Method m : k.getDeclaredMethods()) {
                if (!m.getName().equals(name)) continue;
                if (staticOnly && !Modifier.isStatic(m.getModifiers())) continue;
                if (!matchesArity(m.isVarArgs(), m.getParameterCount(), args.length)) continue;
                candidates.add(m);
            }
        }
        for (Method m : candidates) {
            int score = m.isVarArgs()
                    ? scoreVarargs(m.getParameterTypes(), args)
                    : score(m.getParameterTypes(), args);
            if (score > bestScore) {
                bestScore = score;
                best = m;
            }
        }
        if (best != null) {
            try {
                best.setAccessible(true);
            } catch (Throwable ignored) {
                // public members of exported packages do not need it
            }
        }
        return best;
    }

    static Object[] normalizeVarargs(boolean varargs, Class<?>[] params, Object[] args) {
        if (!varargs || params.length == 0) return args;
        int fixed = params.length - 1;
        Class<?> arrayType = params[fixed];
        if (args.length == params.length && args[fixed] != null
                && arrayType.isInstance(args[fixed])) {
            return args;
        }
        if (args.length < fixed) return args;
        Class<?> component = arrayType.getComponentType();
        Object rest = Array.newInstance(component, args.length - fixed);
        for (int i = fixed; i < args.length; i++) {
            Array.set(rest, i - fixed, args[i]);
        }
        Object[] out = new Object[params.length];
        System.arraycopy(args, 0, out, 0, fixed);
        out[fixed] = rest;
        return out;
    }

    static boolean matchesArity(boolean varargs, int paramCount, int argc) {
        if (paramCount == argc) return true;
        return varargs && argc >= paramCount - 1;
    }

    static int scoreVarargs(Class<?>[] params, Object[] args) {
        int fixed = params.length - 1;
        Class<?> component = params[fixed].getComponentType();
        int score = 0;
        for (int i = 0; i < fixed; i++) {
            score += scoreOne(params[i], args[i]);
        }
        for (int i = fixed; i < args.length; i++) {
            score += scoreOne(component, args[i]);
        }
        return score;
    }

    static int scoreOne(Class<?> p, Object a) {
        if (a == null) {
            return p.isPrimitive() ? 0 : 1;
        }
        Class<?> ac = a.getClass();
        if (p.isAssignableFrom(ac)) {
            return 3;
        }
        if (p.isPrimitive()) {
            if ((p == int.class && (a instanceof Integer || a instanceof Byte
                    || a instanceof Short || a instanceof Character))
                    || (p == long.class && (a instanceof Long || a instanceof Integer
                    || a instanceof Byte || a instanceof Short || a instanceof Character))
                    || (p == double.class && a instanceof Number)
                    || (p == float.class && a instanceof Number)
                    || (p == short.class && (a instanceof Short || a instanceof Byte))
                    || (p == byte.class && a instanceof Byte)
                    || (p == char.class && a instanceof Character)
                    || (p == boolean.class && a instanceof Boolean)) {
                return 2;
            }
            return -1;
        }
        return 0;
    }

    static int score(Class<?>[] params, Object[] args) {
        int score = 0;
        for (int i = 0; i < params.length; i++) {
            Class<?> p = params[i];
            Object a = args[i];
            if (a == null) {
                score += p.isPrimitive() ? 0 : 1;
                continue;
            }
            Class<?> ac = a.getClass();
            if (p.isAssignableFrom(ac)) {
                score += 3;
            } else if (p.isPrimitive()) {
                if ((p == int.class && (a instanceof Integer || a instanceof Byte
                        || a instanceof Short || a instanceof Character))
                        || (p == long.class && (a instanceof Long || a instanceof Integer
                        || a instanceof Byte || a instanceof Short || a instanceof Character))
                        || (p == double.class && a instanceof Number)
                        || (p == float.class && a instanceof Number)
                        || (p == short.class && (a instanceof Short || a instanceof Byte))
                        || (p == byte.class && a instanceof Byte)
                        || (p == char.class && a instanceof Character)
                        || (p == boolean.class && a instanceof Boolean)) {
                    score += 2;
                } else {
                    score -= 1;
                }
            }
        }
        return score;
    }

    // Compatibility shims for the Rust JNI table, which passes a runtime id
    // to every bridge call. The legacy bridge is process-global.
    static Object classForEngine(long runtime, String name) throws Throwable { return classFor(name); }
    static boolean classExistsEngine(long runtime, String name) { return classExists(name); }
    static Object newInstanceEngine(long runtime, String cn, Object[] args) throws Throwable { return newInstance(cn, args); }
    static Object callStaticEngine(long runtime, String cn, String name, Object[] args) throws Throwable { return callStatic(cn, name, args); }
    static Object getStaticEngine(long runtime, String cn, String name) throws Throwable { return getStatic(cn, name); }
    static void setStaticEngine(long runtime, String cn, String name, Object value) throws Throwable { setStatic(cn, name, value); }
    static Object newArrayEngine(long runtime, String component, int length) throws Throwable { return newArray(component, length); }
    static boolean isInstanceEngine(long runtime, Object value, String name) { return isInstance(value, name); }
    static boolean isInterfaceEngine(long runtime, String name) { return isInterface(name); }
    static Object proxyEngine(long runtime, String[] ifaces, long handle) throws Throwable { return proxy(ifaces, handle); }
    static String[] methodParamsEngine(long runtime, Object target, String cn, String name, int argc, boolean isStatic) throws Throwable {
        return methodParams(target, cn, name, argc, isStatic);
    }
    static String[] ctorParamsEngine(long runtime, String cn, int argc) throws Throwable {
        return ctorParams(cn, argc);
    }
    static String[][] methodOverloadsEngine(long runtime, Object target, String cn, String name, int argc, boolean isStatic) throws Throwable {
        String[] one = methodParams(target, cn, name, argc, isStatic);
        return one == null ? new String[0][] : new String[][] { one };
    }
    static String[] samInfoEngine(long runtime, String cn) {
        return new String[0];
    }
}
