// Simulates media recovery arriving before an image or video
// reports its load result. Failed elements retry once and
// elements that finish loading keep their existing DOM nodes.

import { afterEach, expect, it } from 'bun:test'
import { cleanup, fireEvent, render } from '@testing-library/react'
import { RetryWhenRestored } from './RetryWhenRestored'

afterEach(cleanup)

for (const tag of ['img', 'video'] as const) {
  const child = tag === 'img' ? <img src="/media/missing.png" /> : <video src="/media/missing.mp4" />
  const view = (mediaMissing: number) => <RetryWhenRestored mediaMissing={mediaMissing}>{child}</RetryWhenRestored>

  it(`retries a pending ${tag} once when recovery precedes its error`, () => {
    const { container, rerender } = render(view(2))
    const original = container.querySelector(tag)!
    Object.defineProperty(original, tag === 'img' ? 'complete' : 'readyState', { value: tag === 'img' ? false : 0 })
    rerender(view(1))
    rerender(view(0))
    expect(container.querySelector(tag)).toBe(original)
    fireEvent.error(original)
    const retry = container.querySelector(tag)!
    expect(retry).not.toBe(original)
    fireEvent.error(retry)
    expect(container.querySelector(tag)).toBe(retry)
  })

  it(`leaves a ${tag} that succeeds after recovery mounted`, () => {
    const { container, rerender } = render(view(1))
    const original = container.querySelector(tag)!
    Object.defineProperty(original, tag === 'img' ? 'complete' : 'readyState', { value: tag === 'img' ? false : 0 })
    rerender(view(0))
    if (tag === 'img') fireEvent.load(original)
    else fireEvent.loadedMetadata(original)
    fireEvent.error(original)
    expect(container.querySelector(tag)).toBe(original)
  })

  it(`retries an already failed ${tag} when files return`, () => {
    const { container, rerender } = render(view(1))
    const original = container.querySelector(tag)!
    fireEvent.error(original)
    rerender(view(0))
    expect(container.querySelector(tag)).not.toBe(original)
  })
}
