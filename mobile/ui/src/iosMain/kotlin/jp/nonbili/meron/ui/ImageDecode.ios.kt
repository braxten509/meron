// iOS decodes downloaded or cached image bytes into a
// bitmap for Compose, using the size requested by the
// shared reader. The common source set declares the
// interface.

package jp.nonbili.meron.ui

import androidx.compose.ui.graphics.ImageBitmap
import androidx.compose.ui.graphics.toComposeImageBitmap
import org.jetbrains.skia.Image

// maxSide is not applied on iOS yet: Skia decodes at full size.
actual fun decodeImageBitmap(bytes: ByteArray, maxSide: Int): ImageBitmap? = runCatching { Image.makeFromEncoded(bytes).toComposeImageBitmap() }.getOrNull()

actual fun decodeThumbnailBitmap(bytes: ByteArray, side: Int): ImageBitmap? = decodeImageBitmap(bytes, 0)
