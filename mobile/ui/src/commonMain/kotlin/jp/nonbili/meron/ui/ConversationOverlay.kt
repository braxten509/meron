// Shared surfaces drawn above the conversation and beneath its floating controls.
// The thread screen and quick reply share these dimensions to reserve scroll space.
// The band reads the blur preference when drawing over the message list.
package jp.nonbili.meron.ui

import androidx.compose.foundation.BorderStroke
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.RowScope
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Surface
import androidx.compose.runtime.Composable
import androidx.compose.runtime.remember
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Brush
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.Shape
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.unit.dp
import dev.chrisbanes.haze.HazeInput
import dev.chrisbanes.haze.HazeState
import dev.chrisbanes.haze.blur.HazeBlurStyle
import dev.chrisbanes.haze.blur.HazeColorEffect
import dev.chrisbanes.haze.blur.hazeBlur

internal val REPLY_CARD_TOP_MARGIN = 8.dp
internal val REPLY_CARD_RADIUS = 24.dp

// The reply card and the pinned header ride over the conversation, so they
// sit higher than the bubbles (3.dp) that scroll under them.
internal val FLOATING_CARD_ELEVATION = 4.dp
internal val FLOATING_BAR_CHIP_HEIGHT = 48.dp

// A card floating on content that scrolls on under it, drawn like the reply
// card and the pinned header.
@Composable
internal fun FloatingCard(
    shape: Shape,
    modifier: Modifier = Modifier,
    content: @Composable () -> Unit,
) {
    Surface(
        color = MaterialTheme.colorScheme.surface,
        shape = shape,
        border = BorderStroke(1.dp, MaterialTheme.colorScheme.outlineVariant.copy(alpha = 0.6f)),
        shadowElevation = FLOATING_CARD_ELEVATION,
        modifier = modifier,
        content = content,
    )
}

// One of an app bar's controls, floating on the content that scrolls on under
// the bar.
@Composable
internal fun FloatingBarChip(
    shape: Shape,
    modifier: Modifier = Modifier,
    content: @Composable RowScope.() -> Unit,
) {
    FloatingCard(shape, modifier.height(FLOATING_BAR_CHIP_HEIGHT)) {
        Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.Center, content = content)
    }
}

/** The "Blurred bars" setting (off by default, see BLURRED_BARS_PREF). */
internal val LocalBlurredBars = androidx.compose.runtime.compositionLocalOf { false }

/** Draws the band behind controls that float over a conversation.
 *
 * Haze captures the scrolling content. The two fade distances soften exposed edges.
 * Devices with WebView offscreen rendering issues can use the plain veil.
 */
@Composable
internal fun FrostedBand(
    hazeState: HazeState,
    heightPx: Int,
    modifier: Modifier = Modifier,
    fadeAtTopPx: Float = 0f,
    fadeAtBottomPx: Float = 0f,
    // Strong enough to keep lines of text from reading as text, light
    // enough that they still show moving underneath.
    tintAlpha: Float = 0.3f,
) {
    if (heightPx <= 0) return
    val background = MaterialTheme.colorScheme.background
    if (!LocalBlurredBars.current) {
        // The plain veil: same band, no blur, so mail pages are drawn once.
        Box(modifier.fillMaxWidth().height(with(LocalDensity.current) { heightPx.toDp() }).background(background.copy(alpha = 0.85f)))
        return
    }
    val style =
        remember(background, heightPx, fadeAtTopPx, fadeAtBottomPx, tintAlpha) {
            val height = heightPx.toFloat()
            HazeBlurStyle {
                backgroundColor(background)
                blurRadius(12.dp)
                noiseFactor(0f)
                colorEffects(listOf(HazeColorEffect.tint(background.copy(alpha = tintAlpha))))
                // Where blurring is unavailable (before Android 12); Haze
                // strengthens it to make up for the missing blur.
                fallbackColorEffect(HazeColorEffect.tint(background.copy(alpha = 0.7f)))
                if (fadeAtTopPx > 0f || fadeAtBottomPx > 0f) {
                    mask(
                        Brush.verticalGradient(
                            0f to if (fadeAtTopPx > 0f) Color.Transparent else Color.Black,
                            (fadeAtTopPx / height).coerceIn(0f, 0.5f) to Color.Black,
                            (1f - fadeAtBottomPx / height).coerceIn(0.5f, 1f) to Color.Black,
                            1f to if (fadeAtBottomPx > 0f) Color.Transparent else Color.Black,
                        ),
                    )
                }
            }
        }
    Box(
        modifier
            .fillMaxWidth()
            .height(with(LocalDensity.current) { heightPx.toDp() })
            .hazeBlur(HazeInput.Sources(hazeState), style),
    )
}
