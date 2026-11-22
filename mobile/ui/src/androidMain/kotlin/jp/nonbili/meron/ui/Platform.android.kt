// Android supplies device-specific defaults and services
// requested by the common UI, including clock and input
// behavior. The common source set declares the
// interface.

package jp.nonbili.meron.ui

import android.app.Activity
import android.os.Build
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.material3.ColorScheme
import androidx.compose.material3.dynamicDarkColorScheme
import androidx.compose.material3.dynamicLightColorScheme
import androidx.compose.runtime.Composable
import androidx.compose.runtime.SideEffect
import androidx.compose.ui.platform.LocalConfiguration
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.LocalView
import androidx.compose.ui.text.PlatformTextStyle
import androidx.core.view.WindowCompat

actual fun currentTimeMillis(): Long = System.currentTimeMillis()

actual val platformName: String = "android"

actual val maskPasswordsByDefault: Boolean = true

actual val nativeTextKeyboardOptions: KeyboardOptions = KeyboardOptions.Default

actual val avatarPlatformTextStyle: PlatformTextStyle? =
    PlatformTextStyle(includeFontPadding = false)

@Composable
actual fun SyncSystemBarAppearance(dark: Boolean) {
    val view = LocalView.current
    if (view.isInEditMode) return
    SideEffect {
        val window = (view.context as? Activity)?.window ?: return@SideEffect
        WindowCompat.getInsetsController(window, view).apply {
            isAppearanceLightStatusBars = !dark
            isAppearanceLightNavigationBars = !dark
        }
    }
}

actual val dynamicColorSupported: Boolean = Build.VERSION.SDK_INT >= Build.VERSION_CODES.S

@Composable
actual fun platformDynamicColorScheme(dark: Boolean): ColorScheme? {
    if (!dynamicColorSupported) return null
    val context = LocalContext.current
    // A new wallpaper swaps the system palette and reports it as a configuration
    // change. The activity handles those itself and is not recreated, so read the
    // configuration here to have the palette looked up again.
    LocalConfiguration.current
    return if (dark) dynamicDarkColorScheme(context) else dynamicLightColorScheme(context)
}
