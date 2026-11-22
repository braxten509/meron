// Renders the HTML body inside a mobile conversation.
// The native web view reports its dimensions and handles selected links.
// The parent bubble receives size changes and media actions.

package jp.nonbili.meron.ui

import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableFloatStateOf
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.alpha
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.layout.layout
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.unit.Constraints
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import jp.nonbili.meron.shared.applyRemoteContentPolicy
import jp.nonbili.meron.shared.mailBodyCsp
import kotlin.math.roundToInt
import kotlin.random.Random

// Compose Constraints packs sizes into bit fields and cannot represent
// dimensions past ~262k px; sizing the WebView to an unclamped page height
// (a very tall newsletter, or a bogus negative report from the JS bridge)
// makes measurement throw. 20000dp stays well under the limit at any density.
internal val MailBodyMaxReportedHeight = 20_000.dp

internal fun clampMailBodyHeight(reported: Dp): Dp = reported.coerceIn(0.dp, MailBodyMaxReportedHeight)

/** A one-off token for the viewer's `script-src`. Only has to be unguessable by
 *  the mail rendered beside it, which never sees this process. */
private fun randomScriptNonce(): String = buildString { repeat(4) { append(Random.nextInt(1, Int.MAX_VALUE).toString(36)) } }

/** `hue-rotate(180deg)` as the matrix the engine applies, row by row. */
private val HUE_TURN =
    listOf(
        floatArrayOf(-0.574f, 1.43f, 0.144f),
        floatArrayOf(0.426f, 0.43f, 0.144f),
        floatArrayOf(0.426f, 1.43f, -0.856f),
    )

/**
 * The color a darkened body has to be painted with to come out as [surface]: the darkening filter run
 * backwards, like desktop's darkenedFloor. The filter can't reach every color — a very saturated
 * surface falls outside what it produces — and then this is the nearest it gets to, a shade duller
 * than the surface itself.
 */
internal fun darkenedFloor(surface: Color): Color {
    val channels = floatArrayOf(surface.red, surface.green, surface.blue)
    val (r, g, b) = HUE_TURN.map { row -> (1f - (row[0] * channels[0] + row[1] * channels[1] + row[2] * channels[2])).coerceIn(0f, 1f) }
    return Color(r, g, b)
}

