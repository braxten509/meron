// Folder listings supply roles consumed by the desktop
// reply and draft views. The fixtures verify those roles
// remain available after the account folders are loaded.

import { afterEach, beforeEach, describe, expect, it } from 'bun:test'
import type { Folder } from '../types'
// Initialize the account/boot dependency graph before importing mail directly.
import './accounts'
import { mail$ } from './mail'
import { ui$ } from './ui'
import { isDraftFolder, isInboxFolder, loadFolders } from './mailFolders'

describe('loadFolders', () => {
  let previousGo: unknown
  let previousFolders: Folder[]
  let previousFoldersByAccount: Record<string, Folder[]>
  let previousSelectedAccount: string

  beforeEach(() => {
    previousGo = (window as any).go
    previousFolders = mail$.folders.peek()
    previousFoldersByAccount = mail$.foldersByAccount.peek()
    previousSelectedAccount = ui$.selectedAccount.peek()
    ui$.selectedAccount.set('acc')
    mail$.folders.set([])
    mail$.foldersByAccount.set({})
  })

  afterEach(() => {
    mail$.folders.set(previousFolders)
    mail$.foldersByAccount.set(previousFoldersByAccount)
    ui$.selectedAccount.set(previousSelectedAccount)
    if (previousGo === undefined) delete (window as any).go
    else (window as any).go = previousGo
  })

  it.each([{}, { folders: null }])('keeps folder readers usable after an empty response: %j', async (response) => {
    ;(window as any).go = {
      main: { App: { Invoke: async () => response } },
    }

    await loadFolders('acc')

    expect(mail$.folders.get()).toEqual([])
    expect(mail$.foldersByAccount.acc.get()).toEqual([])
    expect(isInboxFolder('INBOX', 'acc')).toBe(true)
    expect(isDraftFolder('Drafts', 'acc')).toBe(true)
  })

  it('keeps server folder roles available to reply and draft readers', async () => {
    const folders: Folder[] = [
      { id: 'received', account_id: 'acc', name: 'Received', role: 'inbox', unread: 2 },
      { id: 'unfinished', account_id: 'acc', name: 'Unfinished', role: 'drafts', unread: 0 },
    ]
    ;(window as any).go = {
      main: { App: { Invoke: async () => ({ folders }) } },
    }

    await loadFolders('acc')

    expect(mail$.folders.get()).toEqual(folders)
    expect(mail$.foldersByAccount.acc.get()).toEqual(folders)
    expect(isInboxFolder('received', 'acc')).toBe(true)
    expect(isDraftFolder('unfinished', 'acc')).toBe(true)
  })

  it('leaves the visible list alone when refreshing another account', async () => {
    const visible: Folder[] = [{ id: 'c', account_id: 'acc', name: 'a/b/c', role: '', unread: 0 }]
    const other: Folder[] = [{ id: 'INBOX', account_id: 'other', name: 'INBOX', role: 'inbox', unread: 1 }]
    mail$.folders.set(visible)
    ;(window as any).go = {
      main: { App: { Invoke: async () => ({ folders: other }) } },
    }

    await loadFolders('other', false)

    expect(mail$.folders.get()).toEqual(visible)
    expect(mail$.foldersByAccount.other.get()).toEqual(other)
  })
})
