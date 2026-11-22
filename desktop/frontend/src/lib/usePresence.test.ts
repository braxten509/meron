// Panels use presence state to remain mounted during an
// exit animation. Tests cover the initial settled state
// and immediate changes when animation is disabled.

import { afterEach, beforeEach, describe, expect, it, jest } from 'bun:test'
import { StrictMode } from 'react'
import { act, renderHook } from '@testing-library/react'
import { usePresence } from './usePresence'

const setReducedMotion = (reduce: boolean) => {
  ;(globalThis as any).matchMedia = (query: string) => ({
    matches: reduce && query.includes('prefers-reduced-motion'),
    addEventListener: () => {},
    removeEventListener: () => {},
  })
}

const originalMatchMedia = (globalThis as any).matchMedia

describe('usePresence', () => {
  beforeEach(() => {
    jest.useFakeTimers()
    setReducedMotion(false)
  })

  afterEach(() => {
    jest.useRealTimers()
    ;(globalThis as any).matchMedia = originalMatchMedia
  })

  const advance = (ms: number) =>
    act(() => {
      jest.advanceTimersByTime(ms)
    })

  it('starts settled, without animating what is already open at mount', () => {
    expect(renderHook(() => usePresence(true, 200)).result.current).toBe('open')
    expect(renderHook(() => usePresence(false, 200)).result.current).toBe('closed')
  })

  it('settles at once on a change made without animating', () => {
    const view = renderHook(({ open, animate }) => usePresence(open, 200, animate), {
      initialProps: { open: false, animate: false },
    })
    view.rerender({ open: true, animate: false })
    expect(view.result.current).toBe('open')
    view.rerender({ open: false, animate: false })
    expect(view.result.current).toBe('closed')
    view.rerender({ open: true, animate: true })
    expect(view.result.current).toBe('entering')
  })

  it('does not animate at mount under StrictMode', () => {
    const closed = renderHook(() => usePresence(false, 200), { wrapper: StrictMode })
    expect(closed.result.current).toBe('closed')
    const open = renderHook(() => usePresence(true, 200), { wrapper: StrictMode })
    expect(open.result.current).toBe('open')
    advance(200)
    expect(closed.result.current).toBe('closed')
    expect(open.result.current).toBe('open')
  })

  it('enters, then settles open after the duration', () => {
    const view = renderHook(({ open }) => usePresence(open, 200), { initialProps: { open: false } })
    view.rerender({ open: true })
    expect(view.result.current).toBe('entering')
    advance(199)
    expect(view.result.current).toBe('entering')
    advance(1)
    expect(view.result.current).toBe('open')
  })

  it('stays mounted while exiting, then closes after the duration', () => {
    const view = renderHook(({ open }) => usePresence(open, 200), { initialProps: { open: true } })
    view.rerender({ open: false })
    expect(view.result.current).toBe('exiting')
    advance(200)
    expect(view.result.current).toBe('closed')
  })

  it('reopening mid-exit enters again instead of finishing the close', () => {
    const view = renderHook(({ open }) => usePresence(open, 200), { initialProps: { open: true } })
    view.rerender({ open: false })
    advance(100)
    view.rerender({ open: true })
    expect(view.result.current).toBe('entering')
    advance(200)
    expect(view.result.current).toBe('open')
  })

  it('skips the animation phases when the system asks for reduced motion', () => {
    setReducedMotion(true)
    const view = renderHook(({ open }) => usePresence(open, 200), { initialProps: { open: false } })
    view.rerender({ open: true })
    expect(view.result.current).toBe('open')
    view.rerender({ open: false })
    expect(view.result.current).toBe('closed')
  })
})
