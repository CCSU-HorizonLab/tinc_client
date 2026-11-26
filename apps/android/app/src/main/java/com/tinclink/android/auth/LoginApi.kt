package com.tinclink.android.auth

import org.json.JSONObject
import java.net.HttpURLConnection
import java.net.URL

sealed interface LoginResult {
    data class Success(val sid: String, val netName: String, val nodeIp: String) : LoginResult
    data class Failure(val message: String) : LoginResult
}

/** Phase 0 login probe. The returned token is validated and discarded, never persisted. */
object LoginApi {
    fun login(input: ValidatedLoginInput): LoginResult {
        val connection = try {
            URL(input.serverOrigin + "api/tinc/client/login").openConnection() as HttpURLConnection
        } catch (_: Exception) {
            return LoginResult.Failure("无法创建服务端连接")
        }
        return try {
            connection.requestMethod = "POST"
            connection.connectTimeout = 5_000
            connection.readTimeout = 20_000
            connection.instanceFollowRedirects = false
            connection.doOutput = true
            connection.setRequestProperty("Content-Type", "application/json; charset=utf-8")
            connection.setRequestProperty("Accept", "application/json")
            val body = requestBody(input).toByteArray(Charsets.UTF_8)
            connection.outputStream.use { it.write(body) }
            if (connection.responseCode != 200) {
                return LoginResult.Failure("登录失败：HTTP ${connection.responseCode}")
            }
            val contentType = connection.contentType?.substringBefore(';')?.trim()
            if (!contentType.equals("application/json", ignoreCase = true)) {
                return LoginResult.Failure("登录响应不是 JSON")
            }
            val bytes = connection.inputStream.use { stream ->
                val output = java.io.ByteArrayOutputStream()
                val buffer = ByteArray(4096)
                while (true) {
                    val count = stream.read(buffer)
                    if (count < 0) break
                    if (output.size() + count > 64 * 1024) {
                        return LoginResult.Failure("登录响应超过大小限制")
                    }
                    output.write(buffer, 0, count)
                }
                output.toByteArray()
            }
            parseSuccess(String(bytes, Charsets.UTF_8), input.accessCode)
        } catch (_: Exception) {
            LoginResult.Failure("登录请求失败，请检查地址和网络")
        } finally {
            connection.disconnect()
        }
    }

    internal fun requestBody(input: ValidatedLoginInput): String = JSONObject()
        .put("accessCode", input.accessCode)
        .put("password", input.password)
        .toString()

    internal fun parseSuccess(body: String, requestedCode: String): LoginResult {
        return try {
            val json = JSONObject(body)
            val status = json.opt("status")
            if (status !is Int || status <= 0) {
                return LoginResult.Failure("服务端未确认登录成功")
            }
            val sid = json.requiredString("sid")
            val netName = json.requiredString("net_name")
            val nodeIp = json.requiredString("node_ip")
            json.requiredString("token")
            val returnedCode = json.opt("access_code")
            if (returnedCode != null &&
                (returnedCode !is String || !returnedCode.equals(requestedCode, true))
            ) {
                return LoginResult.Failure("服务端返回的接入码不匹配")
            }
            if (!nodeIp.matches(Regex("(?:[0-9]{1,3}\\.){3}[0-9]{1,3}")) ||
                nodeIp.split('.').any { it.toInt() > 255 }
            ) {
                return LoginResult.Failure("服务端返回的节点 IP 无效")
            }
            LoginResult.Success(sid, netName, nodeIp)
        } catch (_: Exception) {
            LoginResult.Failure("登录响应缺少有效节点身份")
        }
    }

    private fun JSONObject.requiredString(key: String): String =
        (opt(key) as? String)?.takeIf { it.isNotBlank() }
            ?: throw IllegalArgumentException("Missing $key")
}
