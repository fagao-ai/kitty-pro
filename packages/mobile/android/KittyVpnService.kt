package com.kitty.pro

import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.Service
import android.content.Intent
import android.content.pm.ServiceInfo
import android.net.VpnService
import android.os.Build
import android.os.IBinder
import android.os.ParcelFileDescriptor
import androidx.core.app.NotificationCompat
import dev.dioxus.main.R
import java.util.concurrent.atomic.AtomicBoolean

class KittyVpnService : VpnService() {
    companion object {
        const val CONFIG_EXTRA = "config"
        private const val CHANNEL_ID = "kitty-pro-vpn"
        private const val NOTIFICATION_ID = 1001

        @Volatile
        private var activeService: KittyVpnService? = null

        init {
            System.loadLibrary("main")
        }

        @JvmStatic
        fun trafficPayload(): String = activeService?.nativeTraffic().orEmpty()

        @JvmStatic
        fun stopActive(): Boolean {
            val service = activeService ?: return false
            service.shutdownCoreAsync()
            service.stopSelf()
            return true
        }
    }

    private var tunnel: ParcelFileDescriptor? = null
    private val shutdownRequested = AtomicBoolean(false)

    private external fun nativeStart(config: String, tunFd: Int, dataPath: String): String?
    private external fun nativeStop(): String?
    private external fun nativeTraffic(): String?

    override fun onCreate() {
        super.onCreate()
        activeService = this
        createNotificationChannel()
    }

    override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int {
        val config = intent?.getStringExtra(CONFIG_EXTRA)
        if (config.isNullOrBlank()) {
            KittyVpnBridge.updateState("error: missing VPN configuration")
            stopSelf(startId)
            return Service.START_NOT_STICKY
        }

        startVpnForeground()
        shutdownCore()
        val descriptor = establishTunnel()
        if (descriptor == null) {
            KittyVpnBridge.updateState("error: Android rejected the VPN interface")
            stopSelf(startId)
            return Service.START_NOT_STICKY
        }
        tunnel = descriptor
        KittyVpnBridge.updateState("starting")
        Thread {
            val error = nativeStart(config, descriptor.fd, filesDir.absolutePath)
            if (error.isNullOrBlank()) {
                KittyVpnBridge.updateState("running")
            } else {
                KittyVpnBridge.updateState("error: $error")
                shutdownCore()
                stopSelf()
            }
        }.apply {
            name = "kitty-pro-vpn-core"
            start()
        }
        return Service.START_STICKY
    }

    override fun onRevoke() {
        KittyVpnBridge.updateState("stopped")
        shutdownCoreAsync()
        stopSelf()
        super.onRevoke()
    }

    override fun onDestroy() {
        shutdownCoreAsync()
        if (activeService === this) {
            activeService = null
        }
        KittyVpnBridge.updateState("stopped")
        super.onDestroy()
    }

    override fun onBind(intent: Intent?): IBinder? = super.onBind(intent)

    private fun establishTunnel(): ParcelFileDescriptor? {
        val builder = Builder()
            .setSession("Kitty Pro")
            .setMtu(1500)
            .addAddress("172.19.0.1", 30)
            .addAddress("fdfe:dcba:9876::1", 126)
            .addRoute("0.0.0.0", 0)
            .addRoute("::", 0)
            .addDnsServer("172.19.0.2")

        // The embedded core shares this process. Excluding it keeps its
        // upstream connections outside the VPN and prevents an outbound loop.
        try {
            builder.addDisallowedApplication(packageName)
        } catch (_: android.content.pm.PackageManager.NameNotFoundException) {
            KittyVpnBridge.updateState("error: unable to exclude the VPN process")
            return null
        }
        return builder.establish()
    }

    private fun shutdownCore() {
        tunnel?.close()
        tunnel = null
        nativeStop()
    }

    private fun shutdownCoreAsync() {
        if (!shutdownRequested.compareAndSet(false, true)) {
            return
        }
        tunnel?.close()
        tunnel = null
        Thread {
            nativeStop()
        }.apply {
            name = "kitty-pro-vpn-stop"
            start()
        }
    }

    private fun createNotificationChannel() {
        if (Build.VERSION.SDK_INT < Build.VERSION_CODES.O) {
            return
        }
        val manager = getSystemService(NotificationManager::class.java)
        manager.createNotificationChannel(
            NotificationChannel(
                CHANNEL_ID,
                "Kitty Pro VPN",
                NotificationManager.IMPORTANCE_LOW,
            ),
        )
    }

    private fun startVpnForeground() {
        val notification = NotificationCompat.Builder(this, CHANNEL_ID)
            .setContentTitle("Kitty Pro")
            .setContentText("VPN connection is active")
            .setSmallIcon(R.drawable.kitty_notification)
            .setOngoing(true)
            .build()
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.UPSIDE_DOWN_CAKE) {
            startForeground(
                NOTIFICATION_ID,
                notification,
                ServiceInfo.FOREGROUND_SERVICE_TYPE_SPECIAL_USE,
            )
        } else {
            startForeground(NOTIFICATION_ID, notification)
        }
    }
}
