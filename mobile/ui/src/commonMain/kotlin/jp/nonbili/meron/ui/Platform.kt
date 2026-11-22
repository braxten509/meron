// Declares platform-specific values and operations
// consumed by the shared mobile UI. Android and iOS
// provide implementations in their respective source
// sets.

package jp.nonbili.meron.ui

import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.material3.ColorScheme
import androidx.compose.runtime.Composable
import androidx.compose.ui.text.PlatformTextStyle

/** Wall-clock time in milliseconds since the Unix epoch. */
expect fun currentTimeMillis(): Long

/**
 * "android" or "ios", as written into a backup envelope next to the app
 * version. Each platform versions independently, so the version alone does not
 * say which build produced a file.
 */
expect val platformName: String

/**
 * Whether password fields should start masked. False on iOS, where masking
 * (PasswordVisualTransformation) suppresses the long-press paste menu in
 * Compose Multiplatform.
 */
expect val maskPasswordsByDefault: Boolean

/**
 * Platform-specific [KeyboardOptions] that opts into native text input on iOS
 * (enabling the system long-press context menu for paste, autofill, etc.).
 * Returns [KeyboardOptions.Default] on Android.
 */
expect val nativeTextKeyboardOptions: KeyboardOptions

/**
 * Android's default font padding shifts small avatar initials off centre.
 * Other platforms use their native text metrics unchanged.
 */
expect val avatarPlatformTextStyle: PlatformTextStyle?

/**
 * Matches the system status/navigation bar icon tint to the app's own theme.
 * The system night mode is not a reliable signal, because the appearance mode
 * is picked in-app: a light theme under system dark mode would otherwise get
 * white icons drawn on a light background.
 */
@Composable
expect fun SyncSystemBarAppearance(dark: Boolean)

/** Whether the system can supply a wallpaper-derived palette (Material You, Android 12+). */
expect val dynamicColorSupported: Boolean

/** The system's dynamic Material scheme, or null when [dynamicColorSupported] is false. */
@Composable
expect fun platformDynamicColorScheme(dark: Boolean): ColorScheme?
