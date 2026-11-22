// iOS implements the shared preference interface against
// the platform store so UI choices can survive
// application restarts. The common source set declares
// the interface.

package jp.nonbili.meron.ui

import platform.Foundation.NSUserDefaults

/** NSUserDefaults-backed [AppPreferences]; keys are prefixed by namespace. */
class IosAppPreferences(
    private val namespace: String,
) : AppPreferences {
    private val defaults = NSUserDefaults.standardUserDefaults

    private fun storageKey(key: String) = "$namespace.$key"

    override fun getString(
        key: String,
        default: String,
    ): String = defaults.stringForKey(storageKey(key)) ?: default

    override fun putString(
        key: String,
        value: String,
    ) = defaults.setObject(value, forKey = storageKey(key))

    override fun getBoolean(
        key: String,
        default: Boolean,
    ): Boolean = if (defaults.objectForKey(storageKey(key)) == null) default else defaults.boolForKey(storageKey(key))

    override fun putBoolean(
        key: String,
        value: Boolean,
    ) = defaults.setBool(value, forKey = storageKey(key))

    override fun getInt(
        key: String,
        default: Int,
    ): Int = if (defaults.objectForKey(storageKey(key)) == null) default else defaults.integerForKey(storageKey(key)).toInt()

    override fun putInt(
        key: String,
        value: Int,
    ) = defaults.setInteger(value.toLong(), forKey = storageKey(key))

    override fun getStringSet(
        key: String,
        default: Set<String>,
    ): Set<String> {
        val array = defaults.arrayForKey(storageKey(key)) ?: return default
        return array.filterIsInstance<String>().toSet()
    }

    override fun putStringSet(
        key: String,
        value: Set<String>,
    ) = defaults.setObject(value.toList(), forKey = storageKey(key))

    override fun remove(key: String) = defaults.removeObjectForKey(storageKey(key))
}
