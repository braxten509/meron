// A certificate error must identify the server before the client
// can ask the reader to trust it. Ordinary connection errors
// retain their original meaning and follow normal error
// handling.

package jp.nonbili.meron.shared

import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertNull
import kotlin.test.assertTrue

class CertificateTrustTest {
    @Test
    fun certificateFailuresName_theServerThatRefused() {
        // What a Proton Mail Bridge account produces on its first sync.
        val imap = "tls handshake: untrusted-certificate: invalid peer certificate: Other(OtherError(CaUsedAsEndEntity))"
        assertTrue(isUntrustedCertificateError(imap))
        assertEquals(CertificateProtocol.IMAP, untrustedCertificateProtocol(imap))

        val smtp = "smtp-untrusted-certificate: invalid peer certificate"
        assertTrue(isUntrustedCertificateError(smtp))
        assertEquals(CertificateProtocol.SMTP, untrustedCertificateProtocol(smtp))
    }

    @Test
    fun ordinaryFailuresAreLeftAlone() {
        assertNull(untrustedCertificateProtocol("login failed: authentication failed"))
        assertTrue(!isUntrustedCertificateError("connect 127.0.0.1:1143: connection refused"))
    }

    @Test
    fun fingerprintIsPrintedForComparison() {
        assertEquals("E3:B0:C4:42:98:FC:1C:14", formatCertificateFingerprint("e3b0c44298fc1c14"))
        assertEquals("", formatCertificateFingerprint(""))
    }

    @Test
    fun commonNameIsPickedOutOfTheRdnSequence() {
        assertEquals("127.0.0.1", certificateCommonName("CN=127.0.0.1, O=Proton AG"))
        assertEquals("Proton Mail Bridge", certificateCommonName("C=CH, O=Proton AG, CN=Proton Mail Bridge"))
        assertEquals("O=Proton AG", certificateCommonName("O=Proton AG"))
    }

    @Test
    fun probeResponseCarriesWhatTheDialogShows() {
        val certificate =
            parseProbeCertResponse(
                """
                {"certificate":{"fingerprint":"8db220e8","subject":"CN=127.0.0.1, O=Fake Bridge",
                "issuer":"CN=127.0.0.1, O=Fake Bridge","not_before":"Sat, 22 Aug 2026 23:57:04 +0000",
                "not_after":"Mon, 24 Aug 2026 23:57:04 +0000","self_signed":true}}
                """.trimIndent(),
            )
        assertEquals("8db220e8", certificate?.fingerprint)
        assertEquals("CN=127.0.0.1, O=Fake Bridge", certificate?.issuer)
        assertEquals(true, certificate?.selfSigned)
        assertEquals("Mon, 24 Aug 2026 23:57:04 +0000", certificate?.notAfter)
    }

    @Test
    fun aProbeThatFoundNoCertificateIsNotAPrompt() {
        assertNull(parseProbeCertResponse("""{"certificate":{"fingerprint":""}}"""))
        assertNull(parseProbeCertResponse("""{"error":{"message":"tcp connect: connection refused"}}"""))
    }

    /** The probe has to take the same route as the connection that failed. */
    @Test
    fun probesCarryTheAccountProxy() {
        val custom =
            ProbeCertParams(
                host = "127.0.0.1",
                port = 1025,
                protocol = "smtp",
                starttls = true,
                proxy = ProxySpec("socks5", "127.0.0.1", 9050),
            ).toJson()
        assertTrue(custom.contains("\"proxy\":{\"mode\":\"socks5\""), custom)

        // No account override: the key is left out, and the core follows the
        // app-wide proxy exactly as the mail connection does.
        val inherited = ProbeCertParams(host = "127.0.0.1", port = 1143, protocol = "imap", starttls = true).toJson()
        assertTrue(!inherited.contains("proxy"), inherited)
    }

    @Test
    fun pinsAreSentOnlyForTheServerThatWasAccepted() {
        val imapOnly = AccountCertPinParams(accountId = "acc", certPin = "abc").toJson()
        assertTrue(imapOnly.contains("\"cert_pin\":\"abc\""))
        assertTrue(!imapOnly.contains("smtp_cert_pin"))

        val smtpOnly = AccountCertPinParams(accountId = "acc", smtpCertPin = "def").toJson()
        assertTrue(smtpOnly.contains("\"smtp_cert_pin\":\"def\""))
        assertTrue(!smtpOnly.contains("\"cert_pin\":"))
    }
}
