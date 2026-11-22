// Declares the coroutine dispatcher used for
// blocking mobile I/O. Each native source set
// supplies a dispatcher appropriate for its
// platform.

package jp.nonbili.meron.ui

import kotlinx.coroutines.CoroutineDispatcher

/** Dispatcher for blocking core (JNI/FFI) and file work. */
internal expect val ioDispatcher: CoroutineDispatcher
