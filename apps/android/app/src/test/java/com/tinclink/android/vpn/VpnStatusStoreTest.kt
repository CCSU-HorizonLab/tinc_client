package com.tinclink.android.vpn

import org.junit.After
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Before
import org.junit.Test

class VpnStatusStoreTest {
    @Before
    fun setUp() {
        VpnStatusStore.resetForTest()
    }

    @After
    fun tearDown() {
        VpnStatusStore.resetForTest()
    }

    @Test
    fun tunLifecyclePublishesRealPhasesAndClearsDescriptorOnStop() {
        VpnStatusStore.publish(VpnPhase.PERMISSION_REQUIRED, "等待授权")
        VpnStatusStore.publish(VpnPhase.STARTING, "正在创建")
        val ready = VpnStatusStore.publish(VpnPhase.TUN_READY, "TUN 已创建", 42)

        assertEquals(VpnPhase.TUN_READY, ready.phase)
        assertEquals(42, ready.tunFd)

        val stopped = VpnStatusStore.publish(VpnPhase.IDLE, "TUN 已释放")
        assertEquals(VpnPhase.IDLE, stopped.phase)
        assertNull(stopped.tunFd)
    }

    @Test
    fun logHistoryIsBounded() {
        repeat(100) { VpnStatusStore.publish(VpnPhase.STARTING, "event-$it") }

        assertEquals(80, VpnStatusStore.logs().size)
        assertEquals(true, VpnStatusStore.logs().first().endsWith("event-20"))
        assertEquals(true, VpnStatusStore.logs().last().endsWith("event-99"))
    }
}
