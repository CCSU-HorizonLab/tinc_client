package com.tinclink.android

import android.Manifest
import android.app.Activity
import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent
import android.content.IntentFilter
import android.content.pm.PackageManager
import android.net.VpnService
import android.os.Build
import android.os.Bundle
import android.view.ViewGroup
import android.widget.Button
import android.widget.LinearLayout
import android.widget.ScrollView
import android.widget.TextView
import com.tinclink.android.vpn.TincLinkVpnService
import com.tinclink.android.vpn.VpnPhase
import com.tinclink.android.vpn.VpnStatusStore

class MainActivity : Activity() {
    private lateinit var statusView: TextView
    private lateinit var logView: TextView
    private var receiverRegistered = false
    private var startPendingAfterNotificationRequest = false

    private val statusReceiver = object : BroadcastReceiver() {
        override fun onReceive(context: Context?, intent: Intent?) {
            renderStatus()
        }
    }

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        setContentView(buildContentView())
        renderStatus()
    }

    override fun onStart() {
        super.onStart()
        registerStatusReceiver()
        renderStatus()
    }

    override fun onStop() {
        if (receiverRegistered) {
            unregisterReceiver(statusReceiver)
            receiverRegistered = false
        }
        super.onStop()
    }

    @Suppress("DEPRECATION")
    override fun onActivityResult(requestCode: Int, resultCode: Int, data: Intent?) {
        super.onActivityResult(requestCode, resultCode, data)
        if (requestCode != REQUEST_VPN_PERMISSION) return

        if (resultCode == RESULT_OK) {
            startVpnService()
        } else {
            VpnStatusStore.publish(VpnPhase.PERMISSION_REQUIRED, "用户未授予系统 VPN 权限")
            renderStatus()
        }
    }

    override fun onRequestPermissionsResult(
        requestCode: Int,
        permissions: Array<out String>,
        grantResults: IntArray,
    ) {
        super.onRequestPermissionsResult(requestCode, permissions, grantResults)
        if (requestCode == REQUEST_NOTIFICATION_PERMISSION && startPendingAfterNotificationRequest) {
            startPendingAfterNotificationRequest = false
            requestVpnPermission()
        }
    }

    private fun buildContentView(): ScrollView {
        val density = resources.displayMetrics.density
        val padding = (20 * density).toInt()

        val container = LinearLayout(this).apply {
            orientation = LinearLayout.VERTICAL
            setPadding(padding, padding, padding, padding)
            layoutParams = ViewGroup.LayoutParams(
                ViewGroup.LayoutParams.MATCH_PARENT,
                ViewGroup.LayoutParams.WRAP_CONTENT,
            )
        }

        container.addView(TextView(this).apply {
            text = "TincLink Android Phase 0"
            textSize = 24f
        })
        container.addView(TextView(this).apply {
            text = "本版本只验证 Android VPN 授权和 TUN 生命周期，不代表 Tinc 已连接。"
            textSize = 15f
        })

        statusView = TextView(this).apply {
            textSize = 17f
            setPadding(0, padding, 0, padding)
        }
        container.addView(statusView)

        container.addView(Button(this).apply {
            text = "申请权限并创建 TUN"
            setOnClickListener { beginVpnStart() }
        })
        container.addView(Button(this).apply {
            text = "关闭并释放 TUN"
            setOnClickListener { stopVpnService() }
        })
        container.addView(Button(this).apply {
            text = "刷新状态和日志"
            setOnClickListener { renderStatus() }
        })

        logView = TextView(this).apply {
            textSize = 13f
            setTextIsSelectable(true)
            setPadding(0, padding, 0, 0)
        }
        container.addView(logView)

        return ScrollView(this).apply { addView(container) }
    }

    private fun beginVpnStart() {
        if (
            Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU &&
            checkSelfPermission(Manifest.permission.POST_NOTIFICATIONS) !=
            PackageManager.PERMISSION_GRANTED
        ) {
            startPendingAfterNotificationRequest = true
            requestPermissions(
                arrayOf(Manifest.permission.POST_NOTIFICATIONS),
                REQUEST_NOTIFICATION_PERMISSION,
            )
            return
        }
        requestVpnPermission()
    }

    @Suppress("DEPRECATION")
    private fun requestVpnPermission() {
        val permissionIntent = VpnService.prepare(this)
        if (permissionIntent == null) {
            startVpnService()
        } else {
            VpnStatusStore.publish(VpnPhase.PERMISSION_REQUIRED, "等待 Android 系统 VPN 授权")
            renderStatus()
            startActivityForResult(permissionIntent, REQUEST_VPN_PERMISSION)
        }
    }

    private fun startVpnService() {
        val intent = Intent(this, TincLinkVpnService::class.java)
            .setAction(TincLinkVpnService.ACTION_START)
        startForegroundService(intent)
    }

    private fun stopVpnService() {
        val intent = Intent(this, TincLinkVpnService::class.java)
            .setAction(TincLinkVpnService.ACTION_STOP)
        startService(intent)
    }

    private fun registerStatusReceiver() {
        val filter = IntentFilter(TincLinkVpnService.ACTION_STATUS_CHANGED)
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU) {
            registerReceiver(statusReceiver, filter, RECEIVER_NOT_EXPORTED)
        } else {
            @Suppress("DEPRECATION")
            registerReceiver(statusReceiver, filter)
        }
        receiverRegistered = true
    }

    private fun renderStatus() {
        val snapshot = VpnStatusStore.snapshot
        val descriptor = snapshot.tunFd?.let { "\nTUN FD：$it" }.orEmpty()
        statusView.text = "状态：${snapshot.phase}\n${snapshot.message}$descriptor"
        logView.text = buildString {
            append("运行日志（不记录 Token、密码或密钥）\n\n")
            VpnStatusStore.logs().forEach { append(it).append('\n') }
        }
    }

    companion object {
        private const val REQUEST_VPN_PERMISSION = 1001
        private const val REQUEST_NOTIFICATION_PERMISSION = 1002
    }
}
