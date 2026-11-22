// Implements civil-date arithmetic and calendar grid
// placement for mobile. Month views need date
// positions that remain meaningful across local
// timezone changes.

package jp.nonbili.meron.ui

import jp.nonbili.meron.shared.CalendarItem

/**
 * A calendar date with no zone, for the month grid. Mirrors desktop's
 * lib/calendarGrid.ts: days are local dates; the core sends all-day items as
 * UTC midnights (end exclusive) and timed ones as instants, so the two are
 * placed on days differently.
 */
internal data class CivilDate(
    val year: Int,
    val month: Int,
    val day: Int,
) : Comparable<CivilDate> {
    /** Days since 1970-01-01. */
    val epochDay: Long
        get() = daysFromCivil(year, month, day)

    /** 0 Sunday … 6 Saturday. */
    val weekday: Int
        get() = ((epochDay + 4).mod(7L)).toInt()

    /** This date's UTC midnight, as the core stores all-day bounds. */
    val utcSeconds: Long
        get() = epochDay * DAY_SECONDS

    fun plusDays(days: Long): CivilDate = fromEpochDay(epochDay + days)

    override fun compareTo(other: CivilDate): Int = epochDay.compareTo(other.epochDay)

    companion object {
        fun fromEpochDay(epochDay: Long): CivilDate {
            val (year, month, day) = civilFromDays(epochDay)
            return CivilDate(year, month, day)
        }

        /** The calendar date a UTC midnight names. */
        fun ofUtcSeconds(epochSeconds: Long): CivilDate = fromEpochDay(epochSeconds.floorDiv(DAY_SECONDS))
    }
}

internal const val DAY_SECONDS = 86_400L

/** The six weeks a month view shows for [year]/[month], starting on
 *  [weekStart] (0 Sunday … 6 Saturday). Always 42 days. */
internal fun monthGrid(
    year: Int,
    month: Int,
    weekStart: Int = 0,
): List<CivilDate> {
    val first = CivilDate(year, month, 1)
    val offset = (first.weekday - weekStart).mod(7)
    val start = first.plusDays(-offset.toLong())
    return List(42) { start.plusDays(it.toLong()) }
}

/** The month [months] after [year]/[month]. */
internal fun shiftMonth(
    year: Int,
    month: Int,
    months: Int,
): Pair<Int, Int> {
    val index = year * 12 + (month - 1) + months
    return index.floorDiv(12) to index.mod(12) + 1
}

/**
 * Whether [item] happens on local [day]. [localMidnight] is the epoch second
 * a local date starts at (see [epochSecondsForLocalDate]), passed in so this
 * stays testable.
 */
internal fun occursOn(
    item: CalendarItem,
    day: CivilDate,
    localMidnight: (CivilDate) -> Long,
): Boolean {
    if (item.allDay) {
        val date = day.utcSeconds
        return item.start <= date && date < maxOf(item.end, item.start + DAY_SECONDS)
    }
    val dayStart = localMidnight(day)
    val dayEnd = localMidnight(day.plusDays(1))
    if (item.end <= item.start) return item.start in dayStart until dayEnd
    return item.start < dayEnd && item.end > dayStart
}

/** The range `calendar.events` is asked for a grid: a day either side, for
 *  all-day items in zones east or west of UTC. */
internal fun gridRange(
    days: List<CivilDate>,
    localMidnight: (CivilDate) -> Long,
): Pair<Long, Long> = localMidnight(days.first()) - DAY_SECONDS to localMidnight(days.last().plusDays(1)) + DAY_SECONDS
