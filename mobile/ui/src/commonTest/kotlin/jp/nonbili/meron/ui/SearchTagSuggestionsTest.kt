// Search completion operates on the tag currently being
// typed. The fixtures exclude tags already present and
// preserve the other terms when a suggestion is chosen.

package jp.nonbili.meron.ui

import kotlin.test.Test
import kotlin.test.assertEquals

class SearchTagSuggestionsTest {
    private val known = listOf("homework", "todo", "work")

    @Test
    fun suggestsByWhatIsTypedWithoutTagsAlreadyListed() {
        assertEquals(listOf("homework", "work"), tagSuggestions(known, "wor"))
        assertEquals(listOf("homework", "todo"), tagSuggestions(known, "work, "))
    }

    @Test
    fun completesOnlyTheTagBeingTyped() {
        assertEquals("todo, work ", completeTag("todo, wo", "work"))
    }
}