@Composable
internal fun HtmlMessageBody(
    html: String,
    // What remembers whether this body's quote was opened (see QuoteFoldMemory):
    // the message id, so the bubble and the reader agree.
    quoteKey: String = html.hashCode().toString(),
    // Whether this message's remote content may load. The mail is spliced into
    // the document below, so its own baked CSP meta ends up outside that
    // document's head, where a meta policy is ignored: the head built here is
    // what actually enforces the decision on both platforms.
    allowRemote: Boolean,
    maxHeight: Dp = Dp.Unspecified,
    onOpenUrl: (String) -> Unit,
    onOpenImage: (String) -> Unit = {},
    fitWideContent: Boolean = false,
    onNaturalWidth: (Dp) -> Unit = {},
    mediaMissing: Int = 0,
    // What a darkened body turns its white into: the bubble, row or card the
    // view sits on. Inverted white is black, which caps nothing.
    darkSurface: Color = Color.Black,
) {
    // The WebView can't tell Compose how tall its content is, so a tiny script
    // reports document height through a platform bridge and we size the view to
    // it. The bubble caps the height (desktop uses 360px) and the WebView scrolls
    // past that; the full-screen reader passes no cap and shows the whole email.
    // Saveable so a message the conversation list scrolled out and back in
    // returns at its measured height: its web view is a fresh one, and growing
    // from the placeholder while on screen would move the list under the reader.
    var contentHeightDp by rememberSaveable(html) { mutableFloatStateOf(0f) }
    val contentHeight = contentHeightDp.dp
    // A WebView reaches neither the app's text sizes nor the system font-size
    // setting, so the reading typography is baked into the stylesheet below.
    // Sizing the text rather than zooming the page is what the overrides here
    // already assume: they flatten the mail's own sizes, so scaling the two
    // declarations scales every body, and images and tables keep fitting the
    // width they were laid out for.
    // Plain-text bodies are sized in sp, which carries the system font setting
    // on both platforms. Where the web view doesn't apply that setting itself,
    // fold it in here so an HTML mail and a text one stay the same size.
    val systemFontScale = if (MailWebViewFollowsSystemFontScale) 1f else LocalDensity.current.fontScale
    val bodyFontSize = scaledCssPx(MESSAGE_HTML_BASE_PX * systemFontScale, LocalMessageFontScale.current)
    // The script's overflow floor (see contentHeight in the script), saved with
    // the height it produced, along with what laid it out: the typography and
    // fitting here, and the width, which only the document knows and checks
    // itself. A floor from another layout would hold text that now runs
    // shorter at its old height, so it is handed back only to the same one.
    // Read once into the document: a later report must not rebuild the page
    // it came from.
    val overflowLayout = "$bodyFontSize|${LocalDensity.current.fontScale}|$fitWideContent"
    var savedOverflowLayout by rememberSaveable(html) { mutableStateOf("") }
    var savedOverflowExtent by rememberSaveable(html) { mutableIntStateOf(0) }
    var savedOverflowWidth by rememberSaveable(html) { mutableIntStateOf(0) }
    val initialOverflow =
        remember(html, overflowLayout) {
            if (savedOverflowLayout == overflowLayout) savedOverflowExtent to savedOverflowWidth else 0 to 0
        }
    // A fresh nonce per document admits the measurement script below and nothing
    // else: the mail is spliced into this page, so a script of its own that
    // survived the core's sanitiser would still have no way to name the token.
    val scriptNonce = remember(html, allowRemote) { randomScriptNonce() }
    val showQuotedLabel = tr("chat.showQuotedText")
    val hideQuotedLabel = tr("chat.hideQuotedText")
    val darkBody = LocalDarkMailBodies.current
    val mobileHtml =
        remember(
            html,
            quoteKey,
            allowRemote,
            scriptNonce,
            fitWideContent,
            darkBody,
            darkSurface,
            bodyFontSize,
            showQuotedLabel,
            hideQuotedLabel,
            initialOverflow,
        ) {
            val body = applyRemoteContentPolicy(html, allowRemote)
            // The state the reader left the quote in, read once per document:
            // toggles inside the page don't rebuild (and so reload) it, they
            // are reported back through onQuoteToggle instead. A folded quote
            // is folded from the first paint — the class is on the root before
            // the body parses, and the script below only places the toggle.
            val quoteOpen = QuoteFoldMemory.isOpen(quoteKey)
            val quoteClass = if (html.contains(HTML_QUOTE_ATTR) && !quoteOpen) "meron-quote-folded" else ""
            val rootClass = listOfNotNull(quoteClass.ifEmpty { null }, "meron-dark".takeIf { darkBody }).joinToString(" ")
            // Inline for the same reason as the height override below: a
            // sender's `html { background }` would otherwise sit, undarkened,
            // behind the inverted body. Not left transparent: a body
            // background under a transparent root propagates to the canvas,
            // outside the body's filter, so a mail declaring a white body
            // stayed white while its text inverted to white on it. The root
            // paints the surface the view sits on, which is what the body's
            // inverted white comes out as too.
            val backdrop = darkSurface.copy(alpha = 1f)
            val rootBackground = if (darkBody) " background: ${backdrop.toHex()} !important;" else ""
            val floor = darkenedFloor(backdrop)
            val floorChannels = listOf(floor.red, floor.green, floor.blue).joinToString(", ") { (it * 255).roundToInt().toString() }
            """
            <!doctype html>
            <!-- The self-sizing WebView needs its document boxes to follow the
                 message. Newsletter resets commonly force html/body to
                 height:100%, pinning them to the empty initial viewport. The
                 override goes inline rather than into the head stylesheet:
                 sender styles are parsed later, and between two equally
                 specific !important rules the later one wins, while an inline
                 declaration outranks every stylesheet rule of the same
                 importance wherever the sender's <style> sits. -->
            <html class="$rootClass" style="height: auto !important; min-height: 0 !important;$rootBackground">
            <head>
              <meta http-equiv="Content-Security-Policy" content="${mailBodyCsp(allowRemote, scriptNonce)}">
              <meta id="meron-viewport" name="viewport" content="width=device-width, initial-scale=1.0">
              <style>
                html, body {
                  margin: 0;
                  padding: 0;
                  width: 100%;
                  /* `anywhere` also shrinks min-content to a single glyph, so a
                     narrow table cell (a 32px spacer holding a name, say) would
                     wrap its text one character per line. `break-word` still
                     breaks long words that would overflow, but leaves intrinsic
                     widths alone. */
                  overflow-wrap: break-word;
                  word-break: normal;
                  font-size: $bodyFontSize;
                  line-height: 1.45;
                }
                body, p, div, span, td, th, li, a {
                  font-size: $bodyFontSize !important;
                  line-height: 1.45 !important;
                }
                /* Preheaders hide their inbox-preview text with an inline
                   font-size:0; the override above would resurrect it as a
                   column of stray characters. */
                [style*="font-size:0"]:not([style*="font-size:0."]),
                [style*="font-size: 0"]:not([style*="font-size: 0."]) {
                  font-size: 0 !important;
                }
                /* max-width alone keeps fixed-pixel layouts (width="600") inside
                   the bubble. Forcing width:auto on top of it would also beat
                   the width="100%" attribute every email layout table relies on,
                   shrinking rows to their content and stranding right-aligned
                   cells and full-width dividers. */
                table {
                  max-width: 100% !important;
                }
                /* ...but max-width cannot shrink a table below its min-content
                   width, and nested fixed-width tables make that constraint
                   circular: the outer table's cell is sized by the inner
                   <table width="640">, so the inner table's max-width:100%
                   resolves against a 640px cell and never clamps, leaving the
                   outer table a 640px min-content width to inherit. The page
                   then lays out wider than the view and is clipped, not
                   scrolled. Clearing the width only where it is declared in
                   pixels breaks that chain at its source while leaving the
                   width="100%" tables above untouched. */
                table[width]:not([width$="%"]) {
                  width: auto !important;
                }
                table.code .diff-line-num {
                  width: 35px !important;
                  min-width: 35px;
                  white-space: nowrap;
                }
                td.line_content pre,
                th.line_content pre {
                  margin: 0 !important;
                  padding: 0 !important;
                  border: 0 !important;
                  border-radius: 0;
                  overflow-wrap: anywhere;
                  white-space: pre-wrap;
                }
                td.line_content pre code,
                th.line_content pre code {
                  min-width: 0;
                }
                img {
                  max-width: 100% !important;
                  height: auto !important;
                }
                div[data-meron-image-grid] {
                  display: flex !important;
                  flex-wrap: wrap !important;
                  gap: 4px !important;
                }
                div[data-meron-image-grid] > * {
                  flex: 1 1 30% !important;
                  max-width: calc(33.333% - 3px) !important;
                  box-sizing: border-box !important;
                  margin: 0 !important;
                }
                /* The folded quoted tail and its toggle (see QuoteFold.kt). The
                   core strips sender `data-*` attributes and meron- classes, so
                   both are ours. */
                html.meron-quote-folded [data-meron-quote] {
                  display: none !important;
                }
                button.meron-quote-toggle {
                  display: block;
                  width: 32px;
                  height: 16px;
                  margin: 6px 0;
                  padding: 0;
                  border: 1px solid currentColor;
                  border-radius: 8px;
                  background: transparent;
                  color: inherit;
                  opacity: 0.5;
                  font: 700 11px/1 sans-serif;
                  letter-spacing: 1px;
                }
                /* Open: the chip stays lit, so it reads as a state rather than
                   as the same button as when folded. */
                button.meron-quote-toggle[aria-expanded="true"] {
                  opacity: 0.9;
                  background: color-mix(in srgb, currentColor 16%, transparent);
                }
                button.meron-quote-toggle::before {
                  content: '•••';
                }
                /* Dark bodies: the mail is drawn inverted, light text on dark,
                   with the hue turned back so links and brand colors stay
                   recognisable. Pictures are inverted a second time to come
                   out as sent (background ones are marked by applyDarkBody
                   below, which also drops the class for mail designed dark). */
                html.meron-dark body,
                html.meron-dark img,
                html.meron-dark video,
                html.meron-dark [data-meron-picture] {
                  filter: invert(1) hue-rotate(180deg);
                }
                /* ...but only once: inside a re-inverted picture an image is
                   already back to its own colors. */
                html.meron-dark [data-meron-picture] img {
                  filter: none;
                }
                /* Inverted white is black, not the surface the view sits on,
                   so a layer over the body caps how light it gets before the
                   filter runs: at the color that inverts to that surface (see
                   darkenedFloor). The layer is the first thing positioned in
                   the body, above everything in the flow and below whatever
                   comes after it with a filter of its own -- the pictures,
                   which a cap on the whole body would tint with the surface
                   instead. What else paints above it is capped one element at
                   a time (see capBackgrounds below). */
                html.meron-dark body::before {
                  content: '' !important;
                  display: block !important;
                  position: absolute !important;
                  inset: 0 !important;
                  background: ${floor.toHex()} !important;
                  mix-blend-mode: darken !important;
                  pointer-events: none !important;
                }
              </style>
            </head>
            <body style="height: auto !important; min-height: 0 !important;">$body
              <script nonce="$scriptNonce">
                (function () {
                  ${mailDocumentScript(fitWideContent, initialOverflow, showQuotedLabel, hideQuotedLabel, quoteOpen, floorChannels)}
                  dropForeignViewports();
                  groupConsecutiveImages();
                  restoreDeclaredCanvas();
                  applyDarkBody();
                  window.addEventListener('load', report);
                  document.addEventListener('DOMContentLoaded', report);
                  if (window.ResizeObserver) {
                    new ResizeObserver(report).observe(document.documentElement);
                  }
                  if (pinchZooms && window.visualViewport) {
                    window.visualViewport.addEventListener('resize', report);
                  }
                  setTimeout(report, 300);
                })();
              </script>
            </body>
            </html>
            """.trimIndent()
        }
    val measured = contentHeight > 0.dp
    val capped = maxHeight != Dp.Unspecified && measured && contentHeight > maxHeight
    val webViewModifier =
        Modifier
            .fillMaxWidth()
            .then(
                if (measured) {
                    Modifier.height(contentHeight)
                } else {
                    // Fixed, not a minimum: in a lazy list the height is otherwise
                    // unbounded and the web view sizes itself to its own layout,
                    // which before the view has a width is hundreds of thousands
                    // of pixels of one-word lines.
                    Modifier.height(80.dp)
                },
            )

    // One call site whichever way the height falls: crossing the cap only swaps
    // the wrapper's modifiers. Were the web view composed in two places, growing
    // past the cap (opening a long quote does) would build a new one and reload
    // the page, losing whatever the reader just did in it.
    val htmlScrollState = rememberScrollState()
    Box(
        Modifier
            .fillMaxWidth()
            .then(
                if (capped) {
                    Modifier
                        .height(maxHeight)
                        .appScrollbar(htmlScrollState, endOffset = HtmlBubbleHorizontalPadding)
                        .verticalScroll(htmlScrollState)
                } else {
                    Modifier
                },
            ),
    ) {
        MailWebViewWithLinkMenu(
            html = mobileHtml,
            mediaMissing = mediaMissing,
            onContentHeight = { contentHeightDp = clampMailBodyHeight(it).value },
            onOverflowExtent = { extent, width ->
                savedOverflowLayout = overflowLayout
                savedOverflowExtent = extent
                savedOverflowWidth = width
            },
            onOpenUrl = onOpenUrl,
            onOpenImage = onOpenImage,
            onQuoteToggle = { open -> QuoteFoldMemory.setOpen(quoteKey, open) },
            onNaturalWidth = onNaturalWidth,
            modifier = webViewModifier,
            fitWideContent = fitWideContent,
            transparentBackground = darkBody,
        )
    }
}

