package com.tinclink.android.auth

import org.json.JSONObject
import java.net.HttpURLConnection
import java.net.URL

sealed interface LoginResult {
    data class Success(
        val sid: String,
        val netName: String,
        val nodeIp: String,
        val authorizedRouteCount: Int,
    ) : LoginResult
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
            if (ipv4Octets(nodeIp) == null) {
                return LoginResult.Failure("服务端返回的节点 IP 无效")
            }
            val routes = json.optJSONArray("allowed_routes")
                ?: return LoginResult.Failure("登录响应缺少授权路由快照")
            if (routes.length() > 1024) {
                return LoginResult.Failure("授权路由数量超出安全上限")
            }
            val revision = json.requiredString("route_revision")
            if (revision.length > 256 || revision.trim() != revision ||
                revision.any { it.code !in 33..126 }
            ) {
                return LoginResult.Failure("路由版本格式无效")
            }
            val lease = json.opt("route_lease_seconds")
            if (lease !is Int || lease !in 30..3600) {
                return LoginResult.Failure("路由租约无效")
            }
            val uniqueRoutes = HashSet<String>()
            for (index in 0 until routes.length()) {
                val route = routes.opt(index) as? String
                    ?: return LoginResult.Failure("授权路由格式无效")
                if (!route.endsWith("/32") || route.trim() != route) {
                    return LoginResult.Failure("授权路由仅允许 IPv4 /32")
                }
                val address = route.removeSuffix("/32")
                val octets = ipv4Octets(address)
                    ?: return LoginResult.Failure("授权路由包含无效 IP")
                if (address == nodeIp || octets[0] == 0 || octets[0] == 127 ||
                    octets[0] >= 224 || (octets[0] == 169 && octets[1] == 254)
                ) {
                    return LoginResult.Failure("授权路由包含不允许的目标")
                }
                uniqueRoutes.add(route)
            }
            LoginResult.Success(sid, netName, nodeIp, uniqueRoutes.size)
        } catch (_: Exception) {
            LoginResult.Failure("登录响应缺少有效节点身份")
        }
    }

    private fun JSONObject.requiredString(key: String): String =
        (opt(key) as? String)?.takeIf { it.isNotBlank() }
            ?: throw IllegalArgumentException("Missing $key")

    private fun ipv4Octets(value: String): List<Int>? {
        val parts = value.split('.')
        if (parts.size != 4) return null
        return parts.map { part ->
            if (part.isEmpty() || part.length > 3 || part.any { it !in '0'..'9' }) return null
            val octet = part.toIntOrNull() ?: return null
            if (octet !in 0..255 || octet.toString() != part) return null
            octet
        }
    }
}
