package com.tinclink.android.vpn

import android.app.Notification
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.app.Service
import android.content.Intent
import android.content.pm.ServiceInfo
import android.net.VpnService
import android.os.Build
import android.os.ParcelFileDescriptor
import com.tinclink.android.MainActivity
import java.io.IOException

class TincLinkVpnService : VpnService() {
    private var tunInterface: ParcelFileDescriptor? = null

    override fun onCreate() {
        super.onCreate()
        createNotificationChannel()
    }

    override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int {
        when (intent?.action) {
            ACTION_STOP -> stopVpn("用户已断开 VPN")
            ACTION_START -> startVpn()
            else -> {
                publish(VpnPhase.FAILED, "收到无效的 VPN 服务指令")
                stopSelf()
            }
        }
        return Service.START_NOT_STICKY
    }

    @Synchronized
    private fun startVpn() {
        if (tunInterface != null) {
            publish(
                VpnPhase.TUN_READY,
                "TUN 已存在；Phase 0 尚未接入 Tinc 数据面",
                tunInterface?.fd,
            )
            return
        }

        startAsForeground("正在创建 Android TUN")
        publish(VpnPhase.STARTING, "正在创建 Android TUN")

        try {
            val established = Builder()
                .setSession("TincLink Phase 0")
                .setMtu(PHASE0_MTU)
                .addAddress(PHASE0_CLIENT_ADDRESS, PHASE0_PREFIX_LENGTH)
                .addRoute(PHASE0_TEST_ROUTE, PHASE0_PREFIX_LENGTH)
                .setBlocking(false)
                .establish()
                ?: throw IllegalStateException("系统未建立 TUN；VPN 授权可能已失效")

            tunInterface = established
            publish(
                VpnPhase.TUN_READY,
                "TUN 已创建；仅接管文档测试地址，尚未建立 Tinc 隧道",
                established.fd,
            )
            updateNotification("TUN 已创建，等待 Tinc 引擎接入")
        } catch (error: Exception) {
            val closeError = closeTun()
            val detail = closeError?.let { "；同时无法确认 TUN 已释放：$it" }.orEmpty()
            publish(VpnPhase.FAILED, "TUN 创建失败：${safeMessage(error)}$detail")
            if (closeError == null) {
                stopForeground(STOP_FOREGROUND_REMOVE)
                stopSelf()
            } else {
                updateNotification("TUN 清理失败，请返回应用重试")
            }
        }
    }

    @Synchronized
    private fun stopVpn(reason: String) {
        publish(VpnPhase.STOPPING, "正在关闭 Android TUN")
        val closeError = closeTun()
        if (closeError == null) {
            publish(VpnPhase.IDLE, reason)
            stopForeground(STOP_FOREGROUND_REMOVE)
            stopSelf()
        } else {
            publish(VpnPhase.FAILED, "TUN 关闭失败，不能确认资源已经释放：$closeError")
            updateNotification("TUN 关闭失败，请返回应用重试")
        }
    }

    override fun onRevoke() {
        stopVpn("系统已撤销 VPN 授权并关闭 TUN")
        super.onRevoke()
    }

    override fun onDestroy() {
        val previousPhase = VpnStatusStore.snapshot.phase
        val closeError = closeTun()
        if (closeError != null) {
            publish(VpnPhase.FAILED, "VPN 服务结束时 TUN 关闭失败：$closeError")
        } else if (
            previousPhase == VpnPhase.STARTING ||
            previousPhase == VpnPhase.TUN_READY ||
            previousPhase == VpnPhase.STOPPING
        ) {
            publish(VpnPhase.IDLE, "VPN 服务已结束，TUN 已释放")
        }
        super.onDestroy()
    }

    private fun closeTun(): String? {
        val current = tunInterface ?: return null
        return try {
            current.close()
            tunInterface = null
            null
        } catch (error: IOException) {
            safeMessage(error)
        }
    }

    private fun publish(phase: VpnPhase, message: String, tunFd: Int? = null) {
        val snapshot = VpnStatusStore.publish(phase, message, tunFd)
        sendBroadcast(
            Intent(ACTION_STATUS_CHANGED)
                .setPackage(packageName)
                .putExtra(EXTRA_PHASE, snapshot.phase.name)
                .putExtra(EXTRA_MESSAGE, snapshot.message)
                .putExtra(EXTRA_TUN_FD, snapshot.tunFd ?: -1),
        )
    }

    private fun startAsForeground(content: String) {
        val notification = buildNotification(content)
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.UPSIDE_DOWN_CAKE) {
            startForeground(
                NOTIFICATION_ID,
                notification,
                ServiceInfo.FOREGROUND_SERVICE_TYPE_SYSTEM_EXEMPTED,
            )
        } else {
            startForeground(NOTIFICATION_ID, notification)
        }
    }

    private fun updateNotification(content: String) {
        getSystemService(NotificationManager::class.java)
            .notify(NOTIFICATION_ID, buildNotification(content))
    }

    private fun buildNotification(content: String): Notification {
        val openApp = PendingIntent.getActivity(
            this,
            0,
            Intent(this, MainActivity::class.java),
            PendingIntent.FLAG_UPDATE_CURRENT or PendingIntent.FLAG_IMMUTABLE,
        )
        return Notification.Builder(this, NOTIFICATION_CHANNEL_ID)
            .setSmallIcon(android.R.drawable.stat_sys_warning)
            .setContentTitle("TincLink Android")
            .setContentText(content)
            .setContentIntent(openApp)
            .setOngoing(true)
            .build()
    }

    private fun createNotificationChannel() {
        val channel = NotificationChannel(
            NOTIFICATION_CHANNEL_ID,
            "VPN 运行状态",
            NotificationManager.IMPORTANCE_LOW,
        )
        getSystemService(NotificationManager::class.java).createNotificationChannel(channel)
    }

    private fun safeMessage(error: Exception): String =
        error.message?.take(160) ?: error.javaClass.simpleName

    companion object {
        const val ACTION_START = "com.tinclink.android.action.START_VPN"
        const val ACTION_STOP = "com.tinclink.android.action.STOP_VPN"
        const val ACTION_STATUS_CHANGED = "com.tinclink.android.action.VPN_STATUS_CHANGED"
        const val EXTRA_PHASE = "phase"
        const val EXTRA_MESSAGE = "message"
        const val EXTRA_TUN_FD = "tun_fd"

        private const val NOTIFICATION_CHANNEL_ID = "tinclink_vpn"
        private const val NOTIFICATION_ID = 2101
        private const val PHASE0_MTU = 1500
        private const val PHASE0_CLIENT_ADDRESS = "192.0.2.2"
        private const val PHASE0_TEST_ROUTE = "192.0.2.1"
        private const val PHASE0_PREFIX_LENGTH = 32
    }
}
