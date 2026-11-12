package com.kitty.pro

import android.content.Context
import android.content.Intent
import android.net.VpnService
import android.os.Build
import android.os.Handler
import android.os.Looper
import java.util.concurrent.CountDownLatch
import java.util.concurrent.TimeUnit

/**
 * The functions the embedded core calls back into.
 *
 * The core is a Go library in this process and cannot raise a tunnel or ask the
 * user for anything, so every one of these is reached through JNI from
 * `packages/singbox/src/android.rs`, on a thread of the core's choosing.
 */
object KittyVpnBridge {
    init {
        // The core and the engine share one library, and NSH's Java side loads it
        // when its view is built — which is after this object first runs. Loading
        // it here is what makes that ordering irrelevant, and it is what lets
        // [nativeInit] below resolve at all.
        System.loadLibrary("nsh")
    }

    private external fun nativeInit(context: Context)

    /**
     * Hand the core this process's `JavaVM` and a context to call through. Called
     * once by [KittyApplication], before the core can want either.
     *
     * The application, deliberately, rather than the front activity:
     * `ndk-context` holds one object for the life of the process and has no way to
     * replace it, while the activity changes under rotation and on every screen.
     * Everything the core asks of the object is something an application context
     * answers — a class loader, `getFilesDir`, a system service, and here
     * `VpnService.prepare`, which is documented to take any context. What does need
     * an activity — showing the system's allow-VPN dialog and owning its result —
     * goes to [KittyVpnPermissionActivity] instead. It could not be done any other
     * way even if this object held an activity: NSH's `AppActivity` is `final`, so
     * nothing could forward `onActivityResult` from it.
     */
    @JvmStatic
    fun attach(context: Context) {
        nativeInit(context.applicationContext)
    }

    @Volatile
    private var state = "stopped"

    /**
     * Ask for the tunnel. Returns 0 when the service is starting, 1 when the user
     * has been asked first, and a negative number when neither happened; the core
     * treats only negative as a refusal.
     */
    @JvmStatic
    fun start(context: Context, config: String): Int {
        if (config.isBlank()) {
            state = "error: VPN configuration is empty"
            return -1
        }
        val app = context.applicationContext
        if (Looper.myLooper() == Looper.getMainLooper()) {
            return startOnMainThread(app, config)
        }
        // Not `context.startActivity` from this thread: starting the permission
        // activity is legal off the main thread, but `state` is what the interface
        // reads, and one thread writing it is what makes "authorization" a state a
        // reader can see rather than one it can miss. The core waits on this call,
        // so a latch costs nothing it was not already paying.
        val latch = CountDownLatch(1)
        var result = -1
        Handler(Looper.getMainLooper()).post {
            result = startOnMainThread(app, config)
            latch.countDown()
        }
        return if (latch.await(5, TimeUnit.SECONDS)) result else -1
    }

    @JvmStatic
    fun stop(context: Context) {
        state = "stopped"
        if (!KittyVpnService.stopActive()) {
            context.stopService(Intent(context, KittyVpnService::class.java))
        }
    }

    @JvmStatic
    fun status(): String = state

    @JvmStatic
    fun traffic(): String = KittyVpnService.trafficPayload()

    @JvmStatic
    fun updateState(nextState: String) {
        state = nextState
    }

    private fun startOnMainThread(context: Context, config: String): Int {
        val approval = VpnService.prepare(context)
        if (approval != null) {
            state = "authorization"
            val request = Intent(context, KittyVpnPermissionActivity::class.java)
                .putExtra(KittyVpnPermissionActivity.APPROVAL_EXTRA, approval)
                .putExtra(KittyVpnPermissionActivity.CONFIG_EXTRA, config)
                // Required, not style: this starts from an application context,
                // which is not itself a task, and Android refuses the call without
                // it. The animation is off because there is no window of ours to
                // see — the permission activity draws nothing.
                .addFlags(Intent.FLAG_ACTIVITY_NEW_TASK or Intent.FLAG_ACTIVITY_NO_ANIMATION)
            context.startActivity(request)
            return 1
        }
        launchService(context, config)
        return 0
    }

    internal fun launchService(context: Context, config: String) {
        state = "starting"
        val intent = Intent(context, KittyVpnService::class.java)
            .putExtra(KittyVpnService.CONFIG_EXTRA, config)
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.O) {
            context.startForegroundService(intent)
        } else {
            context.startService(intent)
        }
    }
}
