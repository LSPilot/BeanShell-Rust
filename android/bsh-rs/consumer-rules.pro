# Keep the JNI bindings; names must match the Rust exports.
-keep class me.yun.bsh.rust.Native { *; }
-keep class me.yun.bsh.rust.BshEngine { public *; }
-keepclasseswithmembernames class * {
    native <methods>;
}

# The runtime resolves these classes by name from native code.
-keep class bshrs.** { *; }
