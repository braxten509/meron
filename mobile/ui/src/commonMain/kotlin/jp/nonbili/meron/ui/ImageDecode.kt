package jp.nonbili.meron.ui

import androidx.compose.ui.graphics.ImageBitmap

/** Decode encoded image bytes (PNG/JPEG/...) into a Compose [ImageBitmap].
 *  With [maxSide] > 0 a big picture is decoded smaller (halved until its longest
 *  side fits), so a 12-megapixel camera photo shown in a mail doesn't hold 48 MB
 *  and stall the screen while it is handed to the graphics chip. */
expect fun decodeImageBitmap(bytes: ByteArray, maxSide: Int = 0): ImageBitmap?

/** A picture for a small box `side` px across (an avatar): shrunk smoothly so
 *  its shorter side is `side`. Drawing a big photo straight into a small box
 *  shrinks it in one rough step and looks blocky. */
expect fun decodeThumbnailBitmap(bytes: ByteArray, side: Int): ImageBitmap?

/** Longest side for pictures shown inside a mail (well above the shown size). */
internal const val INLINE_IMAGE_MAX_SIDE = 2048

/** Longest side for the full-screen viewer: sharp when zoomed, and far below
 *  the size Android refuses to draw. */
internal const val FULL_IMAGE_MAX_SIDE = 4096
