// Android keeps account images separate from downloaded
// attachments. These tests cover URL-based cache keys,
// selection of a cached image and removal of the oldest
// files.

package jp.nonbili.meron.ui

import java.io.File
import java.nio.file.Files
import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertFalse
import kotlin.test.assertNotEquals
import kotlin.test.assertNull
import kotlin.test.assertTrue

class ImageLoadAndroidTest {
    @Test
    fun androidMediaRootDirectoryKeepsAccountMediaAndAttachmentsSeparate() {
        assertEquals("media", androidMediaRootDirectoryName("avatars/account/avatar.png"))
        assertEquals("media", androidMediaRootDirectoryName("wallpapers/account/wallpaper.webp"))
        assertEquals("attachments", androidMediaRootDirectoryName("account/inbox/1/0.png"))
        assertEquals("attachments", androidMediaRootDirectoryName("rss-account/sub/item/0.jpg"))
    }

    @Test
    fun remoteImageCacheKeyIsStableAndUrlScoped() {
        val first = remoteImageCacheKey("https://example.com/image.png?a=1")
        val second = remoteImageCacheKey("https://example.com/image.png?a=1")
        val other = remoteImageCacheKey("https://example.com/image.png?a=2")

        assertEquals(64, first.length)
        assertEquals(first, second)
        assertNotEquals(first, other)
        assertTrue(first.all { it in '0'..'9' || it in 'a'..'f' })
    }

    @Test
    fun cachedRemoteImageFileFindsNewestImageForUrl() {
        withTemporaryDirectory("meron-image-cache-find") { directory ->
            val key = remoteImageCacheKey("https://example.com/photo")
            File(directory, "$key.txt").writeText("not-image")
            val old =
                File(directory, "$key.jpg").apply {
                    writeBytes(byteArrayOf(1))
                    setLastModified(100)
                }
            val newest =
                File(directory, "$key.webp").apply {
                    writeBytes(byteArrayOf(2))
                    setLastModified(200)
                }

            assertEquals(newest, cachedRemoteImageFile(directory, "https://example.com/photo"))
            assertTrue(old.exists())
            assertNull(cachedRemoteImageFile(directory, "https://example.com/other"))
        }
    }

    @Test
    fun pruneRemoteImageCacheDeletesOldestFilesFirst() {
        withTemporaryDirectory("meron-image-cache-prune") { directory ->
            val oldest =
                File(directory, "old.png").apply {
                    writeBytes(ByteArray(60))
                    setLastModified(100)
                }
            val middle =
                File(directory, "middle.png").apply {
                    writeBytes(ByteArray(50))
                    setLastModified(200)
                }
            val newest =
                File(directory, "new.png").apply {
                    writeBytes(ByteArray(40))
                    setLastModified(300)
                }

            pruneRemoteImageCache(directory, maxBytes = 90)

            assertFalse(oldest.exists())
            assertTrue(middle.exists())
            assertTrue(newest.exists())
            assertTrue(directory.listFiles().orEmpty().sumOf { it.length() } <= 90)
        }
    }
}

private fun withTemporaryDirectory(
    prefix: String,
    block: (File) -> Unit,
) {
    val directory = Files.createTempDirectory(prefix).toFile()
    try {
        block(directory)
    } finally {
        directory.deleteRecursively()
    }
}
