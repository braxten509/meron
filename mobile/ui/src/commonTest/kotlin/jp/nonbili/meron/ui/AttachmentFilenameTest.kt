// Inline attachments may arrive with a generated name or no usable
// extension. These tests exercise how the suggested filename uses
// cached image information and preserves an existing valid name.

package jp.nonbili.meron.ui

import jp.nonbili.meron.shared.MessageAttachment
import kotlin.test.Test
import kotlin.test.assertEquals

class AttachmentFilenameTest {
    @Test
    fun addsImageExtensionToDatedInlineName() {
        assertEquals(
            "scan_2024.10.22.png",
            safeAttachmentFilename(MessageAttachment("scan 2024.10.22", "image/png", key = "message/0.22")),
        )
    }

    @Test
    fun borrowsCachedImageExtensionAndPreservesExistingImageNames() {
        assertEquals("Release_v1.2.png", safeAttachmentFilename(MessageAttachment("Release v1.2", "image/png", key = "message/0.png")))
        assertEquals("photo.jpeg", safeAttachmentFilename(MessageAttachment("photo.jpeg", "image/jpeg", key = "message/0.jpg")))
        assertEquals("report.pdf", safeAttachmentFilename(MessageAttachment("report.pdf", key = "message/0.pdf")))
    }

    @Test
    fun preservesAlternativeImageExtensions() {
        assertEquals("photo.jfif", safeAttachmentFilename(MessageAttachment("photo.jfif", "image/jpeg", key = "message/0.jfif")))
        assertEquals("anim.apng", safeAttachmentFilename(MessageAttachment("anim.apng", "image/png", key = "message/0.apng")))
    }

    @Test
    fun preservesExtensionlessOrdinaryAttachmentNames() {
        assertEquals("README", safeAttachmentFilename(MessageAttachment("README", "text/plain", key = "message/0.plain")))
        assertEquals("invite", safeAttachmentFilename(MessageAttachment("invite", "text/calendar", key = "message/0.calendar")))
    }
}
