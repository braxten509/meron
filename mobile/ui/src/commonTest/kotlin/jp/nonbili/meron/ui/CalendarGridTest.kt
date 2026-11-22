// The mobile calendar converts dates to grid positions
// and back. Cases cover day-of-week calculations and how
// the configured week start anchors a six-week month
// view.

package jp.nonbili.meron.ui

import jp.nonbili.meron.shared.CalendarItem
import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertFalse
import kotlin.test.assertTrue

class CalendarGridTest {
    // A fixed zone for the tests: local midnight is UTC midnight + 9h earlier (UTC+9).
    private val tokyoMidnight = { date: CivilDate -> date.utcSeconds - 9 * 3600 }

    @Test
    fun civilDatesRoundTripAndKnowTheirWeekday() {
        val date = CivilDate(2026, 10, 1)
        assertEquals(1_790_812_800L, date.utcSeconds)
        assertEquals(4, date.weekday, "a Thursday")
        assertEquals(date, CivilDate.ofUtcSeconds(date.utcSeconds + 3600))
        assertEquals(CivilDate(2024, 3, 1), CivilDate(2024, 2, 28).plusDays(2), "through a leap day")
    }

    @Test
    fun aMonthGridIsSixWeeksFromTheWeekStart() {
        val sunday = monthGrid(2026, 10, 0)
        assertEquals(42, sunday.size)
        assertEquals(CivilDate(2026, 9, 27), sunday.first())
        assertEquals(CivilDate(2026, 11, 7), sunday.last())
        val monday = monthGrid(2026, 10, 1)
        assertEquals(CivilDate(2026, 9, 28), monday.first())
        assertEquals(1, monday.first().weekday)
    }

    @Test
    fun monthsShiftAcrossYears() {
        assertEquals(2027 to 1, shiftMonth(2026, 12, 1))
        assertEquals(2025 to 12, shiftMonth(2026, 1, -1))
        assertEquals(2026 to 10, shiftMonth(2026, 10, 0))
    }

    @Test
    fun allDayItemsSitOnTheirUtcDatesAndTimedOnesOnTheLocalDaysTheyTouch() {
        val start = CivilDate(2026, 10, 5).utcSeconds
        val holiday = CalendarItem(id = "h", key = "h", source = "event", title = "Holiday", start = start, end = start + 2 * DAY_SECONDS, allDay = true)
        assertFalse(occursOn(holiday, CivilDate(2026, 10, 4), tokyoMidnight))
        assertTrue(occursOn(holiday, CivilDate(2026, 10, 5), tokyoMidnight))
        assertTrue(occursOn(holiday, CivilDate(2026, 10, 6), tokyoMidnight))
        assertFalse(occursOn(holiday, CivilDate(2026, 10, 7), tokyoMidnight))

        // 22:00 to 02:00 Tokyo time on the 5th.
        val evening = tokyoMidnight(CivilDate(2026, 10, 5)) + 22 * 3600
        val late = CalendarItem(id = "l", key = "l", source = "event", title = "Late", start = evening, end = evening + 4 * 3600)
        assertTrue(occursOn(late, CivilDate(2026, 10, 5), tokyoMidnight))
        assertTrue(occursOn(late, CivilDate(2026, 10, 6), tokyoMidnight))
        assertFalse(occursOn(late, CivilDate(2026, 10, 7), tokyoMidnight))

        val instant = late.copy(end = late.start)
        assertTrue(occursOn(instant, CivilDate(2026, 10, 5), tokyoMidnight))
        assertFalse(occursOn(instant, CivilDate(2026, 10, 6), tokyoMidnight))
    }

    @Test
    fun aGridRangeHasADayToSpare() {
        val days = monthGrid(2026, 10, 0)
        val (from, to) = gridRange(days, tokyoMidnight)
        assertEquals(tokyoMidnight(days.first()) - DAY_SECONDS, from)
        assertEquals(tokyoMidnight(CivilDate(2026, 11, 8)) + DAY_SECONDS, to)
    }
}
