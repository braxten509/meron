// Checks how desktop RPC retries recognize a sidecar timeout.
// Retryable timeouts can precede a successful answer,
// while other errors follow their ordinary failure path.

import { afterEach, describe, expect, it } from 'bun:test'
import { invokeRetryingTimeout, isSidecarTimeout } from './bridge'

const original = (window as any).go

afterEach(() => {
  ;(window as any).go = original
})

function mockInvoke(handler: (command: string) => unknown) {
  const calls: string[] = []
  ;(window as any).go = {
    main: {
      App: {
        Invoke: async (command: string) => {
          calls.push(command)
          return handler(command)
        },
      },
    },
  }
  return calls
}

describe('isSidecarTimeout', () => {
  it('matches only the bridge timeout', () => {
    expect(isSidecarTimeout(new Error('sidecar account.list timeout after 5.004s'))).toBe(true)
    expect(isSidecarTimeout('sidecar app.prefsGet timeout after 5s')).toBe(true)
    expect(isSidecarTimeout(new Error('sidecar account.list error: store init failed'))).toBe(false)
  })
})

describe('invokeRetryingTimeout', () => {
  it('retries timeouts until the core answers', async () => {
    let failures = 2
    const calls = mockInvoke(() => {
      if (failures-- > 0) throw new Error('sidecar account.list timeout after 5.004s')
      return { accounts: [] }
    })
    expect(await invokeRetryingTimeout<{ accounts: unknown[] }>('account.list')).toEqual({ accounts: [] })
    expect(calls.length).toBe(3)
  })

  it('does not retry the core’s own errors', async () => {
    const calls = mockInvoke(() => {
      throw new Error('sidecar account.list error: boom')
    })
    await expect(invokeRetryingTimeout('account.list')).rejects.toThrow('boom')
    expect(calls.length).toBe(1)
  })

  it('gives up after the last attempt', async () => {
    const calls = mockInvoke(() => {
      throw new Error('sidecar account.list timeout after 5s')
    })
    await expect(invokeRetryingTimeout('account.list', {}, 3)).rejects.toThrow('timeout')
    expect(calls.length).toBe(3)
  })
})
