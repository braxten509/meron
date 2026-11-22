// Refreshing a page can remove previously visible
// threads. These fixtures check how incoming rows
// replace or merge with the desktop mailbox page.

import { describe, expect, it } from 'bun:test'
import type { Message } from '../types'
import {
  THREAD_LIST_PAGE_SIZE,
  beginThreadListRead,
  completeRemovedThread,
  mergeRefreshedThreadPage,
  nextSelectedAfterRefresh,
  releaseRemovedThread,
  suppressRemovedThread,
} from './mail'

function row(id: string, date: number): Message {
  return {
    id,
    account_id: 'acc',
    folder_id: 'INBOX',
    thread_id: id,
    from_name: '',
    from_addr: '',
    to: '',
    subject: id,
    preview: '',
    body: '',
    date,
    unread: false,
    starred: false,
    has_attachments: false,
  }
}

describe('mergeRefreshedThreadPage', () => {
  it('drops a thread the refreshed page no longer has', () => {
    const previous = [row('archived', 30), row('kept', 20), row('older', 1)]
    const fetched = [row('kept', 20), row('slid-up', 10)]
    expect(mergeRefreshedThreadPage(previous, fetched).map((thread) => thread.thread_id)).toEqual(['kept', 'slid-up'])
  })

  it('keeps threads the reader scrolled to, past a full first page', () => {
    const fetched = Array.from({ length: THREAD_LIST_PAGE_SIZE }, (_, index) => row(`t${index}`, 200 - index))
    const previous = [row('archived', 180), ...fetched, row('scrolled', 10)]
    expect(mergeRefreshedThreadPage(previous, fetched).map((thread) => thread.thread_id)).toEqual([
      ...fetched.map((thread) => thread.thread_id),
      'scrolled',
    ])
  })

  it('keeps a second-page row tied with the refreshed page boundary', () => {
    const fetched = Array.from({ length: THREAD_LIST_PAGE_SIZE }, (_, index) => row(`t${index}`, 200 - index))
    const boundary = row('page-two', fetched.at(-1)!.date)
    expect(mergeRefreshedThreadPage([...fetched, boundary], fetched)).toEqual([...fetched, boundary])
  })

  it('keeps pinned rows that no longer match a filtered refresh', () => {
    const read = row('read', 30)
    const unread = row('unread', 20)
    expect(mergeRefreshedThreadPage([read, unread], [unread], false, { read: true })).toEqual([read, unread])
  })

  it('puts newly arrived mail first and leaves older mail behind it', () => {
    const previous = [row('old', 10)]
    const fetched = [row('new', 20), row('old', 10)]
    expect(mergeRefreshedThreadPage(previous, fetched).map((thread) => thread.thread_id)).toEqual(['new', 'old'])
  })

  it('does not lift an older thread above newer mail already on screen', () => {
    const previous = [row('oct-1', 200), row('oct-2', 190)]
    const fetched = [row('sept', 100), row('oct-1', 200), row('oct-2', 190)]
    expect(mergeRefreshedThreadPage(previous, fetched).map((thread) => thread.thread_id)).toEqual([
      'oct-1',
      'oct-2',
      'sept',
    ])
  })

  it('drops absent rows from a successful empty page with no cursor', () => {
    expect(mergeRefreshedThreadPage([row('removed', 10)], [])).toEqual([])
  })

  it('preserves only triage pins in a successful empty page', () => {
    const pinned = row('read', 20)
    expect(mergeRefreshedThreadPage([pinned, row('removed', 10)], [], false, { read: true })).toEqual([pinned])
  })

  it('keeps loaded rows when an empty page still has a cursor', () => {
    const previous = [row('older', 10)]
    expect(mergeRefreshedThreadPage(previous, [], true)).toEqual(previous)
  })

  it('keeps scrolled threads when a short page still has more below', () => {
    const previous = [row('top', 30), row('open', 20), row('scrolled', 1)]
    const fetched = [row('top', 30)]
    expect(mergeRefreshedThreadPage(previous, fetched, true).map((thread) => thread.thread_id)).toEqual([
      'top',
      'open',
      'scrolled',
    ])
  })

  it('drops scrolled threads when a short page is the end of the folder', () => {
    const previous = [row('top', 30), row('open', 20), row('scrolled', 1)]
    const fetched = [row('top', 30)]
    expect(mergeRefreshedThreadPage(previous, fetched, false).map((thread) => thread.thread_id)).toEqual(['top'])
  })
})

describe('nextSelectedAfterRefresh', () => {
  it('follows the open conversation to the next row the refresh kept', () => {
    const previous = [row('archived', 30), row('open', 20), row('next', 10)]
    const visible = [row('next', 10)]
    expect(nextSelectedAfterRefresh(previous, visible, 'open')).toBe('next')
  })

  it('steps backward when the open thread was the last row kept', () => {
    const previous = [row('prev', 20), row('open', 10)]
    const visible = [row('prev', 20)]
    expect(nextSelectedAfterRefresh(previous, visible, 'open')).toBe('prev')
  })

  it('leaves a selection that was never in this list', () => {
    const visible = [row('next', 10)]
    expect(nextSelectedAfterRefresh([], visible, 'notification')).toBe('notification')
  })

  it('clears the selection when the refresh kept nothing', () => {
    const previous = [row('open', 10)]
    expect(nextSelectedAfterRefresh(previous, [], 'open')).toBe('')
  })
})

describe('thread list removal guards', () => {
  it('hides a removal from reads already in flight but allows later replies', () => {
    const archived = row('acc#INBOX#t.archived', 5)
    const neighbour = row('acc#INBOX#t.neighbour', 4)
    const before = beginThreadListRead()
    const rollback = suppressRemovedThread(archived.thread_id)
    const during = beginThreadListRead()
    completeRemovedThread(archived.thread_id)
    const after = beginThreadListRead()
    try {
      expect(before.filter([archived, neighbour])).toEqual([neighbour])
      expect(during.filter([archived, neighbour])).toEqual([neighbour])
      expect(after.filter([archived, neighbour])).toEqual([archived, neighbour])
    } finally {
      rollback()
      before.dispose()
      during.dispose()
      after.dispose()
    }
  })

  it('releases in-flight reads when the removal fails or is undone', () => {
    const archived = row('acc#INBOX#t.archived', 5)
    const rollback = suppressRemovedThread(archived.thread_id)
    const read = beginThreadListRead()
    try {
      expect(read.filter([archived])).toEqual([])
      rollback()
      expect(read.filter([archived])).toEqual([archived])
      suppressRemovedThread(archived.thread_id)
      releaseRemovedThread(archived.thread_id)
      expect(read.filter([archived])).toEqual([archived])
    } finally {
      releaseRemovedThread(archived.thread_id)
      read.dispose()
    }
  })
})
