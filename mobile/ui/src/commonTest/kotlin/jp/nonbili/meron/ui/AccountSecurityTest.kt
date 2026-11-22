// Editing an account port can suggest a new
// transport-security mode. The tests distinguish
// untouched defaults from explicit choices the account
// editor must preserve.

package jp.nonbili.meron.ui

import kotlin.test.Test
import kotlin.test.assertEquals

class AccountSecurityTest {
    @Test
    fun standardPortsSelectTheirExpectedSecurityMode() {
        assertEquals(MailSecurity.TLS, mailSecurityForPort(993))
        assertEquals(MailSecurity.TLS, mailSecurityForPort(465))
        assertEquals(MailSecurity.STARTTLS, mailSecurityForPort(143))
        assertEquals(MailSecurity.STARTTLS, mailSecurityForPort(587))
        assertEquals(MailSecurity.NONE, mailSecurityForPort(3143))
    }

    @Test
    fun portEditsOnlyUpdateUntouchedSecurityModes() {
        assertEquals(MailSecurity.STARTTLS, mailSecurityAfterPortEdit(MailSecurity.TLS, false, "143"))
        assertEquals(MailSecurity.TLS, mailSecurityAfterPortEdit(MailSecurity.STARTTLS, false, "993"))
        assertEquals(MailSecurity.NONE, mailSecurityAfterPortEdit(MailSecurity.NONE, true, "993"))
        assertEquals(MailSecurity.STARTTLS, mailSecurityAfterPortEdit(MailSecurity.STARTTLS, false, ""))
    }

    @Test
    fun discoveryPreservesUserAndExistingServerChoices() {
        // A host the user owns keeps its port and security mode.
        assertEquals(
            MailServerSelection("143", MailSecurity.STARTTLS),
            mailServerSelectionAfterDiscovery("143", MailSecurity.STARTTLS, false, true, false, 993),
        )
        assertEquals(
            MailServerSelection("143", MailSecurity.STARTTLS),
            mailServerSelectionAfterDiscovery("143", MailSecurity.STARTTLS, false, false, false, 0),
        )
        assertEquals(
            MailServerSelection("993", MailSecurity.NONE),
            mailServerSelectionAfterDiscovery("143", MailSecurity.NONE, true, false, false, 993),
        )
        assertEquals(
            MailServerSelection("143", MailSecurity.STARTTLS),
            mailServerSelectionAfterDiscovery("993", MailSecurity.TLS, false, false, false, 143),
        )
        assertEquals(
            MailServerSelection("143", MailSecurity.STARTTLS),
            mailServerSelectionAfterDiscovery("143", MailSecurity.STARTTLS, true, false, true, 993),
        )
    }

    @Test
    fun discoveryRefreshesServersItFilledInItself() {
        // Switching to another provider re-runs discovery: the port and mode
        // left over from the previous lookup must not survive it.
        assertEquals(
            MailServerSelection("993", MailSecurity.TLS),
            mailServerSelectionAfterDiscovery("3143", MailSecurity.NONE, false, false, false, 993),
        )
        assertEquals(
            MailServerSelection("993", MailSecurity.TLS),
            mailServerSelectionAfterDiscovery(
                "143",
                MailSecurity.STARTTLS,
                true,
                true,
                true,
                993,
                preserveUserSettings = false,
            ),
        )
    }

    @Test
    fun discoveryOnlyTreatsACompleteServerAsReplaceable() {
        assertEquals(true, discoveredServerIsComplete("mail.example.com", 993))
        assertEquals(false, discoveredServerIsComplete("", 993))
        assertEquals(false, discoveredServerIsComplete("mail.example.com", 0))
    }
}
