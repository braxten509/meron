// Edits the visible identity of a mobile mail account.
// The account settings page supplies aliases and the current avatar.
// Callbacks report edits and image selection to the account state.

package jp.nonbili.meron.ui

import androidx.compose.foundation.Image
import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.Close
import androidx.compose.material.icons.filled.ContentPaste
import androidx.compose.material.icons.filled.Edit
import androidx.compose.material.icons.filled.Image
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Surface
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
import androidx.compose.ui.graphics.ImageBitmap
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.layout.ContentScale
import androidx.compose.ui.platform.LocalClipboardManager
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.text.input.KeyboardType
import androidx.compose.ui.unit.dp
import kotlinx.coroutines.withContext

// A single send-as alias rendered as an editable row instead of a free-form
// textarea: an email field, an optional display-name field, and a remove button.
@Composable
internal fun AliasEditorRow(
    email: String,
    name: String,
    onEmailChange: (String) -> Unit,
    onNameChange: (String) -> Unit,
    onRemove: () -> Unit,
) {
    Surface(color = MaterialTheme.colorScheme.surface, modifier = Modifier.fillMaxWidth()) {
        Row(
            modifier = Modifier.fillMaxWidth().padding(start = 16.dp, end = 4.dp, top = 8.dp, bottom = 8.dp),
            verticalAlignment = Alignment.CenterVertically,
            horizontalArrangement = Arrangement.spacedBy(8.dp),
        ) {
            Column(
                modifier = Modifier.weight(1f),
                verticalArrangement = Arrangement.spacedBy(8.dp),
            ) {
                Row(
                    modifier = Modifier.fillMaxWidth(),
                    verticalAlignment = Alignment.CenterVertically,
                    horizontalArrangement = Arrangement.spacedBy(8.dp),
                ) {
                    OutlinedTextField(
                        value = email,
                        onValueChange = onEmailChange,
                        label = { Text(tr("accounts.fields.emailAddress")) },
                        singleLine = true,
                        keyboardOptions = nativeTextKeyboardOptions.copy(keyboardType = KeyboardType.Email),
                        modifier = Modifier.weight(1f),
                    )
                    if (email.isEmpty()) {
                        val clipboardManager = LocalClipboardManager.current
                        IconButton(
                            onClick = {
                                val text = clipboardManager.getText()?.text.orEmpty()
                                if (text.isNotEmpty()) {
                                    onEmailChange(text)
                                }
                            },
                        ) {
                            Icon(
                                Icons.Filled.ContentPaste,
                                contentDescription = tr("common.paste"),
                            )
                        }
                    }
                }
                Row(
                    modifier = Modifier.fillMaxWidth(),
                    verticalAlignment = Alignment.CenterVertically,
                    horizontalArrangement = Arrangement.spacedBy(8.dp),
                ) {
                    OutlinedTextField(
                        value = name,
                        onValueChange = onNameChange,
                        label = { Text(tr("accounts.fields.displayNameMeronOnly")) },
                        singleLine = true,
                        keyboardOptions = nativeTextKeyboardOptions,
                        modifier = Modifier.weight(1f),
                    )
                    if (name.isEmpty()) {
                        val clipboardManager = LocalClipboardManager.current
                        IconButton(
                            onClick = {
                                val text = clipboardManager.getText()?.text.orEmpty()
                                if (text.isNotEmpty()) {
                                    onNameChange(text)
                                }
                            },
                        ) {
                            Icon(
                                Icons.Filled.ContentPaste,
                                contentDescription = tr("common.paste"),
                            )
                        }
                    }
                }
            }
            IconButton(onClick = onRemove) {
                Icon(
                    Icons.Filled.Close,
                    contentDescription = tr("settings.account.removeAlias"),
                    tint = MaterialTheme.colorScheme.onSurfaceVariant,
                )
            }
        }
    }
}

@Composable
internal fun AccountAvatarEditor(
    name: String,
    url: String,
    onPick: (() -> Unit)?,
    modifier: Modifier = Modifier,
    fallbackIcon: ImageVector? = null,
) {
    val side = with(LocalDensity.current) { 64.dp.roundToPx() }
    var image by remember(url, side) { mutableStateOf<ImageBitmap?>(null) }
    LaunchedEffect(url, side) {
        image = null
        if (url.isNotBlank()) {
            image = withContext(ioDispatcher) { loadThumbnailBitmapRef(url, side) }
        }
    }
    Box(
        modifier = modifier.size(64.dp),
        contentAlignment = Alignment.Center,
    ) {
        Box(
            modifier =
                Modifier
                    .size(64.dp)
                    .clip(CircleShape)
                    .then(if (onPick != null) Modifier.clickable(onClick = onPick) else Modifier),
            contentAlignment = Alignment.Center,
        ) {
            val bmp = image
            if (bmp != null) {
                Image(
                    bitmap = bmp,
                    contentDescription = tr("avatar.edit"),
                    modifier = Modifier.size(64.dp).clip(CircleShape),
                    contentScale = ContentScale.Crop,
                )
            } else {
                Avatar(name, 64.dp, fallbackIcon)
            }
        }
        if (onPick != null) {
            Box(
                modifier =
                    Modifier
                        .align(Alignment.BottomEnd)
                        .size(22.dp)
                        .clip(CircleShape)
                        .background(MaterialTheme.colorScheme.primary),
                contentAlignment = Alignment.Center,
            ) {
                Icon(
                    Icons.Filled.Edit,
                    contentDescription = tr("settings.account.changeAvatar"),
                    tint = MaterialTheme.colorScheme.onPrimary,
                    modifier = Modifier.size(13.dp),
                )
            }
        }
    }
}
