// The settings dialog displays account-specific controls
// alongside application preferences. The account fixtures
// exercise when a custom avatar can be removed from the
// header.

import { afterEach, beforeEach, describe, expect, it } from 'bun:test'
import { act, cleanup, fireEvent, render } from '@testing-library/react'
import { accounts$ } from '../../states/accounts'
import type { Account } from '../../types'
import { AccountPanel } from './SettingsDialog'

const account = (avatar_url: string): Account =>
  ({
    id: 'a1',
    email: 'a1@example.com',
    display_name: 'A One',
    provider: 'custom',
    auth_type: 'password',
    imap_host: 'imap.example.com',
    imap_port: 993,
    smtp_host: 'smtp.example.com',
    smtp_port: 465,
    tls: true,
    avatar_url,
  }) as Account

type Call = { command: string; payload: any }

describe('AccountPanel avatar', () => {
  let calls: Call[] = []

  beforeEach(() => {
    calls = []
    ;(window as any).go = {
      main: {
        App: {
          Invoke: async (command: string, payload: unknown) => {
            calls.push({ command, payload })
            return {}
          },
        },
      },
    }
  })

  afterEach(cleanup)

  it('removes a custom avatar from the settings header', async () => {
    accounts$.set([account('/media/avatars/a1/pic.png')])
    const view = render(<AccountPanel account={accounts$.peek()[0]} />)

    await act(async () => {
      fireEvent.click(view.getByRole('button', { name: 'Remove avatar' }))
    })

    expect(calls.filter((call) => call.command === 'account.setAvatar').map((call) => call.payload)).toEqual([
      { id: 'a1', avatar_url: '' },
    ])
    expect(accounts$.peek()[0].avatar_url).toBe('')
  })

  it('offers no remove button while the account has no custom avatar', () => {
    accounts$.set([account('')])
    const view = render(<AccountPanel account={accounts$.peek()[0]} />)
    expect(view.queryByRole('button', { name: 'Remove avatar' })).toBeNull()
  })
})
