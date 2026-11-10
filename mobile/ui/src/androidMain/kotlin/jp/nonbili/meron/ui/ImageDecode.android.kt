package jp.nonbili.meron.ui

import android.graphics.Bitmap
import android.graphics.BitmapFactory
import androidx.compose.ui.graphics.ImageBitmap
import androidx.compose.ui.graphics.asImageBitmap

actual fun decodeImageBitmap(bytes: ByteArray, maxSide: Int): ImageBitmap? =
    runCatching {
        val options = BitmapFactory.Options()
        if (maxSide > 0) {
            val bounds = BitmapFactory.Options().apply { inJustDecodeBounds = true }
            BitmapFactory.decodeByteArray(bytes, 0, bytes.size, bounds)
            val longest = maxOf(bounds.outWidth, bounds.outHeight)
            var sample = 1
            while (longest / sample > maxSide) sample *= 2
            options.inSampleSize = sample
        }
        BitmapFactory.decodeByteArray(bytes, 0, bytes.size, options)?.asImageBitmap()
    }.getOrNull()

actual fun decodeThumbnailBitmap(bytes: ByteArray, side: Int): ImageBitmap? =
    runCatching {
        if (side <= 0) return@runCatching decodeImageBitmap(bytes)
        // Twice the box, as an @2x picture: drawn at half size with smooth filtering.
        val target = side * 2
        val bounds = BitmapFactory.Options().apply { inJustDecodeBounds = true }
        BitmapFactory.decodeByteArray(bytes, 0, bytes.size, bounds)
        val shorter = minOf(bounds.outWidth, bounds.outHeight)
        if (shorter <= 0) return@runCatching null
        // The decoder halves cheaply while the result stays at least twice the box.
        var sample = 1
        while (shorter / (sample * 2) >= target * 2) sample *= 2
        var bitmap =
            BitmapFactory.decodeByteArray(bytes, 0, bytes.size, BitmapFactory.Options().apply { inSampleSize = sample })
                ?: return@runCatching null
        // Then halve with filtering, and one last filtered step to the box.
        while (minOf(bitmap.width, bitmap.height) / 2 >= target) {
            bitmap = bitmap.scaledTo(bitmap.width / 2, bitmap.height / 2)
        }
        val small = minOf(bitmap.width, bitmap.height)
        if (small > target) {
            val scale = target.toFloat() / small
            bitmap = bitmap.scaledTo(maxOf(1, Math.round(bitmap.width * scale)), maxOf(1, Math.round(bitmap.height * scale)))
        }
        bitmap.asImageBitmap()
    }.getOrNull()

private fun Bitmap.scaledTo(width: Int, height: Int): Bitmap {
    val scaled = Bitmap.createScaledBitmap(this, maxOf(1, width), maxOf(1, height), true)
    if (scaled !== this) recycle()
    return scaled
}
