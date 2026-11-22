// The desktop destination history orders recent
// transfers and promotes reused destinations. The
// fixtures check identity and ordering when a
// destination recurs.

import { afterEach, describe, expect, it } from 'bun:test'
import {
  parseRecentThreadTransfers,
  recentThreadTransfers$,
  rememberThreadTransfer,
  type RecentThreadTransfer,
} from './recentThreadTransfers'

const transfer = (targetFolderId: string, action: 'move' | 'copy' = 'move'): RecentThreadTransfer => ({
  action,
  sourceAccountId: 'acc',
  targetAccountId: 'acc',
  targetFolderId,
})

afterEach(() => {
  recentThreadTransfers$.set([])
  localStorage.removeItem('meron.recentThreadTransfers')
})

describe('recent thread transfers', () => {
  it('keeps most recent first and promotes repeats without duplicates', () => {
    recentThreadTransfers$.set([])
    rememberThreadTransfer(transfer('Work'))
    rememberThreadTransfer(transfer('Receipts', 'copy'))
    rememberThreadTransfer(transfer('Work'))
    expect(recentThreadTransfers$.get()).toEqual([transfer('Work'), transfer('Receipts', 'copy')])
    expect(parseRecentThreadTransfers(localStorage.getItem('meron.recentThreadTransfers'))).toEqual(
      recentThreadTransfers$.get(),
    )
  })

  it('keeps actions and source accounts distinct', () => {
    recentThreadTransfers$.set([])
    rememberThreadTransfer(transfer('Work'))
    rememberThreadTransfer(transfer('Work', 'copy'))
    rememberThreadTransfer({ ...transfer('Work', 'copy'), sourceAccountId: 'other' })
    expect(recentThreadTransfers$.get()).toHaveLength(3)
  })

  it('ignores malformed storage and invalid entries', () => {
    expect(parseRecentThreadTransfers('{')).toEqual([])
    expect(parseRecentThreadTransfers('{}')).toEqual([])
    expect(
      parseRecentThreadTransfers(
        JSON.stringify([null, {}, transfer(''), { ...transfer('Work'), targetAccountId: 'other' }, transfer('Work')]),
      ),
    ).toEqual([{ ...transfer('Work'), targetAccountId: 'other' }, transfer('Work')])
  })

  it('bounds persisted history', () => {
    recentThreadTransfers$.set([])
    for (let i = 0; i < 110; i++) rememberThreadTransfer(transfer(String(i)))
    expect(recentThreadTransfers$.get()).toHaveLength(100)
    expect(recentThreadTransfers$.get()[0].targetFolderId).toBe('109')
  })
})
