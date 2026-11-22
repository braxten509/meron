// The account editor maps stored server flags to
// transport-security choices. These tests also check that
// leaving a password field untouched preserves the
// existing secret.

package jp.nonbili.meron.ui

import jp.nonbili.meron.shared.AddPasswordAccountParams
import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertFalse
import kotlin.test.assertTrue

class ServerSettingsTest {
    @Test
    fun storedFlagsMapOntoTheSecurityModesTheEditorOffers() {
        assertEquals(MailSecurity.TLS, mailSecurityOf(tls = true, starttls = false))
        assertEquals(MailSecurity.STARTTLS, mailSecurityOf(tls = true, starttls = true))
        // STARTTLS wins even when the stored `tls` flag is off, matching how the
        // core records a STARTTLS account.
        assertEquals(MailSecurity.STARTTLS, mailSecurityOf(tls = false, starttls = true))
        assertEquals(MailSecurity.NONE, mailSecurityOf(tls = false, starttls = false))
    }

    /**
     * The whole point of the nullable password: editing servers must not blank
     * the credential the keychain holds. An omitted key tells the core to keep
     * it; `""` would clear it.
     */
    @Test
    fun anUntypedPasswordIsOmittedFromTheRequest() {
        val json = paramsWithPassword(null).toJson()
        assertFalse(json.contains("\"password\""), "expected no password key, got: $json")
    }

    @Test
    fun aTypedPasswordIsSent() {
        val json = paramsWithPassword("hunter2").toJson()
        assertTrue(json.contains("\"password\":\"hunter2\""), "expected the password to be sent, got: $json")
    }

    /** A deliberate blank still reaches the core, which is how a password is cleared. */
    @Test
    fun anEmptyPasswordIsDistinctFromAnOmittedOne() {
        assertTrue(paramsWithPassword("").toJson().contains("\"password\":\"\""))
    }

    /**
     * A new account has no stored row to read a pin back from, so the pin the
     * user accepted has to travel on the request that creates it.
     */
    @Test
    fun anAcceptedCertificateIsSentWithTheAccountThatCreatesIt() {
        val json = paramsWithPassword("pw").copy(certPin = "a".repeat(64)).toJson()
        assertTrue(json.contains("\"cert_pin\":\"${"a".repeat(64)}\""), "expected the pin to be sent, got: $json")
        assertFalse(json.contains("\"smtp_cert_pin\""), "an unset SMTP pin must stay omitted, got: $json")
    }

    /** Omitted pins keep whatever the stored account already trusts. */
    @Test
    fun absentPinsAreOmittedRatherThanCleared() {
        val json = paramsWithPassword("pw").toJson()
        assertFalse(json.contains("cert_pin"), "expected no pin keys, got: $json")
    }

    @Test
    fun theMintedAccountIdMatchesTheNormalizedAddress() {
        assertEquals("user@example.com", mailAccountId("  User@Example.COM  "))
    }

    private fun paramsWithPassword(password: String?) =
        AddPasswordAccountParams(
            email = "user@example.com",
            imapHost = "imap.example.com",
            imapPort = 993,
            smtpHost = "smtp.example.com",
            smtpPort = 465,
            username = "user@example.com",
            password = password,
        )
}
