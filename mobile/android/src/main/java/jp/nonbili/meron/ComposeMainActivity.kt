// Starts the shared Compose interface inside the Android
// activity. Activity lifecycle and incoming intents
// supply the host events that open or resume a mobile
// workflow.

package jp.nonbili.meron

import android.content.Context
import android.content.Intent
import android.os.Bundle
import android.util.Log
import androidx.activity.ComponentActivity
import androidx.activity.compose.setContent
import androidx.activity.enableEdgeToEdge
import androidx.compose.foundation.ComposeFoundationFlags
import androidx.compose.ui.preferredFrameRate
import androidx.compose.foundation.ExperimentalFoundationApi
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.material.icons.filled.Refresh
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.input.key.key
import androidx.core.splashscreen.SplashScreen.Companion.installSplashScreen
import jp.nonbili.meron.shared.AddPasswordAccountParams
import jp.nonbili.meron.shared.ComposeDraft
import jp.nonbili.meron.shared.accountSummaryIsRss
import jp.nonbili.meron.shared.coreErrorMessage
import jp.nonbili.meron.shared.parseAccountListResponse
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.Job
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.delay
import kotlinx.coroutines.isActive
import kotlinx.coroutines.launch
import kotlinx.coroutines.sync.Mutex
import kotlinx.coroutines.sync.withLock

class ComposeMainActivity : ComponentActivity() {
    private var incomingMailtoDraft by mutableStateOf<ComposeDraft?>(null)
    private var incomingOAuthCallbackUrl by mutableStateOf<String?>(null)
    private var incomingNotificationThreadTarget by mutableStateOf<jp.nonbili.meron.ui.NotificationThreadTarget?>(null)
    private val foregroundEngineScope = CoroutineScope(SupervisorJob() + Dispatchers.IO)
    private val foregroundEngineMutex = Mutex()
    private var foregroundEngineJob: Job? = null

    @Volatile
    private var activityStarted = false

    override fun attachBaseContext(newBase: Context) {
        super.attachBaseContext(localizedAppContext(newBase))
    }

    @OptIn(ExperimentalFoundationApi::class)
    override fun onCreate(savedInstanceState: Bundle?) {
        ComposeFoundationFlags.isNewContextMenuEnabled = true
        installSplashScreen()
        super.onCreate(savedInstanceState)
        enableEdgeToEdge()
        incomingMailtoDraft = intent.toSharedComposeDraft(this)
        incomingOAuthCallbackUrl = intent.toOAuthCallbackUrl()
        incomingNotificationThreadTarget = intent.toNotificationThreadTarget()
        AndroidNotificationService.ensureChannels(this)
        AndroidReminders.refresh(this)
        // The log sink and crash handler are installed process-wide in
        // MeronApplication, before any component (including background sync)
        // starts, so nothing to install here.
        if (!MeronCoreNative.isLoaded()) {
            AndroidSyncDiagnosticLog.append(this, "Rust core failed to load: ${describeCoreLoadFailure(this)}")
        }
        AndroidBackgroundSyncScheduler.sync(this, loadAppBoolean(this, BACKGROUND_SYNC_ENABLED_PREF, true))
        jp.nonbili.meron.ui.androidImageContext = this
        val coreInitJson =
            if (MeronCoreNative.isLoaded()) {
                MeronCoreNative.configureJson(filesDir.absolutePath, MeronDbKey.get(this))
            } else {
                ""
            }
        val appPrefs =
            jp.nonbili.meron.ui
                .AndroidAppPreferences(this, "meron_app")
        val kanbanPrefs =
            jp.nonbili.meron.ui
                .AndroidAppPreferences(this, "meron_kanban")
        val localeController =
            jp.nonbili.meron.ui
                .AndroidLocaleController(this)
        val platformServices = AndroidPlatformServices(this)
        val mobileHost = AndroidMobileHost(this)
        setContent {
            val appCore = remember { JniMeronCore { AndroidNotificationService.requestReconcile(applicationContext) } }
            // Ask for the screen's full rate (120 Hz on the Pixel) while anything
            // moves; with no vote the system treats the app as "normal" and runs it
            // at 60. The vote only applies while the content is being redrawn.
            Box(Modifier.fillMaxSize().preferredFrameRate(120f)) {
            jp.nonbili.meron.ui.MeronApp(
                core = appCore,
                coreLoaded = MeronCoreNative.isLoaded(),
                prefs = appPrefs,
                kanbanPrefs = kanbanPrefs,
                services = platformServices,
                locale = localeController,
                mobileHost = mobileHost,
                coreInitJson = coreInitJson,
                incomingMailtoDraft = incomingMailtoDraft,
                onMailtoDraftConsumed = ::consumeMailtoIntent,
                incomingOAuthCallbackUrl = incomingOAuthCallbackUrl,
                onOAuthCallbackConsumed = ::consumeOAuthIntent,
                incomingNotificationThreadTarget = incomingNotificationThreadTarget,
                // Storing and pushing to the OS both happen in MeronApp; the
                // Activity only has to reload its resources for the new language.
                onLanguageApplied = { recreate() },
                demoAccount = if (BuildConfig.MERON_DEMO) demoTestAccount() else null,
            )
            }
        }
    }

