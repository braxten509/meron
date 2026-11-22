// Provides the mobile host with a protocol ping and
// a description of its active transport. Startup
// checks can verify what the shared bridge is
// using.

package jp.nonbili.meron.shared

object SharedMobileContract {
    const val protocolVersion: Int = EXPECTED_PROTOCOL_VERSION

    fun pingJson(id: Long = 1): String = pingRequest(id).toJson()

    fun transportName(coreLoaded: Boolean): String = if (coreLoaded) "Rust core" else "Java fallback"
}
