// Editing or reconnecting an account must keep its
// original identity. These cases cover form state and
// the account banner after a failed or successful
// reconnect.

import { afterEach, beforeEach, describe, expect, it, jest } from 'bun:test'
import { act, cleanup, renderHook } from '@testing-library/react'
import { accounts$ } from '../../states/accounts'
import { connectivity$, dismissSyncError, setSyncError } from '../../states/connectivity'
import { ui$ } from '../../states/ui'
import type { Account } from '../../types'
import { useAccountDialog } from './useAccountDialog'

const account: Account = {
  id: 'legacy-user1-mail-localhost',
  email: 'user1@mail.localhost',
  display_name: 'Elena Rostova',
  provider: 'custom',
  auth_type: 'password',
  imap_host: '127.0.0.1',
  imap_port: 1143,
  smtp_host: '127.0.0.1',
  smtp_port: 1025,
  tls: false,
  needs_reconnect: true,
}

describe('account reconnect and edit', () => {
  let saved: any
  let fail: boolean
  let previousGo: PropertyDescriptor | undefined
  let profileEmail: string

  beforeEach(() => {
    saved = null
    fail = false
    profileEmail = account.email
    previousGo = Object.getOwnPropertyDescriptor(window, 'go')
    accounts$.set([account])
    ui$.setupMode.set('custom')
    ui$.reconnectAccountId.set(account.id)
    ui$.setupOpen.set(true)
    ui$.settingsOpen.set(false)
    ui$.system.set(null)
    setSyncError(account.id, 'old connection failure')
    ;(window as any).go = {
      main: {
        App: {
          Invoke: async (command: string, payload: any) => {
            if (
              command === 'account.addPassword' ||
              command === 'account.addGmailOAuth' ||
              command === 'account.addOutlookOAuth'
            ) {
              saved = payload
              if (fail) throw new Error('reconnect failed')
              return { account: { id: payload.id ?? payload.email } }
            }
            if (command === 'oauth.gmailBegin' || command === 'oauth.outlookBegin')
              return { url: 'https://example.com/oauth', needs_external_browser: true }
            if (command === 'oauth.gmailPollProfile' || command === 'oauth.outlookPollProfile')
              return {
                exchanged: true,
                profile: {
                  email: profileEmail,
                  display_name: 'User',
                  access_token: 'access',
                  refresh_token: 'refresh',
                  expires_in: 3600,
                },
              }
            if (command === 'account.list') return { accounts: [{ ...account, needs_reconnect: false }] }
            if (command === 'app.prefsGet') return { prefs: {} }
            if (command === 'mailto.consumePending') return []
            return {}
          },
        },
      },
    }
  })

  afterEach(() => {
    cleanup()
    jest.useRealTimers()
    dismissSyncError()
    ui$.reconnectAccountId.set('')
    if (previousGo) Object.defineProperty(window, 'go', previousGo)
    else delete (window as any).go
  })

  async function reconnect() {
    const hook = renderHook(() => useAccountDialog())
    await act(async () => hook.result.current.setForm((form) => ({ ...form, password: 'new-password' })))
    await act(async () => {
      await hook.result.current.save()
    })
    return hook
  }

  it('retains the original account ID and clears its banner', async () => {
    await reconnect()
    expect(saved.id).toBe(account.id)
    expect(accounts$.peek().map((entry) => entry.id)).toEqual([account.id])
    expect(connectivity$.error.peek()).toBeNull()
  })

  it('keeps the banner when reconnect fails', async () => {
    fail = true
    const hook = await reconnect()
    expect(hook.result.current.error).toBe('reconnect failed')
    expect(connectivity$.error.peek()).toBe('old connection failure')
  })

  it('keeps errors belonging to a different account', async () => {
    setSyncError('other-account', 'other connection failure')
    await reconnect()
    expect(connectivity$.error.peek()).toBe('other connection failure')
    expect(connectivity$.account.peek()).toBe('other-account')
  })

  it('creates a separate account when the email changes and keeps the original banner', async () => {
    const hook = renderHook(() => useAccountDialog())
    await act(async () =>
      hook.result.current.setForm((form) => ({ ...form, email: 'other@example.com', password: 'new-password' })),
    )
    await act(async () => {
      await hook.result.current.save()
    })
    expect(saved).not.toHaveProperty('id')
    expect(connectivity$.account.peek()).toBe(account.id)
    expect(connectivity$.error.peek()).toBe('old connection failure')
  })

  it('omits the password only when editing the same mailbox', async () => {
    accounts$.set([{ ...account, needs_reconnect: false }])
    const hook = renderHook(() => useAccountDialog())
    expect(hook.result.current.saveDisabled).toBe(false)
    await act(async () => {
      await hook.result.current.save()
    })
    expect(saved.id).toBe(account.id)
    expect(saved).not.toHaveProperty('password')
  })

  it('requires a new password when an edit changes the mailbox', async () => {
    accounts$.set([{ ...account, needs_reconnect: false }])
    const hook = renderHook(() => useAccountDialog())
    await act(async () => hook.result.current.setForm((form) => ({ ...form, email: 'other@example.com' })))
    expect(hook.result.current.saveDisabled).toBe(true)
  })

  for (const provider of ['gmail', 'outlook'] as const) {
    for (const sameMailbox of [true, false]) {
      it(`${provider} polling ${sameMailbox ? 'reuses the ID and clears the banner for the same mailbox' : 'keeps the original account and banner for a different sign-in'}`, async () => {
        jest.useFakeTimers()
        ui$.setupMode.set(provider)
        profileEmail = sameMailbox ? ` ${account.email.toUpperCase()} ` : 'other@example.com'
        const hook = renderHook(() => useAccountDialog())
        await act(async () => {
          await hook.result.current.beginOAuth(provider)
        })
        await act(async () => {
          jest.advanceTimersByTime(1000)
        })
        expect(saved).not.toBeNull()
        if (sameMailbox) {
          expect(saved.id).toBe(account.id)
          expect(connectivity$.error.peek()).toBeNull()
        } else {
          expect(saved).not.toHaveProperty('id')
          expect(connectivity$.error.peek()).toBe('old connection failure')
        }
      })
    }
  }
})