/** The web view plus the link menu it asks for on long press. The menu is anchored to
 *  a box laid out exactly like the web view, so the reported press position places it. */
@Composable
private fun MailWebViewWithLinkMenu(
    html: String,
    onContentHeight: (Dp) -> Unit,
    onOverflowExtent: (Int, Int) -> Unit,
    onOpenUrl: (String) -> Unit,
    onOpenImage: (String) -> Unit,
    onQuoteToggle: (Boolean) -> Unit,
    onNaturalWidth: (Dp) -> Unit,
    modifier: Modifier,
    fitWideContent: Boolean,
    transparentBackground: Boolean,
    mediaMissing: Int,
) {
    var menuTarget by remember { mutableStateOf<MessageLinkMenuTarget?>(null) }
    Box(modifier) {
        MailWebView(
            html = html,
            mediaMissing = mediaMissing,
            onContentHeight = onContentHeight,
            onOverflowExtent = onOverflowExtent,
            onOpenUrl = onOpenUrl,
            onOpenImage = onOpenImage,
            onLinkLongPress = { url, offset -> menuTarget = MessageLinkMenuTarget(url, offset) },
            fitWideContent = fitWideContent,
            transparentBackground = transparentBackground,
            onQuoteToggle = onQuoteToggle,
            onNaturalWidth = onNaturalWidth,
            modifier = Modifier.fillMaxSize(),
        )
        MessageLinkContextMenu(
            target = menuTarget,
            onDismiss = { menuTarget = null },
            onOpenUrl = onOpenUrl,
        )
    }
}