    override fun onNewIntent(intent: Intent) {
        super.onNewIntent(intent)
        setIntent(intent)
        incomingMailtoDraft = intent.toSharedComposeDraft(this)
        incomingOAuthCallbackUrl = intent.toOAuthCallbackUrl()
        incomingNotificationThreadTarget = intent.toNotificationThreadTarget()
    }

    private fun consumeMailtoIntent() {
        incomingMailtoDraft = null
        setIntent(
            Intent(intent).apply {
                action = Intent.ACTION_MAIN
                data = null
                clipData = null
                removeExtra(Intent.EXTRA_EMAIL)
                removeExtra(Intent.EXTRA_CC)
                removeExtra(Intent.EXTRA_BCC)
                removeExtra(Intent.EXTRA_SUBJECT)
                removeExtra(Intent.EXTRA_TEXT)
                removeExtra(Intent.EXTRA_STREAM)
            },
        )
    }

    private fun consumeOAuthIntent() {
        incomingOAuthCallbackUrl = null
        setIntent(
            Intent(intent).apply {
                action = Intent.ACTION_MAIN
                data = null
                clipData = null
            },
        )
    }

    @Deprecated("Deprecated in ComponentActivity; fine for a single fire-and-forget permission request")
    override fun onRequestPermissionsResult(
        requestCode: Int,
        permissions: Array<String>,
        grantResults: IntArray,
    ) {
        super.onRequestPermissionsResult(requestCode, permissions, grantResults)
        if (requestCode == NOTIFICATION_PERMISSION_REQUEST_CODE) {
            AndroidReminders.refresh(this)
            jp.nonbili.meron.ui.NotificationPermissionSignal
                .signal()
        }
    }

    // Host the shared Engine (warm session pool + foreground IMAP IDLE) only while
    // the activity is visible; park it (stop IDLE, drop pooled sockets) when it
    // isn't. Off the main thread since foreground opens the DB and spawns IDLE.
    override fun onStart() {
        super.onStart()
        // System settings may have changed permission or muted the channel.
        AndroidReminders.refresh(this)
        activityStarted = true
        // The background worker syncs while the activity is stopped, and its
        // core events never reach this composition. Announce the return so the
        // visible mailbox re-reads the store instead of showing what was on
        // screen when the user left.
        jp.nonbili.meron.ui.AppForegroundSignal
            .signal()
        foregroundEngineJob?.cancel()
        foregroundEngineJob =
            foregroundEngineScope.launch {
                refreshForegroundManagedTokens()
                if (!isActive || !activityStarted) return@launch
                foregroundEngineMutex.withLock {
                    if (!isActive || !activityStarted) return@withLock
                    invokeEngineLifecycle("engine.foreground")
                }
                // AccountManager access tokens are short-lived. Keep the stored
                // token fresh while this activity remains visible so a later
                // IDLE reconnect does not authenticate with an expired token.
                while (isActive && activityStarted) {
                    delay(FOREGROUND_TOKEN_REFRESH_CHECK_INTERVAL_MS)
                    refreshForegroundManagedTokens()
                }
            }
    }

    override fun onStop() {
        activityStarted = false
        foregroundEngineJob?.cancel()
        foregroundEngineJob = null
        super.onStop()
        // Mail read or archived in the app changes the count the home screen
        // is about to show.
        AndroidUnreadWidget.refreshAsync(this)
        foregroundEngineScope.launch {
            foregroundEngineMutex.withLock {
                // A newer onStart may have won while this stop was queued.
                if (!activityStarted) invokeEngineLifecycle("engine.background")
            }
        }
    }

