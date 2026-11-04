package me.yun.bsh;

import android.app.Application;
import android.util.Log;

/** Loads the native library early so the first run does not pay dlopen cost. */
public class BshApp extends Application {
    @Override
    public void onCreate() {
        super.onCreate();
        Thread loader = new Thread(() -> {
            try {
                System.loadLibrary("bsh_rs");
            } catch (Throwable t) {
                Log.e("bsh-rs", "native library load failed", t);
            }
        }, "bsh-native-loader");
        loader.setDaemon(true);
        loader.start();
    }
}
