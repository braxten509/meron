// iOS provides the dispatcher used by shared UI code
// for input and output work outside the rendering
// path. The common source set declares the
// interface.

package jp.nonbili.meron.ui

import kotlinx.coroutines.CoroutineDispatcher
import kotlinx.coroutines.Dispatchers

internal actual val ioDispatcher: CoroutineDispatcher = Dispatchers.Default
