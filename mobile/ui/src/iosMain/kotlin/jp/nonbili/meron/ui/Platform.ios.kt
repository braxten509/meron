// iOS supplies device-specific defaults and services
// requested by the common UI, including clock and input
// behavior. The common source set declares the
// interface.

package jp.nonbili.meron.ui

import androidx.compose.foundation.ComposeFoundationFlags
import androidx.compose.foundation.ExperimentalFoundationApi
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.material3.ColorScheme
import androidx.compose.runtime.Composable
import androidx.compose.ui.ExperimentalComposeUiApi
import androidx.compose.ui.text.PlatformTextStyle
import androidx.compose.ui.text.input.PlatformImeOptions
import platform.Foundation.NSDate
import platform.Foundation.timeIntervalSince1970

actual fun currentTimeMillis(): Long = (NSDate().timeIntervalSince1970 * 1000.0).toLong()

actual val platformName: String = "ios"

actual val maskPasswordsByDefault: Boolean = false

@OptIn(ExperimentalComposeUiApi::class, ExperimentalFoundationApi::class)
actual val nativeTextKeyboardOptions: KeyboardOptions =
    run {
        ComposeFoundationFlags.isNewContextMenuEnabled = true
        KeyboardOptions(
            platformImeOptions =
                PlatformImeOptions {
                    usingNativeTextInput(true)
                },
        )
    }

actual val avatarPlatformTextStyle: PlatformTextStyle? = null

/** iOS derives the status bar style from the hosting view controller. */
@Composable
actual fun SyncSystemBarAppearance(dark: Boolean) = Unit

actual val dynamicColorSupported: Boolean = false

@Composable
actual fun platformDynamicColorScheme(dark: Boolean): ColorScheme? = null