    /** Refresh Android AccountManager-owned Gmail tokens before any foreground
     *  watcher authenticates. Browser-flow OAuth is refreshed inside Engine. */
    private suspend fun refreshForegroundManagedTokens() {
        if (!MeronCoreNative.isLoaded()) return
        val response =
            runCatching { MeronCoreNative.invokeJson("""{"id":91,"method":"account.list"}""") }
                .getOrElse { error ->
                    logForegroundEngineError("account list failed before foreground watch: ${error.message}")
                    return
                }
        coreErrorMessage(response)?.let { message ->
            logForegroundEngineError("account list failed before foreground watch: $message")
            return
        }
        parseAccountListResponse(response)
            .filterNot { account -> accountSummaryIsRss(account) || account.paused || account.needsReconnect }
            .forEach { account ->
                val refresh =
                    runCatching {
                        GoogleAccountManagerAuth.mintAndPushToken(
                            this,
                            account.id,
                            requestId = 91L,
                            skipIfFresh = true,
                        )
                    }.getOrElse { error ->
                        logForegroundEngineError("token refresh crashed before foreground watch for ${account.id}: ${error.message}")
                        return@forEach
                    }
                when (refresh) {
                    GoogleAccountManagerAuth.TokenRefresh.Failed -> {
                        logForegroundEngineError("token refresh failed before foreground watch for ${account.id}")
                    }

                    GoogleAccountManagerAuth.TokenRefresh.TransientError -> {
                        logForegroundEngineError("token refresh hit a network error before foreground watch for ${account.id}")
                    }

                    GoogleAccountManagerAuth.TokenRefresh.NotNeeded,
                    GoogleAccountManagerAuth.TokenRefresh.StillFresh,
                    is GoogleAccountManagerAuth.TokenRefresh.Refreshed,
                    -> {
                        Unit
                    }
                }
            }
    }

    private fun invokeEngineLifecycle(method: String) {
        if (!MeronCoreNative.isLoaded()) {
            logForegroundEngineError("$method skipped because the Rust core is unavailable")
            return
        }
        val response =
            runCatching {
                MeronCoreNative.invokeJson("{\"id\":92,\"method\":\"$method\",\"params\":{}}")
            }.getOrElse { error ->
                logForegroundEngineError("$method crashed: ${error.message}")
                return
            }
        coreErrorMessage(response)?.let { message ->
            logForegroundEngineError("$method failed: $message")
        }
        // onStop can race a final non-cancellable JNI foreground call. If that
        // happened, park the engine once more so no activity-owned watch leaks.
        if (method == "engine.foreground" && !activityStarted) {
            invokeEngineLifecycle("engine.background")
        }
    }

    private fun logForegroundEngineError(message: String) {
        Log.e(FOREGROUND_ENGINE_TAG, message)
        AndroidSyncDiagnosticLog.appendRedacted(this, "$FOREGROUND_ENGINE_TAG: $message")
    }

    fun currentMailtoDraftForTesting(): ComposeDraft? = incomingMailtoDraft

    fun currentOAuthCallbackUrlForTesting(): String? = incomingOAuthCallbackUrl

    private companion object {
        const val FOREGROUND_ENGINE_TAG = "MeronForegroundEngine"
        const val FOREGROUND_TOKEN_REFRESH_CHECK_INTERVAL_MS = 5 * 60 * 1_000L
    }
}

private fun Intent.toNotificationThreadTarget(): jp.nonbili.meron.ui.NotificationThreadTarget? {
    val account = getStringExtra(AndroidNotificationService.EXTRA_ACCOUNT_ID).orEmpty()
    val folder = getStringExtra(AndroidNotificationService.EXTRA_FOLDER).orEmpty()
    val threadKey = getStringExtra(AndroidNotificationService.EXTRA_THREAD_KEY).orEmpty()
    val fromAddr = getStringExtra(AndroidNotificationService.EXTRA_FROM_ADDR).orEmpty()
    // The group summary carries no thread key: it opens the folder, not a thread.
    if (account.isBlank() || folder.isBlank()) return null
    return jp.nonbili.meron.ui
        .NotificationThreadTarget(accountId = account, folder = folder, threadKey = threadKey, fromAddr = fromAddr)
}

/** The PC's private test mail server, which `adb reverse` brings to this phone's
 *  localhost. Only the demo build uses it; the login is the test server's own. */
private fun demoTestAccount() =
    jp.nonbili.meron.shared.AddPasswordAccountParams(
        email = "alice@mail.test",
        displayName = "Alice",
        senderName = "Alice",
        imapHost = "127.0.0.1",
        imapPort = 35143,
        smtpHost = "127.0.0.1",
        smtpPort = 35587,
        username = "alice@mail.test",
        password = "testpass",
        tls = false,
        starttls = false,
        smtpTls = false,
        smtpStarttls = false,
    )
