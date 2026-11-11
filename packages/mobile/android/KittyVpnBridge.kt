package com.kitty.pro

import android.app.Activity
import android.content.Context
import android.content.Intent
import android.net.VpnService
import android.os.Looper
import java.util.concurrent.CountDownLatch
import java.util.concurrent.TimeUnit

object KittyVpnBridge {
    private const val VPN_PERMISSION_REQUEST = 23141

    @Volatile
    private var pendingConfig: String? = null

    @Volatile
    private var state = "stopped"

    @JvmStatic
    fun start(activity: Activity, config: String): Int {
        if (config.isBlank()) {
            state = "error: VPN configuration is empty"
            return -1
        }
        if (Looper.myLooper() == Looper.getMainLooper()) {
            return startOnMainThread(activity, config)
        }

        val latch = CountDownLatch(1)
        var result = -1
        activity.runOnUiThread {
            result = startOnMainThread(activity, config)
            latch.countDown()
        }
        return if (latch.await(5, TimeUnit.SECONDS)) result else -1
    }

    @JvmStatic
    fun stop(context: Context) {
        pendingConfig = null
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
    fun onActivityResult(activity: Activity, requestCode: Int, resultCode: Int) {
        if (requestCode != VPN_PERMISSION_REQUEST) {
            return
        }
        val config = pendingConfig
        pendingConfig = null
        if (resultCode != Activity.RESULT_OK || config == null) {
            state = "error: Android VPN authorization was denied"
            return
        }
        launchService(activity, config)
    }

    @JvmStatic
    fun updateState(nextState: String) {
        state = nextState
    }

    private fun startOnMainThread(activity: Activity, config: String): Int {
        val approval = VpnService.prepare(activity)
        if (approval != null) {
            pendingConfig = config
            state = "authorization"
            activity.startActivityForResult(approval, VPN_PERMISSION_REQUEST)
            return 1
        }
        launchService(activity, config)
        return 0
    }

    private fun launchService(context: Context, config: String) {
        state = "starting"
        val intent = Intent(context, KittyVpnService::class.java)
            .putExtra(KittyVpnService.CONFIG_EXTRA, config)
        if (android.os.Build.VERSION.SDK_INT >= android.os.Build.VERSION_CODES.O) {
            context.startForegroundService(intent)
        } else {
            context.startService(intent)
        }
    }
}
