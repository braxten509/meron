// Chooses the icon used to represent a file in
// mobile attachment lists. The selection reflects
// the type information available for that
// attachment.

package jp.nonbili.meron.ui

import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.filled.InsertDriveFile
import androidx.compose.material.icons.filled.AudioFile
import androidx.compose.material.icons.filled.Code
import androidx.compose.material.icons.filled.ContactPage
import androidx.compose.material.icons.filled.Description
import androidx.compose.material.icons.filled.Email
import androidx.compose.material.icons.filled.Event
import androidx.compose.material.icons.filled.FolderZip
import androidx.compose.material.icons.filled.FontDownload
import androidx.compose.material.icons.filled.Image
import androidx.compose.material.icons.filled.Inventory2
import androidx.compose.material.icons.filled.Key
import androidx.compose.material.icons.filled.PictureAsPdf
import androidx.compose.material.icons.filled.Slideshow
import androidx.compose.material.icons.filled.TableChart
import androidx.compose.material.icons.filled.VideoFile
import androidx.compose.ui.graphics.vector.ImageVector

// Mirrors desktop's `fileIconFor` (components/chat/messageHelpers.ts): the same
// extension groups and mime fallbacks, with Material icons in place of lucide.

private val EXT_GROUPS: List<Pair<ImageVector, List<String>>> =
    listOf(
        Icons.Filled.PictureAsPdf to listOf("pdf"),
        Icons.Filled.Description to listOf("doc", "docx", "rtf", "odt", "pages", "txt", "md", "log"),
        Icons.Filled.TableChart to listOf("xls", "xlsx", "xlsm", "csv", "tsv", "ods", "numbers"),
        Icons.Filled.Slideshow to listOf("ppt", "pptx", "odp", "key"),
        Icons.Filled.FolderZip to listOf("zip", "rar", "7z", "tar", "gz", "tgz", "bz2", "xz", "zst"),
        Icons.Filled.Image to listOf("jpg", "jpeg", "png", "gif", "webp", "heic", "heif", "bmp", "tif", "tiff", "svg", "avif"),
        Icons.Filled.VideoFile to listOf("mp4", "m4v", "mov", "mkv", "webm", "avi", "wmv"),
        Icons.Filled.AudioFile to listOf("mp3", "m4a", "wav", "flac", "ogg", "oga", "opus", "aac"),
        // Meeting invites and contact cards, the two structured files mail carries most.
        Icons.Filled.Event to listOf("ics", "ical", "ifb", "vcs"),
        Icons.Filled.ContactPage to listOf("vcf", "vcard"),
        // A forwarded message attached whole.
        Icons.Filled.Email to listOf("eml", "msg"),
        // Signatures and keys: S/MIME's smime.p7s rides on every signed message.
        Icons.Filled.Key to listOf("p7s", "p7m", "p7c", "asc", "sig", "gpg", "pgp", "pem", "crt", "cer"),
        Icons.Filled.Inventory2 to listOf("apk", "dmg", "exe", "msi", "deb", "rpm", "pkg", "appimage", "iso"),
        Icons.Filled.FontDownload to listOf("ttf", "otf", "woff", "woff2"),
        Icons.Filled.Code to
            listOf(
                "js",
                "ts",
                "jsx",
                "tsx",
                "json",
                "html",
                "htm",
                "css",
                "py",
                "rs",
                "go",
                "java",
                "kt",
                "swift",
                "c",
                "h",
                "cpp",
                "rb",
                "php",
                "sh",
                "sql",
                "xml",
                "yml",
                "yaml",
                "toml",
            ),
    )

private val EXT_ICONS: Map<String, ImageVector> =
    EXT_GROUPS.flatMap { (icon, exts) -> exts.map { it to icon } }.toMap()

/** Pick a file icon from extension first, then mime, falling back to a generic file. */
internal fun fileIconFor(
    filename: String,
    mime: String,
): ImageVector {
    val ext = if ('.' in filename) filename.substringAfterLast('.').lowercase() else ""
    EXT_ICONS[ext]?.let { return it }

    // Specific types before the `text/` catch-all: invites and contact cards are
    // text/calendar and text/vcard.
    val mediaType = mime.lowercase().substringBefore(';').trim()
    return when {
        mediaType == "text/calendar" || mediaType == "application/ics" -> Icons.Filled.Event
        mediaType == "text/vcard" || mediaType == "text/x-vcard" || mediaType == "text/directory" -> Icons.Filled.ContactPage
        mediaType == "message/rfc822" || mediaType == "application/vnd.ms-outlook" -> Icons.Filled.Email
        "pkcs7" in mediaType || "pgp" in mediaType || "x509" in mediaType -> Icons.Filled.Key
        mediaType.startsWith("image/") -> Icons.Filled.Image
        mediaType.startsWith("video/") -> Icons.Filled.VideoFile
        mediaType.startsWith("audio/") -> Icons.Filled.AudioFile
        mediaType.startsWith("font/") -> Icons.Filled.FontDownload
        mediaType.startsWith("text/") -> Icons.Filled.Description
        mediaType == "application/pdf" -> Icons.Filled.PictureAsPdf
        "spreadsheet" in mediaType || "excel" in mediaType -> Icons.Filled.TableChart
        "presentation" in mediaType || "powerpoint" in mediaType -> Icons.Filled.Slideshow
        "wordprocessing" in mediaType || "msword" in mediaType || "opendocument.text" in mediaType -> Icons.Filled.Description
        "zip" in mediaType || "compressed" in mediaType || "tar" in mediaType -> Icons.Filled.FolderZip
        "android.package" in mediaType || "x-msdownload" in mediaType || "diskimage" in mediaType -> Icons.Filled.Inventory2
        "json" in mediaType || "javascript" in mediaType || "xml" in mediaType -> Icons.Filled.Code
        else -> Icons.AutoMirrored.Filled.InsertDriveFile
    }
}
