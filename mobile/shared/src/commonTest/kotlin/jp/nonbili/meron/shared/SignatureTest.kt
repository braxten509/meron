// A sending account inherits the global signature until
// it supplies an override. These tests exercise
// inheritance and how a blank signature resolves for a
// new draft.

package jp.nonbili.meron.shared

import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertNull
import kotlin.test.assertTrue

class SignatureTest {
    private val account = AccountSummary(id = "acc", email = "me@example.com")

    @Test
    fun accountFollowsTheAppSignatureUntilItOverridesIt() {
        assertEquals("<p>App</p>", resolveSignatureHtml(account, "<p>App</p>"))
        assertEquals(
            "<p>Mine</p>",
            resolveSignatureHtml(account.copy(signature = SignatureSpec("custom", "<p>Mine</p>")), "<p>App</p>"),
        )
        assertEquals("", resolveSignatureHtml(account.copy(signature = SignatureSpec("none", "<p>Mine</p>")), "<p>App</p>"))
        assertEquals("<p>App</p>", resolveSignatureHtml(account.copy(signature = SignatureSpec("global", "<p>Mine</p>")), "<p>App</p>"))
    }

    @Test
    fun blankSignaturesResolveToNothing() {
        assertTrue(signatureIsBlank("<p></p>"))
        assertTrue(signatureIsBlank("<p>&nbsp;</p>"))
        assertTrue(!signatureIsBlank("<p><img src=\"/media/logo.png\"></p>"))
        assertEquals("", resolveSignatureHtml(account, "<p>&nbsp;</p>"))
    }

    @Test
    fun htmlBecomesTheLinesTheComposerEdits() {
        assertEquals("Ping\nPong", signaturePlainText("<p>Ping<br>Pong</p>"))
        assertEquals("Ping\n\nPong", signaturePlainText("<p>Ping</p><p></p><p>Pong</p>"))
        assertEquals("R&D <ping>", signaturePlainText("<p>R&amp;D &lt;ping&gt;</p>"))
        assertEquals("", signaturePlainText(""))
    }

    @Test
    fun signatureSitsBelowTypedTextAndAboveAQuote() {
        assertEquals("\n\nPing", bodyWithSignature("", "Ping"))
        assertEquals("typed\n\nPing", bodyWithSignature("typed", "Ping"))
        assertEquals(
            "\n\nPing\n\n> quoted",
            bodyWithSignature("> quoted", "Ping", SignaturePlacement.AboveQuote),
        )
        assertEquals("typed", bodyWithSignature("typed", ""))
    }

    @Test
    fun changingAccountSwapsTheSignatureItWasSeededWith() {
        val tracked = SignatureMark("From A", SignaturePlacement.BelowText)

        val swapped = bodyWithSwappedSignature("Hello\n\nFrom A", tracked, "From B")
        assertEquals("Hello\n\nFrom B", swapped.body)
        assertEquals(SignatureMark("From B", SignaturePlacement.BelowText), swapped.tracking)

        val removed = bodyWithSwappedSignature("Hello\n\nFrom A", tracked, "")
        assertEquals("Hello", removed.body)
        // Not simply "none": the placement survives, so the next account's
        // signature goes back where this one was.
        assertEquals(noSignatureMark(SignaturePlacement.BelowText), removed.tracking)
    }

    @Test
    fun multiLineSignaturesSwapAsABlock() {
        val tracked = SignatureMark("From A\nTeam A", SignaturePlacement.BelowText)
        assertEquals("Hello\n\nFrom B", bodyWithSwappedSignature("Hello\n\nFrom A\nTeam A", tracked, "From B").body)
    }

    @Test
    fun anEditedSignatureIsNeitherRewrittenNorTrackedFurther() {
        val tracked = SignatureMark("From A", SignaturePlacement.BelowText)
        // "From A" appears, but as part of a line the user has written into.
        val out = bodyWithSwappedSignature("Hello\n\nFrom A, but mine now", tracked, "From B")

        assertEquals("Hello\n\nFrom A, but mine now", out.body)
        assertNull(out.tracking)
    }

    @Test
    fun aDraftTheAppLeftWithoutOneGetsTheNewAccountsSignature() {
        val out = bodyWithSwappedSignature("Hello", NO_SIGNATURE, "From B")

        assertEquals("Hello\n\nFrom B", out.body)
        assertEquals(SignatureMark("From B", SignaturePlacement.BelowText), out.tracking)
    }

