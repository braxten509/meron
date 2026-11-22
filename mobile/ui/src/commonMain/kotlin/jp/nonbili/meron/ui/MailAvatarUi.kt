// Renders sender and account avatars in mobile mail lists.
// The conversation view can overlay the account badge on the sender image.
// Fallbacks use the address, account type and display name when no image is available.

package jp.nonbili.meron.ui

import androidx.compose.foundation.Image
import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.offset
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.Image
import androidx.compose.material.icons.filled.RssFeed
import androidx.compose.material3.Icon
import androidx.compose.material3.LocalTextStyle
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.ImageBitmap
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.layout.ContentScale
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.text.TextStyle
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.LineHeightStyle
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import jp.nonbili.meron.shared.AccountSummary
import jp.nonbili.meron.shared.accountSummaryIsRss
import kotlinx.coroutines.withContext

@Composable
internal fun ThreadAvatarWithAccountBadge(
    senderLabel: String,
    showSenderImages: Boolean,
    account: AccountSummary?,
) {
    Box(Modifier.size(38.dp)) {
        SenderAvatar(label = senderLabel, enabled = showSenderImages, size = 38.dp)
        if (account != null) {
            val label = account.displayName.ifBlank { account.email.ifBlank { account.id } }
            Box(
                Modifier
                    .align(Alignment.BottomStart)
                    // Hangs off the corner like desktop's, clear of the sender's initials.
                    .offset(x = (-4).dp, y = 4.dp)
                    .size(18.dp)
                    .clip(CircleShape)
                    .border(2.dp, MaterialTheme.colorScheme.surface, CircleShape),
            ) {
                AccountBadgeAvatar(
                    label = label,
                    avatarUrl = account.avatarUrl,
                    size = 18.dp,
                    fallbackIcon = accountAvatarFallbackIcon(account),
                )
            }
        }
    }
}

@Composable
internal fun AccountBadgeAvatar(
    label: String,
    avatarUrl: String,
    size: Dp,
    fallbackIcon: ImageVector? = null,
) {
    val side = with(LocalDensity.current) { size.roundToPx() }
    var image by remember(avatarUrl, side) { mutableStateOf<ImageBitmap?>(null) }

    LaunchedEffect(avatarUrl, side) {
        image = null
        if (avatarUrl.isNotBlank()) {
            image = withContext(ioDispatcher) { loadThumbnailBitmapRef(avatarUrl, side) }
        }
    }

    val current = image
    if (current != null) {
        Image(
            bitmap = current,
            contentDescription = trf("avatar.forLabel", label),
            contentScale = ContentScale.Crop,
            modifier = Modifier.size(size).clip(CircleShape),
        )
    } else {
        Avatar(label, size, fallbackIcon)
    }
}

@Composable
internal fun SenderAvatar(
    label: String,
    enabled: Boolean,
    size: Dp,
    fallbackName: String = label,
) {
    val core = LocalAvatarCore.current
    val email = remember(label) { extractEmail(label) }
    val side = with(LocalDensity.current) { size.roundToPx() }
    var image by remember(email, enabled, core, side) { mutableStateOf<ImageBitmap?>(null) }

    LaunchedEffect(email, enabled, core, side) {
        image = null
        if (enabled && email != null && core != null) {
            image = loadSenderImage(core, email, side)
        }
    }

    val current = image
    if (enabled && current != null) {
        Image(
            bitmap = current,
            contentDescription = trf("avatar.forLabel", label),
            modifier = Modifier.size(size).clip(CircleShape),
        )
    } else {
        Avatar(fallbackName, size)
    }
}

@Composable
internal fun Avatar(
    name: String,
    size: Dp = 42.dp,
    fallbackIcon: ImageVector? = null,
) {
    Box(
        Modifier
            .size(size)
            .clip(CircleShape)
            .background(avatarBrush(name)),
        contentAlignment = Alignment.Center,
    ) {
        if (fallbackIcon != null) {
            Icon(
                fallbackIcon,
                contentDescription = trf("avatar.forLabel", name),
                tint = Color.White,
                modifier = Modifier.size(size * 0.55f),
            )
        } else {
            val fontSize = (size.value * 0.34f).sp
            Text(
                avatarInitials(name),
                color = Color.White,
                fontWeight = FontWeight.SemiBold,
                fontSize = fontSize,
                // The ambient line height is sized for body text, so at avatar
                // font sizes the extra leading pushes the initials off centre
                // (most visible on the 18dp account badge). Trim it to the glyph
                // box so the letters sit in the middle of the circle.
                lineHeight = fontSize,
                style =
                    LocalTextStyle.current.merge(
                        TextStyle(
                            platformStyle = avatarPlatformTextStyle,
                            lineHeightStyle =
                                LineHeightStyle(
                                    alignment = LineHeightStyle.Alignment.Center,
                                    trim = LineHeightStyle.Trim.Both,
                                ),
                        ),
                    ),
            )
        }
    }
}

/** RSS accounts fall back to a feed glyph instead of initials, matching desktop. */
internal fun accountAvatarFallbackIcon(account: AccountSummary): ImageVector? = if (accountSummaryIsRss(account)) Icons.Filled.RssFeed else null

internal fun extractEmail(value: String): String? {
    val match = Regex("[A-Z0-9._%+-]+@[A-Z0-9.-]+\\.[A-Z]{2,}", RegexOption.IGNORE_CASE).find(value)
    return match?.value?.trim()
}
