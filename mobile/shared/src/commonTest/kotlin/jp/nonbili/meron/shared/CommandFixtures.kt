// A recording core for shared mobile command tests.
// Each invocation captures the command and payload and returns the fixture response.
// The coroutine helper runs calls to this synchronous test double.
package jp.nonbili.meron.shared

import kotlin.coroutines.Continuation
import kotlin.coroutines.EmptyCoroutineContext
import kotlin.coroutines.startCoroutine

internal class RecordingCommandCore(
    private val response: String,
) : MeronCore {
    var lastCommand = ""
    var lastPayloadJson = ""

    override suspend fun invoke(
        command: String,
        payloadJson: String,
    ): String {
        lastCommand = command
        lastPayloadJson = payloadJson
        return response
    }

    override fun events(): CoreEventStream =
        object : CoreEventStream {
            override fun subscribe(listener: (CoreEvent) -> Unit): CloseableHandle = CloseableHandle {}
        }

    override suspend fun protocolVersion(): Int = EXPECTED_PROTOCOL_VERSION
}

internal fun <T> runCommandFixture(block: suspend () -> T): T {
    var value: T? = null
    var error: Throwable? = null
    block.startCoroutine(
        Continuation(EmptyCoroutineContext) {
            it.onSuccess { output -> value = output }
            it.onFailure { thrown -> error = thrown }
        },
    )
    error?.let { throw it }
    @Suppress("UNCHECKED_CAST")
    return value as T
}
