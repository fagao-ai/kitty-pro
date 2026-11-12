package com.kitty.pro

import android.app.Application

/**
 * Hands the Rust core what it needs to call back into Java.
 *
 * The core runs in this process and reaches [KittyVpnBridge] and
 * [AndroidDnsBridge] through `ndk-context`, which an NDK `nativeActivity` would
 * have filled in before `main`. This app is not one of those — NSH's runtime is
 * a library an ordinary activity loads — so nothing has recorded the process's
 * `JavaVM` or its `Context` unless this class does, and the first core call
 * would panic instead of starting a tunnel.
 *
 * `onCreate` rather than `attachBaseContext`, which reads as the earlier and
 * therefore safer hook and is not: `Context#getApplicationContext` is answered
 * from `LoadedApk.mApplication`, and that field is set *after*
 * `Application.attach` returns, so asking for the application context inside
 * `attachBaseContext` answers null. `onCreate` is the first point at which the
 * application is an application, and it still runs before NSH's activity builds
 * its view — which is what loads the library — and long before anything can ask
 * the core for a tunnel.
 */
class KittyApplication : Application() {
    override fun onCreate() {
        super.onCreate()
        KittyVpnBridge.attach(this)
    }
}
