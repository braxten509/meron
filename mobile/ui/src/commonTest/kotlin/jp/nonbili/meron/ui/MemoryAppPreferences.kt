// Provides an in-memory preference store for mobile UI tests.
// Typed setters and getters let state fixtures round-trip
// values without writing to a real device's settings.

package jp.nonbili.meron.ui

internal class MemoryAppPreferences : AppPreferences {
    var stringWrites = 0
        private set

    private val strings = mutableMapOf<String, String>()
    private val booleans = mutableMapOf<String, Boolean>()
    private val ints = mutableMapOf<String, Int>()
    private val stringSets = mutableMapOf<String, Set<String>>()

    override fun getString(
        key: String,
        default: String,
    ): String = strings[key] ?: default

    override fun putString(
        key: String,
        value: String,
    ) {
        stringWrites += 1
        strings[key] = value
    }

    override fun getBoolean(
        key: String,
        default: Boolean,
    ): Boolean = booleans[key] ?: default

    override fun putBoolean(
        key: String,
        value: Boolean,
    ) {
        booleans[key] = value
    }

    override fun getInt(
        key: String,
        default: Int,
    ): Int = ints[key] ?: default

    override fun putInt(
        key: String,
        value: Int,
    ) {
        ints[key] = value
    }

    override fun getStringSet(
        key: String,
        default: Set<String>,
    ): Set<String> = stringSets[key] ?: default

    override fun putStringSet(
        key: String,
        value: Set<String>,
    ) {
        stringSets[key] = value
    }

    override fun remove(key: String) {
        strings.remove(key)
        booleans.remove(key)
        ints.remove(key)
        stringSets.remove(key)
    }
}
