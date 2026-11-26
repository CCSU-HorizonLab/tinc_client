package com.tinclink.android

import android.Manifest
import android.app.Activity
import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent
import android.content.IntentFilter
import android.content.pm.PackageManager
import android.content.pm.ApplicationInfo
import android.net.VpnService
import android.os.Build
import android.os.Bundle
import android.text.InputType
import android.view.View
import android.view.ViewGroup
import android.view.WindowManager
import android.widget.Button
import android.widget.CheckBox
import android.widget.EditText
import android.widget.LinearLayout
import android.widget.ScrollView
import android.widget.TextView
import androidx.core.content.ContextCompat
import com.tinclink.android.auth.LoginInputValidation
import com.tinclink.android.auth.LoginInputValidator
import com.tinclink.android.auth.LoginApi
import com.tinclink.android.auth.LoginResult
import com.tinclink.android.vpn.TincLinkVpnService
import com.tinclink.android.vpn.VpnPhase
import com.tinclink.android.vpn.VpnStatusStore

class MainActivity : Activity() {
    private lateinit var serverInput: EditText
    private lateinit var accessCodeInput: EditText
    private lateinit var passwordInput: EditText
    private lateinit var allowHttpInput: CheckBox
    private lateinit var loginValidationView: TextView
    private lateinit var statusView: TextView
    private lateinit var logView: TextView
    private var receiverRegistered = false
    private var startPendingAfterNotificationRequest = false
    private var loginInFlight = false

    private val statusReceiver = object : BroadcastReceiver() {
        override fun onReceive(context: Context?, intent: Intent?) {
            renderStatus()
        }
    }

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        window.addFlags(WindowManager.LayoutParams.FLAG_SECURE)
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

        container.addView(TextView(this).apply {
            text = "接入信息"
            textSize = 18f
            setPadding(0, padding, 0, 0)
        })
        serverInput = EditText(this).apply {
            hint = "服务端根地址，例如 https://vpn.example.com"
            inputType = InputType.TYPE_CLASS_TEXT or InputType.TYPE_TEXT_VARIATION_URI
            setSingleLine(true)
        }
        container.addView(serverInput)
        accessCodeInput = EditText(this).apply {
            hint = "接入码，例如 TL-7K9M-2Q4F"
            inputType = InputType.TYPE_CLASS_TEXT or InputType.TYPE_TEXT_FLAG_CAP_CHARACTERS
            setSingleLine(true)
        }
        container.addView(accessCodeInput)
        passwordInput = EditText(this).apply {
            hint = "节点密码"
            inputType = InputType.TYPE_CLASS_TEXT or InputType.TYPE_TEXT_VARIATION_PASSWORD
            isSaveEnabled = false
            importantForAutofill = View.IMPORTANT_FOR_AUTOFILL_NO
            setSingleLine(true)
        }
        container.addView(passwordInput)
        allowHttpInput = CheckBox(this).apply {
            text = "测试环境允许 HTTP 地址（仅 Debug 版本）"
        }
        container.addView(allowHttpInput)
        container.addView(Button(this).apply {
            text = "校验接入信息（不发送）"
            setOnClickListener { validateLoginInput() }
        })
        container.addView(Button(this).apply {
            text = "测试服务端登录（不启动 VPN）"
            setOnClickListener { testServerLogin() }
        })
        loginValidationView = TextView(this).apply {
            text = "尚未校验，也未向服务端发送任何数据"
            setPadding(0, 0, 0, padding)
        }
        container.addView(loginValidationView)

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

    private fun validateLoginInput() {
        when (
            val result = LoginInputValidator.validate(
                server = serverInput.text.toString(),
                accessCode = accessCodeInput.text.toString(),
                password = passwordInput.text.toString(),
                allowHttp = allowHttpInput.isChecked,
            )
        ) {
            is LoginInputValidation.Valid -> {
                serverInput.setText(result.input.serverOrigin)
                accessCodeInput.setText(result.input.accessCode)
                passwordInput.text.clear()
                loginValidationView.text =
                    "格式校验通过；尚未向服务端登录，密码已从输入框清除"
            }

            is LoginInputValidation.Invalid -> {
                loginValidationView.text = "格式校验失败：${result.message}"
            }
        }
    }

    private fun testServerLogin() {
        if (loginInFlight) return
        val validation = LoginInputValidator.validate(
            serverInput.text.toString(),
            accessCodeInput.text.toString(),
            passwordInput.text.toString(),
            allowHttpInput.isChecked,
        )
        if (validation is LoginInputValidation.Invalid) {
            loginValidationView.text = "格式校验失败：${validation.message}"
            return
        }
        val input = (validation as LoginInputValidation.Valid).input
        if (input.serverOrigin.startsWith("http://") &&
            applicationInfo.flags and ApplicationInfo.FLAG_DEBUGGABLE == 0
        ) {
            passwordInput.text.clear()
            loginValidationView.text = "正式版本只允许 HTTPS 登录"
            return
        }
        serverInput.setText(input.serverOrigin)
        accessCodeInput.setText(input.accessCode)
        passwordInput.text.clear()
        loginInFlight = true
        loginValidationView.text = "正在验证服务端身份；此操作不会启动 VPN"
        Thread {
            val result = LoginApi.login(input)
            runOnUiThread {
                if (isDestroyed) return@runOnUiThread
                loginInFlight = false
                loginValidationView.text = when (result) {
                    is LoginResult.Success ->
                        "服务端登录成功：${result.netName} / ${result.sid} / ${result.nodeIp}；尚未连接 VPN"
                    is LoginResult.Failure -> result.message
                }
            }
        }.start()
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
        ContextCompat.registerReceiver(
            this,
            statusReceiver,
            filter,
            ContextCompat.RECEIVER_NOT_EXPORTED,
        )
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
