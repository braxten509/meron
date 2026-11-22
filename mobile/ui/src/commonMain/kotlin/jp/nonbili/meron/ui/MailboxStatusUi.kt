// Shows connection and notification status for the current mailbox.
// Account failures can open reconnection, proxy or certificate settings.
// The mailbox supplies the selected account and owns subsequent retries.

package jp.nonbili.meron.ui

import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.Settings
import androidx.compose.material.icons.outlined.NotificationsNone
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.runtime.Composable
import androidx.compose.ui.ExperimentalComposeUiApi
import jp.nonbili.meron.shared.AccountSummary
import jp.nonbili.meron.shared.untrustedCertificateProtocol

@OptIn(ExperimentalMaterial3Api::class, ExperimentalComposeUiApi::class)
@Composable
internal fun MeronMobileState.MailboxStatus(selectedAccount: AccountSummary?) {
    val disconnectedAccount = selectedAccount?.takeIf { it.needsReconnect }
    when {
        disconnectedAccount != null -> {
            StatusBanner(
                message =
                    tr(
                        "mobile.status.cantSignIn",
                        mapOf("account" to disconnectedAccount.email.ifBlank { disconnectedAccount.displayName }),
                    ),
                isError = true,
                actionLabel = tr("settings.account.reconnectButton"),
                onAction = { reconnectAccount(disconnectedAccount) },
                onDismiss = null,
            )
        }

        syncError != null -> {
            val failure = syncError!!
            val failedAccount = failure.accountId?.let { id -> coreAccounts.firstOrNull { it.id == id } }
            val accountLabel = mobileConnectivityAccountLabel(failure.accountId, coreAccounts)
            val proxyEndpoint = mobileProxyEndpointFromSyncError(failure.message)
            val authLike = isAuthError(failure.message)
            // A certificate we cannot validate is not a retryable
            // failure: nothing changes until the user looks at the
            // certificate and decides to trust it.
            val untrustedCert =
                failedAccount != null && untrustedCertificateProtocol(failure.message) != null
            StatusBanner(
                message =
                    when {
                        untrustedCert -> {
                            tr("accounts.certificate.title")
                        }

                        proxyEndpoint != null && accountLabel != null -> {
                            tr(
                                "connectivity.proxyFailedAccount",
                                mapOf("proxy" to proxyEndpoint, "account" to accountLabel),
                            )
                        }

                        accountLabel != null -> {
                            tr("connectivity.syncFailedAccount", mapOf("account" to accountLabel))
                        }

                        else -> {
                            tr("connectivity.syncFailed")
                        }
                    },
                isError = true,
                actionLabel =
                    when {
                        untrustedCert -> tr("accounts.certificate.show")
                        proxyEndpoint != null -> tr("settings.network.proxy")
                        authLike && failedAccount != null -> tr("settings.account.reconnectButton")
                        else -> tr("connectivity.retry")
                    },
                onAction = {
                    when {
                        untrustedCert -> {
                            showServerCertificate(failedAccount!!.id, failure.message)
                        }

                        proxyEndpoint != null -> {
                            previousTopScreen = Screen.Mail
                            val customAccountProxy =
                                failedAccount?.proxy?.mode == "http" ||
                                    failedAccount?.proxy?.mode == "socks5"
                            accountSettingsTargetId = failedAccount?.id?.takeIf { customAccountProxy }
                            accountSettingsProxyTargetId = accountSettingsTargetId
                            settingsGeneralTarget = !customAccountProxy
                            screen = Screen.Settings
                        }

                        authLike && failedAccount != null -> {
                            reconnectAccount(failedAccount)
                        }

                        else -> {
                            syncCoreThreads()
                        }
                    }
                },
                onDismiss = { syncError = null },
            )
        }

        errorBanner != null -> {
            val authLike = isAuthError(errorBanner!!)
            val certificateAccountId = certificateErrorAccountId(selectedAccount?.id, pendingCertificateRetry)
            val certificateAccount = coreAccounts.firstOrNull { it.id == certificateAccountId }
            // Send failures land here too, so this is where an
            // untrusted submission-server certificate shows up.
            val untrustedCert =
                certificateAccount != null && untrustedCertificateProtocol(errorBanner!!) != null
            StatusBanner(
                message = if (untrustedCert) tr("accounts.certificate.title") else errorBanner!!,
                isError = true,
                actionLabel =
                    when {
                        untrustedCert -> tr("accounts.certificate.show")
                        authLike && selectedAccount != null -> tr("settings.account.reconnectButton")
                        else -> tr("connectivity.retry")
                    },
                onAction = {
                    when {
                        untrustedCert -> {
                            showServerCertificate(
                                certificateAccount.id,
                                errorBanner!!,
                                pendingCertificateRetry,
                            )
                        }

                        authLike && selectedAccount != null -> {
                            reconnectAccount(selectedAccount)
                        }

                        else -> {
                            syncCoreThreads()
                        }
                    }
                },
                onDismiss = { errorBanner = null },
            )
        }

        !notificationPermissionGranted &&
            !notificationBannerDismissed &&
            coreAccounts.isNotEmpty() &&
            initialThreadsLoaded &&
            !syncing -> {
            StatusBanner(
                message = tr("mobile.mail.notificationsBannerText"),
                isError = false,
                icon = Icons.Outlined.NotificationsNone,
                actionLabel = tr("mobile.mail.notificationsBannerEnable"),
                onAction = {
                    mobileHost.requestNotificationPermission()
                    notificationPermissionGranted = mobileHost.notificationsEnabled()
                },
                onDismiss = {
                    notificationBannerDismissed = true
                    saveAppBoolean(prefs, NOTIFICATION_BANNER_DISMISSED_PREF, true)
                },
            )
        }
    }
}
