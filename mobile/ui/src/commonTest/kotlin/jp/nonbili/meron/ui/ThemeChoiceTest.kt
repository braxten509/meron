// The mobile theme picker stores choices by light or
// dark appearance. The cases check the currently
// displayed theme and the appearance under which an edit
// is saved.

package jp.nonbili.meron.ui

import kotlin.test.Test
import kotlin.test.assertEquals

class ThemeChoiceTest {
    @Test
    fun paintsThePickForTheSystemAppearance() {
        val choice = ThemeChoice(followSystem = true, light = AppAppearanceMode.Mist, dark = AppAppearanceMode.Forest)
        assertEquals(AppAppearanceMode.Mist, choice.resolve(systemDark = false))
        assertEquals(AppAppearanceMode.Forest, choice.resolve(systemDark = true))
        assertEquals(setOf(AppAppearanceMode.Mist, AppAppearanceMode.Forest), choice.chosen)
    }

    @Test
    fun filesAPickUnderItsOwnAppearance() {
        val choice =
            ThemeChoice(followSystem = true)
                .select(AppAppearanceMode.Graphite)
                .select(AppAppearanceMode.Paper)
        assertEquals(AppAppearanceMode.Graphite, choice.dark)
        assertEquals(AppAppearanceMode.Paper, choice.light)
        assertEquals(AppAppearanceMode.Light, choice.fixed)
    }

    @Test
    fun keepsThePaintedThemeWhenToggledEitherWay() {
        val on = ThemeChoice(fixed = AppAppearanceMode.Plum).withFollowSystem(true, systemDark = true)
        assertEquals(AppAppearanceMode.Plum, on.dark)
        assertEquals(AppAppearanceMode.Plum, on.resolve(systemDark = true))

        val off = on.withFollowSystem(false, systemDark = false)
        assertEquals(AppAppearanceMode.Light, off.fixed)
        assertEquals(AppAppearanceMode.Light, off.resolve(systemDark = true))
    }
}