    @Test
    fun anUnmanagedBodyIsNeverTouched() {
        // A reopened draft already ends in whatever signature it was written
        // with: appending here is how a message ends up with two.
        val out = bodyWithSwappedSignature("Hello\n\nFrom A", null, "From B")

        assertEquals("Hello\n\nFrom A", out.body)
        assertNull(out.tracking)
    }

    @Test
    fun theQuotedCopyOfTheSameSignatureSurvives() {
        // Forward of a message that ends in the same signature: ours is the one
        // above the quote.
        val tracked = SignatureMark("From A", SignaturePlacement.AboveQuote)
        val out = bodyWithSwappedSignature("\n\nFrom A\n\n> Forwarded\n> From A", tracked, "From B")

        assertEquals("\n\nFrom B\n\n> Forwarded\n> From A", out.body)
    }

    @Test
    fun removalClosesOnlyTheSeamItCut() {
        val tracked = SignatureMark("From A", SignaturePlacement.BelowText)
        val body = "one\n\n\n\ntwo\n\nFrom A"

        assertEquals("one\n\n\n\ntwo", bodyWithSwappedSignature(body, tracked, "").body)

        // Above a quote, the blank line meant for typing is not the signature's.
        val quoted = SignatureMark("From A", SignaturePlacement.AboveQuote)
        assertEquals(
            "\n\n> quoted\n\n\n> lines",
            bodyWithSwappedSignature("\n\nFrom A\n\n> quoted\n\n\n> lines", quoted, "").body,
        )
    }

    @Test
    fun aForwardRemembersWhereItsSignatureBelongs() {
        // Opened under an account with no signature: the quote is the whole
        // body, and the mark still knows a signature goes above it.
        val forwarded = "\n\n> Forwarded message"
        val out = bodyWithSwappedSignature(forwarded, noSignatureMark(SignaturePlacement.AboveQuote), "From B")

        assertEquals("\n\nFrom B\n\n> Forwarded message", out.body)
        assertEquals(SignatureMark("From B", SignaturePlacement.AboveQuote), out.tracking)
    }

    @Test
    fun anAmbiguousBodyIsLeftEntirelyAlone() {
        // The user pasted their signature into the message as well. Which copy
        // is ours is a guess, and guessing wrong rewrites their words.
        val tracked = SignatureMark("From A", SignaturePlacement.BelowText)
        val body = "From A\n\nis how I sign off\n\nFrom A\n\nPS. one more thing"
        val out = bodyWithSwappedSignature(body, tracked, "From B")

        assertEquals(body, out.body)
        assertNull(out.tracking)
    }

    @Test
    fun anAmbiguousBodyStillSwapsWhenOursIsUntouchedAtTheEdge() {
        val tracked = SignatureMark("From A", SignaturePlacement.BelowText)
        val body = "From A\n\nis how I sign off\n\nFrom A"

        assertEquals(
            "From A\n\nis how I sign off\n\nFrom B",
            bodyWithSwappedSignature(body, tracked, "From B").body,
        )
    }

    @Test
    fun appendingDoesNotReformatTheBodyAroundIt() {
        val body = "  indented start\n\nand a trailing space \n"

        assertEquals("$body\nFrom B", bodyWithSwappedSignature(body, NO_SIGNATURE, "From B").body)
    }

    @Test
    fun accountSignatureParamsClearTheOverrideWithNull() {
        assertEquals(
            """{"id":"acc","signature":null}""",
            AccountSignatureParams("acc", null).toJson(),
        )
        assertEquals(
            """{"id":"acc","signature":{"mode":"custom","html":"<p>Ping</p>"}}""",
            AccountSignatureParams("acc", SignatureSpec("custom", "<p>Ping</p>")).toJson(),
        )
    }

    @Test
    fun accountListCarriesTheSignatureOverride() {
        val accounts =
            parseAccountListResponse(
                """{"accounts":[{"id":"acc","email":"me@example.com","signature":{"mode":"custom","html":"<p>Ping</p>"}}]}""",
            )
        assertEquals(SignatureSpec("custom", "<p>Ping</p>"), accounts.single().signature)

        val plain = parseAccountListResponse("""{"accounts":[{"id":"acc","email":"me@example.com"}]}""")
        assertEquals(SignatureSpec.followApp, plain.single().signature)
    }
}
