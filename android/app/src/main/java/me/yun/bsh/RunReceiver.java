package me.yun.bsh;

import android.content.BroadcastReceiver;
import android.content.Context;
import android.content.Intent;
import android.util.Log;

/**
 * Headless entry point used for testing on a device:
 *
 * adb shell am broadcast -n me.yun.bsh/.RunReceiver --es code 'print(1+2);'
 *
 * Every output line is logged with a [HH:mm:ss.SSS] timestamp.
 */
public class RunReceiver extends BroadcastReceiver {
    @Override
    public void onReceive(Context context, Intent intent) {
        String code = intent.getStringExtra("code");
        if (code == null) {
            Log.i("bsh-rs", "RunReceiver: no code extra");
            return;
        }
        String output;
        try {
            output = BshBridge.nativeEval(code);
        } catch (Throwable t) {
            output = "native error: " + t;
        }
        Log.i("bsh-rs", output);
    }
}
