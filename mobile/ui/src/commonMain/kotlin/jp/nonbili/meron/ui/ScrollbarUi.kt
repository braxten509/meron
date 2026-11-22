// Computes scrollbar metrics and draws scrolling
// feedback for Compose lists. Thumb size and
// position follow the visible viewport and total
// content.

package jp.nonbili.meron.ui

import androidx.compose.animation.core.animateFloatAsState
import androidx.compose.animation.core.tween
import androidx.compose.foundation.ScrollState
import androidx.compose.foundation.gestures.Orientation
import androidx.compose.foundation.lazy.LazyListState
import androidx.compose.material3.MaterialTheme
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.remember
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.drawWithContent
import androidx.compose.ui.geometry.CornerRadius
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.geometry.Size
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.drawscope.DrawScope
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp
import kotlin.math.max

/** Where the thumb sits and how big it is, in the scroll direction (all in px). */
private data class ScrollbarMetrics(
    val viewport: Float,
    val content: Float,
    val scrolled: Float,
)

private val ThumbThickness = 4.dp
private val ThumbMargin = 2.dp
private val MinThumbLength = 24.dp

// Compose draws no scrollbar on Android or iOS, so long lists give no hint of
// how far along they are. These overlays fade in while a scroll is running and
// fade back out shortly after it stops, matching the platform convention.

/** Scroll position hint for a lazy list. */
@Composable
fun Modifier.appScrollbar(
    state: LazyListState,
    orientation: Orientation = Orientation.Vertical,
    color: Color = thumbColor(),
    endOffset: Dp = 0.dp,
): Modifier {
    val alpha = scrollbarAlpha(state.isScrollInProgress)
    val sizes = remember(state) { ItemSizes() }
    return drawScrollbar(alpha, color, orientation, endOffset) { metrics(state, sizes) }
}

/** Scroll position hint for a plain scrollable column or row. */
@Composable
fun Modifier.appScrollbar(
    state: ScrollState,
    orientation: Orientation = Orientation.Vertical,
    color: Color = thumbColor(),
    endOffset: Dp = 0.dp,
): Modifier {
    val alpha = scrollbarAlpha(state.isScrollInProgress)
    return drawScrollbar(alpha, color, orientation, endOffset) {
        val viewport = if (orientation == Orientation.Vertical) size.height else size.width
        ScrollbarMetrics(
            viewport = viewport,
            content = viewport + state.maxValue,
            scrolled = state.value.toFloat(),
        )
    }
}

/** Readable on the app surfaces; pass a color for the dark sheets and bubbles. */
@Composable
fun thumbColor(): Color = MaterialTheme.colorScheme.onSurface.copy(alpha = 0.4f)

@Composable
private fun scrollbarAlpha(scrolling: Boolean): Float {
    val alpha by
        animateFloatAsState(
            targetValue = if (scrolling) 1f else 0f,
            animationSpec =
                if (scrolling) {
                    tween(durationMillis = 120)
                } else {
                    tween(durationMillis = 400, delayMillis = 500)
                },
            label = "scrollbarAlpha",
        )
    return alpha
}

private fun Modifier.drawScrollbar(
    alpha: Float,
    color: Color,
    orientation: Orientation,
    endOffset: Dp,
    metrics: DrawScope.() -> ScrollbarMetrics?,
): Modifier =
    drawWithContent {
        drawContent()
        if (alpha <= 0f) return@drawWithContent
        val m = metrics() ?: return@drawWithContent
        if (m.content <= m.viewport || m.viewport <= 0f) return@drawWithContent

        val thickness = ThumbThickness.toPx()
        val margin = ThumbMargin.toPx()
        val track = if (orientation == Orientation.Vertical) size.height else size.width
        val length = max(MinThumbLength.toPx(), track * (m.viewport / m.content))
        val progress = (m.scrolled / (m.content - m.viewport)).coerceIn(0f, 1f)
        val start = (track - length) * progress

        // endOffset pushes the thumb past this node's own edge, so a scroller
        // inset by its container's padding still rides that container's edge.
        val edge = thickness + margin - endOffset.toPx()
        val topLeft =
            if (orientation == Orientation.Vertical) {
                Offset(size.width - edge, start)
            } else {
                Offset(start, size.height - edge)
            }
        val thumb =
            if (orientation == Orientation.Vertical) {
                Size(thickness, length)
            } else {
                Size(length, thickness)
            }
        drawRoundRect(
            color = color.copy(alpha = color.alpha * alpha),
            topLeft = topLeft,
            size = thumb,
            cornerRadius = CornerRadius(thickness / 2f),
        )
    }

/** Every item size a lazy list has measured so far, by index. Dropped when the
 *  item count changes: an insertion shifts the indices under it. */
private class ItemSizes {
    var itemCount = -1
    val byIndex = HashMap<Int, Int>()
    var total = 0L

    fun record(
        index: Int,
        size: Int,
    ) {
        total += size - (byIndex.put(index, size) ?: 0)
    }
}

/**
 * Lazy lists only know the items they have measured, so the content length is
 * estimated: items seen so far count at the size they were measured at, and
 * the rest at the average of those. Averaging only the visible items is exact
 * for the uniform rows most of these lists use, but a conversation mixes
 * one-line rows with mails thousands of pixels tall, and there the estimate
 * swung with every row that entered or left the viewport, moving the thumb
 * while the content barely moved.
 */
private fun metrics(
    state: LazyListState,
    sizes: ItemSizes,
): ScrollbarMetrics? {
    val info = state.layoutInfo
    val visible = info.visibleItemsInfo
    val count = info.totalItemsCount
    if (visible.isEmpty() || count == 0) return null
    if (sizes.itemCount != count) {
        sizes.itemCount = count
        sizes.byIndex.clear()
        sizes.total = 0L
    }
    visible.forEach { sizes.record(it.index, it.size) }
    val known = sizes.byIndex.size
    val average = sizes.total.toFloat() / known
    if (average <= 0f) return null
    // Several lists arrange their items with spacing, which sits between the
    // measured sizes; counting only the sizes would leave the thumb long enough
    // to hit the end before the list does.
    val spacing = info.mainAxisItemSpacing.toFloat()
    val viewport = (info.viewportEndOffset - info.viewportStartOffset).toFloat()
    val content =
        sizes.total + (count - known) * average + spacing * (count - 1) +
            info.beforeContentPadding + info.afterContentPadding
    val first = state.firstVisibleItemIndex
    var before = 0f
    for (index in 0 until first) before += sizes.byIndex[index]?.toFloat() ?: average
    val scrolled = before + first * spacing + state.firstVisibleItemScrollOffset
    return ScrollbarMetrics(viewport, content, scrolled)
}
