// Checks the destinations offered for moving or copying a
// conversation. Recent targets are scoped to the action and account,
// and choosing a nested folder forwards the selected account.

import { afterEach, expect, it } from 'bun:test'
import { cleanup, fireEvent, render, waitFor } from '@testing-library/react'
import type { Account, Folder } from '../../types'
import { accounts$ } from '../../states/accounts'
import { mail$ } from '../../states/mail'
import { recentThreadTransfers$ } from '../../states/recentThreadTransfers'
import { ThreadMoveCopyItems } from './ThreadMoveCopyItems'

afterEach(() => {
  accounts$.set([])
  mail$.foldersByAccount.set({})
  recentThreadTransfers$.set([])
})

it('lists up to three valid, account-scoped recent targets of each action inside its flyout', async () => {
  accounts$.set([
    { id: 'acc', email: 'me@example.com' },
    { id: 'other', email: 'other@example.com' },
  ] as Account[])
  const folder = (id: string): Folder => ({ id, name: id, account_id: 'acc', role: '', unread: 0 })
  mail$.foldersByAccount.set({ acc: ['INBOX', 'Work', 'Receipts', 'Older', 'A', 'B', 'C'].map(folder) })
  recentThreadTransfers$.set([
    { action: 'move', sourceAccountId: 'other', targetAccountId: 'other', targetFolderId: 'Work' },
    ...['INBOX', 'Deleted', 'Work', 'Receipts', 'Older', 'A', 'B', 'C'].map((targetFolderId) => ({
      action: targetFolderId === 'Receipts' ? ('copy' as const) : ('move' as const),
      sourceAccountId: 'acc',
      targetAccountId: 'acc',
      targetFolderId,
    })),
  ])
  const view = render(
    <ThreadMoveCopyItems threadId="acc#INBOX#thread" accountId="acc" folderId="INBOX" close={() => {}} />,
  )
  try {
    expect(view.getAllByRole('button').map((button) => button.textContent)).toEqual(['Move to...', 'Copy to...'])
    fireEvent.click(view.getByText('Move to...'))
    const titles = () =>
      Array.from(document.querySelectorAll('[data-thread-context-menu] span[title]')).map((span) => span.textContent)
    await waitFor(() => expect(titles()).toEqual(['Work', 'Older', 'A', 'me@example.com', 'other@example.com']))
    fireEvent.mouseLeave(view.getByText('Move to...'))
    fireEvent.mouseEnter(view.getByText('Copy to...'))
    await waitFor(() => expect(titles()).toEqual(['Receipts', 'me@example.com', 'other@example.com']))
  } finally {
    cleanup()
  }
})

it('nests every account in Move to and forwards the selected account', async () => {
  accounts$.set([
    { id: 'acc', email: 'me@example.com' },
    { id: 'other', email: 'other@example.com' },
  ] as Account[])
  const folders = (account_id: string): Folder[] =>
    ['INBOX', 'Work'].map((id) => ({ id, name: id, account_id, role: '', unread: 0 }))
  mail$.foldersByAccount.set({ acc: folders('acc'), other: folders('other') })
  const moves: string[][] = []
  const view = render(
    <ThreadMoveCopyItems
      threadId="acc#INBOX#thread"
      accountId="acc"
      folderId="INBOX"
      close={() => {}}
      onMove={(...args) => {
        moves.push(args)
      }}
    />,
  )
  try {
    fireEvent.click(view.getByText('Move to...'))
    // Each account opens its folders one level further out; the thread's own
    // account leaves out its source INBOX.
    fireEvent.click(view.getByText('me@example.com'))
    await waitFor(() => expect(view.getByText('Work')).toBeTruthy())
    expect(view.queryByText('INBOX')).toBeNull()
    fireEvent.click(view.getByText('me@example.com'))
    fireEvent.click(view.getByText('other@example.com'))
    fireEvent.click(view.getByText('INBOX'))
    expect(moves).toEqual([['acc#INBOX#thread', 'other', 'INBOX']])
  } finally {
    cleanup()
  }
})
