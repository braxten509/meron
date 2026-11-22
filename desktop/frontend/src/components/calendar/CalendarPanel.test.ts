// Calendar entries can start and finish on the same day
// or cross its boundary. The fixtures exercise the time
// span shown next to an event in the desktop panel.

import { describe, expect, it } from 'bun:test'
import type { CalendarItem } from '../../states/calendar'
import { timeSpan } from './CalendarPanel'

const at = (day: number, hour: number) => new Date(2026, 9, day, hour).getTime() / 1000

const item = (start: number, end: number): CalendarItem => ({
  id: 'e1',
  key: 'e1',
  source: 'event',
  title: 'Offsite',
  notes: '',
  location: '',
  start,
  end,
  all_day: false,
  recurring: false,
  repeat: '',
  account: '',
  thread_id: '',
  message_id: '',
  response: '',
})

const clock = (day: number, hour: number) =>
  new Date(2026, 9, day, hour).toLocaleTimeString([], { hour: '2-digit', minute: '2-digit', hour12: false })
const dated = (day: number, hour: number) =>
  new Date(2026, 9, day, hour).toLocaleString([], {
    month: 'short',
    day: 'numeric',
    hour: '2-digit',
    minute: '2-digit',
    hour12: false,
  })

describe('timeSpan', () => {
  it('shows only times for an event within the day', () => {
    expect(timeSpan(item(at(26, 9), at(26, 17)), new Date(2026, 9, 26))).toBe(`${clock(26, 9)} – ${clock(26, 17)}`)
  })

  it('dates the ends of a multi-day event that fall on other days', () => {
    const offsite = item(at(26, 9), at(28, 17))
    expect(timeSpan(offsite, new Date(2026, 9, 26))).toBe(`${clock(26, 9)} – ${dated(28, 17)}`)
    expect(timeSpan(offsite, new Date(2026, 9, 27))).toBe(`${dated(26, 9)} – ${dated(28, 17)}`)
    expect(timeSpan(offsite, new Date(2026, 9, 28))).toBe(`${dated(26, 9)} – ${clock(28, 17)}`)
  })

  it('reads an end at midnight as the close of the day before', () => {
    expect(timeSpan(item(at(26, 22), at(27, 0)), new Date(2026, 9, 26))).toBe(`${clock(26, 22)} – ${clock(27, 0)}`)
  })
})
