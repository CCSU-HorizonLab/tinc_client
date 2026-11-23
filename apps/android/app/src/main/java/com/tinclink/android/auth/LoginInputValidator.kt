package com.tinclink.android.auth

import java.net.URI
import java.util.Locale

data class ValidatedLoginInput(
    val serverOrigin: String,
    val accessCode: String,
    val password: String,
)

sealed interface LoginInputValidation {
    data class Valid(val input: ValidatedLoginInput) : LoginInputValidation

    data class Invalid(val message: String) : LoginInputValidation
}

object LoginInputValidator {
    private const val ACCESS_CODE_LENGTH = 12
    private const val MAX_PASSWORD_BYTES = 1024
    private const val VALID_ACCESS_CODE_CHARS = "23456789ABCDEFGHJKMNPQRSTUVWXYZ"

    fun validate(
        server: String,
        accessCode: String,
        password: String,
        allowHttp: Boolean = false,
    ): LoginInputValidation {
        val origin = normalizeOrigin(server, allowHttp)
            ?: return LoginInputValidation.Invalid(
                if (allowHttp) {
                    "请输入完整的 HTTP 或 HTTPS 服务根地址，不支持路径、凭据或查询参数"
                } else {
                    "请输入完整的 HTTPS 服务根地址"
                },
            )
        val normalizedCode = normalizeAccessCode(accessCode)
            ?: return LoginInputValidation.Invalid(
                "接入码格式无效，应类似 TL-7K9M-2Q4F",
            )
        val passwordBytes = password.toByteArray(Charsets.UTF_8).size
        if (passwordBytes == 0 || passwordBytes > MAX_PASSWORD_BYTES) {
            return LoginInputValidation.Invalid("密码不能为空或超过 1024 字节")
        }
        return LoginInputValidation.Valid(
            ValidatedLoginInput(
                serverOrigin = origin,
                accessCode = normalizedCode,
                password = password,
            ),
        )
    }

    internal fun normalizeAccessCode(value: String): String? {
        val normalized = value.trim().uppercase(Locale.ROOT)
        if (normalized.length != ACCESS_CODE_LENGTH) return null
        if (!normalized.startsWith("TL-") || normalized[7] != '-') return null
        for ((index, character) in normalized.withIndex()) {
            if (index < 3 || index == 7) continue
            if (character !in VALID_ACCESS_CODE_CHARS) return null
        }
        return normalized
    }

    internal fun normalizeOrigin(value: String, allowHttp: Boolean): String? {
        val uri = try {
            URI(value.trim())
        } catch (_: Exception) {
            return null
        }
        val scheme = uri.scheme?.lowercase(Locale.ROOT) ?: return null
        if (scheme != "https" && !(allowHttp && scheme == "http")) return null
        val host = uri.host?.lowercase(Locale.ROOT) ?: return null
        if (
            uri.userInfo != null ||
            uri.query != null ||
            uri.fragment != null ||
            (uri.rawPath.isNotEmpty() && uri.rawPath != "/") ||
            uri.port == 0 ||
            uri.port > 65535
        ) {
            return null
        }
        val hostPart = if (':' in host) "[$host]" else host
        val portPart = uri.port.takeIf { it > 0 }?.let { ":$it" }.orEmpty()
        return "$scheme://$hostPart$portPart/"
    }
}
