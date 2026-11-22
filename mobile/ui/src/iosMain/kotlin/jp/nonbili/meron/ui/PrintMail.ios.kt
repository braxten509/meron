// iOS connects the shared conversation print action to the
// platform print interface using the message content supplied
// by the caller. The common source set declares the
// interface.

package jp.nonbili.meron.ui

import androidx.compose.runtime.Composable
import androidx.compose.runtime.remember
import androidx.compose.ui.uikit.LocalUIViewController
import kotlinx.cinterop.ExperimentalForeignApi
import platform.UIKit.UIAlertAction
import platform.UIKit.UIAlertActionStyleDefault
import platform.UIKit.UIAlertController
import platform.UIKit.UIAlertControllerStyleAlert
import platform.UIKit.UIDevice
import platform.UIKit.UIMarkupTextPrintFormatter
import platform.UIKit.UIPrintInfo
import platform.UIKit.UIPrintInteractionController
import platform.UIKit.UIUserInterfaceIdiomPad

// UIMarkupTextPrintFormatter has no verified resource-policy enforcement.
// Keep email markup out until a controlled WebKit print renderer replaces it.
internal actual val supportsHtmlMailPrinting = false

@OptIn(ExperimentalForeignApi::class)
@Composable
internal actual fun rememberMailPrinter(errorText: String): (String, String) -> Unit {
    val presenter = LocalUIViewController.current
    val okText = tr("buttons.close")
    return remember(presenter, errorText, okText) {
        { title, html ->
            var errorShown = false

            fun showError() {
                if (errorShown) return
                errorShown = true
                val alert = UIAlertController.alertControllerWithTitle(errorText, message = null, preferredStyle = UIAlertControllerStyleAlert)
                alert.addAction(UIAlertAction.actionWithTitle(okText, style = UIAlertActionStyleDefault, handler = null))
                presenter.presentViewController(alert, animated = true, completion = null)
            }
            val controller = UIPrintInteractionController.sharedPrintController()
            controller.printInfo =
                UIPrintInfo.printInfo().apply {
                    jobName = title
                }
            controller.printFormatter = UIMarkupTextPrintFormatter(markupText = html)
            val completion: (UIPrintInteractionController?, Boolean, platform.Foundation.NSError?) -> Unit = { _, _, error ->
                if (error != null) showError()
            }
            val presented =
                if (UIDevice.currentDevice.userInterfaceIdiom == UIUserInterfaceIdiomPad) {
                    controller.presentFromRect(presenter.view.bounds, inView = presenter.view, animated = true, completionHandler = completion)
                } else {
                    controller.presentAnimated(true, completionHandler = completion)
                }
            if (!presented) showError()
            Unit
        }
    }
}
