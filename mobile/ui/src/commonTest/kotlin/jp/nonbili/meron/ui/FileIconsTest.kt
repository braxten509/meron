// Checks how the attachment view chooses icons from filenames
// and MIME types. Specific contact, calendar, signature and
// mail formats take priority over the generic file fallback.

package jp.nonbili.meron.ui

import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.filled.InsertDriveFile
import androidx.compose.material.icons.filled.ContactPage
import androidx.compose.material.icons.filled.Description
import androidx.compose.material.icons.filled.Email
import androidx.compose.material.icons.filled.Event
import androidx.compose.material.icons.filled.Image
import androidx.compose.material.icons.filled.Key
import kotlin.test.Test
import kotlin.test.assertEquals

class FileIconsTest {
    @Test
    fun namesInvitesContactCardsForwardedMailAndSignaturesByExtension() {
        assertEquals(Icons.Filled.Event, fileIconFor("invite.ics", ""))
        assertEquals(Icons.Filled.ContactPage, fileIconFor("Dana Evans.vcf", ""))
        assertEquals(Icons.Filled.Email, fileIconFor("Fwd.eml", ""))
        assertEquals(Icons.Filled.Key, fileIconFor("smime.p7s", ""))
        assertEquals(Icons.Filled.Image, fileIconFor("IMG_0001.HEIC", ""))
    }

    @Test
    fun checksSpecificTextTypesBeforeTheTextCatchAll() {
        assertEquals(Icons.Filled.Event, fileIconFor("invite", "text/calendar; method=REQUEST"))
        assertEquals(Icons.Filled.ContactPage, fileIconFor("card", "text/x-vcard"))
        assertEquals(Icons.Filled.Description, fileIconFor("notes", "text/plain"))
    }

    @Test
    fun fallsBackToAGenericFile() {
        assertEquals(Icons.AutoMirrored.Filled.InsertDriveFile, fileIconFor("mystery.bin", "application/octet-stream"))
        assertEquals(Icons.AutoMirrored.Filled.InsertDriveFile, fileIconFor("ics", "application/octet-stream"))
    }
}
