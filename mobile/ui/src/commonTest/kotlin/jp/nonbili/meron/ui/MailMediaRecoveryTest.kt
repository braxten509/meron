// Recovery of a missing attachment must reach the
// document that requested it. The fixtures navigate
// between messages and deliver callbacks out of
// order.

package jp.nonbili.meron.ui

import kotlin.test.Test
import kotlin.test.assertNotNull
import kotlin.test.assertNull

class MailMediaRecoveryTest {
    @Test
    fun recoveryWaitsForTheMatchingDocumentAndSurvivesNavigation() {
        val recovery = MailMediaRecovery(2)
        recovery.update(2, documentChanged = true)
        recovery.update(1, documentChanged = false)
        val first = assertNotNull(recovery.request())
        recovery.acknowledge(first, installed = false)
        assertNotNull(recovery.request())
        recovery.update(1, documentChanged = true)
        recovery.acknowledge(first, installed = true)
        val next = assertNotNull(recovery.request())
        recovery.acknowledge(next, installed = true)
        assertNull(recovery.request())
    }

    @Test
    fun anOlderCallbackCannotConsumeAnotherRecoverySignal() {
        val recovery = MailMediaRecovery(2)
        recovery.update(2, documentChanged = true)
        recovery.update(1, documentChanged = false)
        val first = assertNotNull(recovery.request())
        recovery.update(0, documentChanged = false)
        recovery.acknowledge(first, installed = true)
        val latest = assertNotNull(recovery.request())
        recovery.acknowledge(latest, installed = true)
        assertNull(recovery.request())
    }
}
