// Resolves the image associated with a message sender
// for the mobile UI. The reader supplies the sender
// identity and observes the result as it becomes
// available.

package jp.nonbili.meron.ui

import androidx.compose.runtime.staticCompositionLocalOf
import androidx.compose.ui.graphics.ImageBitmap
import jp.nonbili.meron.shared.MeronCore
import jp.nonbili.meron.shared.MobileMailCommandClient
import jp.nonbili.meron.shared.parseSenderImageResponse
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.withContext
import kotlin.io.encoding.Base64

internal val LocalAvatarCore = staticCompositionLocalOf<MeronCore?> { null }

/** Core owns provider selection, placeholder rejection, parent fallback and caching. */
internal suspend fun loadSenderImage(
    core: MeronCore,
    email: String,
    side: Int = 0,
): ImageBitmap? =
    loadCachedImageBitmap("sender:$email#$side") {
        withContext(ioDispatcher) {
            try {
                val imageSource = parseSenderImageResponse(MobileMailCommandClient(core).resolveSenderImage(email))
                if (!imageSource.startsWith("data:image/") || !imageSource.contains(";base64,")) {
                    null
                } else {
                    decodeThumbnailBitmap(Base64.Default.decode(imageSource.substringAfter(";base64,")), side)
                }
            } catch (cancelled: CancellationException) {
                throw cancelled
            } catch (_: Exception) {
                null
            }
        }
    }
