package com.tinclink.android.vpn

import java.time.Instant

enum class VpnPhase {
    IDLE,
    PERMISSION_REQUIRED,
    STARTING,
    TUN_READY,
    STOPPING,
    FAILED,
}

data class VpnSnapshot(
    val phase: VpnPhase,
    val message: String,
    val tunFd: Int? = null,
    val updatedAt: Instant = Instant.now(),
)

/**
 * Process-local observable state for Phase 0. No credential, token or key material belongs here.
 */
object VpnStatusStore {
    private const val MAX_LOG_ENTRIES = 80
    private val lock = Any()
    private val entries = ArrayDeque<String>()

    @Volatile
    var snapshot = VpnSnapshot(VpnPhase.IDLE, "VPN 尚未启动")
        private set

    fun publish(phase: VpnPhase, message: String, tunFd: Int? = null): VpnSnapshot {
        val next = VpnSnapshot(phase, sanitizeStatusMessage(message), tunFd)
        synchronized(lock) {
            snapshot = next
            entries.addLast("${next.updatedAt} [$phase] $message")
            while (entries.size > MAX_LOG_ENTRIES) {
                entries.removeFirst()
            }
        }
        return next
    }

    fun logs(): List<String> = synchronized(lock) { entries.toList() }

    internal fun resetForTest() {
        synchronized(lock) {
            entries.clear()
            snapshot = VpnSnapshot(VpnPhase.IDLE, "VPN 尚未启动")
        }
    }
}

internal fun sanitizeStatusMessage(message: String): String {
    val normalized = StringBuilder(MAX_STATUS_MESSAGE_LENGTH)
    var pendingSpace = false

    for (character in message) {
        if (character.isWhitespace() || character.isISOControl()) {
            pendingSpace = normalized.isNotEmpty()
            continue
        }
        if (pendingSpace && normalized.length < MAX_STATUS_MESSAGE_LENGTH - 1) {
            normalized.append(' ')
        }
        pendingSpace = false
        if (normalized.length >= MAX_STATUS_MESSAGE_LENGTH) break
        normalized.append(character)
    }

    return normalized.toString().ifEmpty { "未提供状态详情" }
}

private const val MAX_STATUS_MESSAGE_LENGTH = 160
