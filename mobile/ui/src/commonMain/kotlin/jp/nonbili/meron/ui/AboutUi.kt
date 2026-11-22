// Displays the app information dialog opened from the mail drawer.
// Release and project information share the same link button.
// Navigation actions return to the host that opened the dialog.

package jp.nonbili.meron.ui

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.Code
import androidx.compose.material.icons.filled.CreditCard
import androidx.compose.material.icons.filled.Favorite
import androidx.compose.material.icons.filled.History
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp

@Composable
internal fun AboutDialog(
    appVersion: String,
    onOpenUrl: (String) -> Unit,
    onShowChangelog: () -> Unit,
    onDismiss: () -> Unit,
) {
    AlertDialog(
        onDismissRequest = onDismiss,
        title = { Text(tr("about.title")) },
        text = {
            Column(verticalArrangement = Arrangement.spacedBy(12.dp)) {
                Text("Meron", style = MaterialTheme.typography.titleMedium, fontWeight = FontWeight.SemiBold)
                Text(
                    tr("about.version", mapOf("version" to appVersion)),
                    style = MaterialTheme.typography.bodyMedium,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                )
                Column(verticalArrangement = Arrangement.spacedBy(10.dp)) {
                    AboutLinkButton(
                        icon = Icons.Filled.Code,
                        label = tr("about.sourceCode"),
                        onClick = { onOpenUrl("https://github.com/nonbili/meron") },
                    )
                    AboutLinkButton(
                        icon = Icons.Filled.History,
                        label = tr("changelog.title"),
                        onClick = onShowChangelog,
                    )
                    Text(
                        tr("about.donate"),
                        style = MaterialTheme.typography.labelLarge,
                        fontWeight = FontWeight.SemiBold,
                        color = MaterialTheme.colorScheme.onSurface,
                    )
                    Column(verticalArrangement = Arrangement.spacedBy(8.dp)) {
                        AboutLinkButton(
                            icon = Icons.Filled.Favorite,
                            label = tr("about.githubSponsors"),
                            onClick = { onOpenUrl("https://github.com/sponsors/rnons") },
                        )
                        AboutLinkButton(
                            icon = Icons.Filled.Favorite,
                            label = tr("about.liberapay"),
                            onClick = { onOpenUrl("https://liberapay.com/rnons") },
                        )
                        AboutLinkButton(
                            icon = Icons.Filled.CreditCard,
                            label = tr("about.paypal"),
                            onClick = { onOpenUrl("https://www.paypal.com/paypalme/nonbili") },
                        )
                        AboutLinkButton(
                            icon = Icons.Filled.CreditCard,
                            label = tr("about.stripe"),
                            onClick = { onOpenUrl("https://donate.stripe.com/dRm8wP0PJ94K4F9a9P48003") },
                        )
                    }
                }
            }
        },
        confirmButton = {
            TextButton(onClick = onDismiss) {
                Text(tr("buttons.close"))
            }
        },
    )
}

@Composable
private fun AboutLinkButton(
    icon: ImageVector,
    label: String,
    onClick: () -> Unit,
) {
    OutlinedButton(
        onClick = onClick,
        modifier = Modifier.fillMaxWidth(),
    ) {
        Row(
            Modifier.fillMaxWidth(),
            verticalAlignment = Alignment.CenterVertically,
        ) {
            Icon(icon, contentDescription = null, modifier = Modifier.size(18.dp))
            Spacer(Modifier.width(8.dp))
            Text(label)
        }
    }
}
