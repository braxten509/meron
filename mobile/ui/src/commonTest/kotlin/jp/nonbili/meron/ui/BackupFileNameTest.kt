// The mobile backup picker suggests a dated filename.
// Fixtures exercise ISO calendar dates, including the Unix
// epoch and leap days, before the name is handed to the
// host.

package jp.nonbili.meron.ui

import kotlin.test.Test
import kotlin.test.assertEquals

class BackupFileNameTest {
    @Test
    fun fileNameCarriesTheDate() {
        // 2026-08-10T00:00:00Z
        assertEquals("meron-backup-2026-08-10.json", backupFileName(1_786_320_000_000L))
    }

    @Test
    fun isoDateHandlesEpochAndLeapDays() {
        assertEquals("1970-01-01", isoDate(0L))
        // Just before midnight UTC still belongs to the previous day.
        assertEquals("1970-01-01", isoDate(86_399_999L))
        assertEquals("1970-01-02", isoDate(86_400_000L))
        // 2024-02-29T12:00:00Z — a leap day, the case the civil-date maths is for.
        assertEquals("2024-02-29", isoDate(1_709_208_000_000L))
        // 2000-02-29T00:00:00Z — a century leap year.
        assertEquals("2000-02-29", isoDate(951_782_400_000L))
        // 2100-03-01T00:00:00Z — 2100 is not a leap year.
        assertEquals("2100-03-01", isoDate(4_107_542_400_000L))
    }

    @Test
    fun isoDateZeroPadsMonthAndDay() {
        // 2026-01-05T00:00:00Z
        assertEquals("2026-01-05", isoDate(1_767_571_200_000L))
    }

    @Test
    fun isoDateHandlesPreEpochInstants() {
        // Dates before 1970 need floor division, not truncation toward zero.
        assertEquals("1969-12-31", isoDate(-1L))
        assertEquals("1969-12-31", isoDate(-86_400_000L))
        assertEquals("1969-12-30", isoDate(-86_400_001L))
    }
}
