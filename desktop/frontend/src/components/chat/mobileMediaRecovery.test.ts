// A missing resource may report its error after a new
// document starts loading. The tests exercise request
// ownership and deduplication while the reader is
// navigating.

import { expect, it, spyOn } from 'bun:test'
import { readFileSync } from 'node:fs'

// Execute the script that both native WebViews ship, including its timing
// around parsing and pending requests. The test DOM can deliver those events
// without depending on an Android emulator or iOS simulator.
const source = readFileSync(
  new URL('../../../../../mobile/ui/src/commonMain/kotlin/jp/nonbili/meron/ui/MailWebView.kt', import.meta.url),
  'utf8',
)
const script = source.match(/internal val RetryFailedMailImagesScript\s*=\s*"""([\s\S]*?)"""/)![1]!.trim()
const run = (doc: Document, window: object, generation: number, revision: number) =>
  new Function(
    'document',
    'window',
    `return ${script.replaceAll('__GENERATION__', String(generation)).replaceAll('__REVISION__', String(revision))}`,
  )(doc, window)

it('keeps recovery alive through parsing and a delayed image error, retrying once', () => {
  const doc = new DOMParser().parseFromString('<html data-meron-media-generation="1"><body></body></html>', 'text/html')
  let ready: DocumentReadyState = 'loading'
  Object.defineProperty(doc, 'readyState', { get: () => ready })
  const window = {}
  expect(run(doc, window, 1, 1)).toBe(true)
  doc.body.innerHTML = '<img src="/media/pending.png">'
  const image = doc.querySelector('img')!
  let complete = false
  Object.defineProperty(image, 'complete', { get: () => complete })
  Object.defineProperty(image, 'naturalWidth', { value: 0 })
  const request = spyOn(image, 'setAttribute')
  ready = 'interactive'
  doc.dispatchEvent(new Event('DOMContentLoaded'))
  expect(request).not.toHaveBeenCalled()
  complete = true
  image.dispatchEvent(new Event('error'))
  expect(request).toHaveBeenCalledTimes(1)
  image.dispatchEvent(new Event('error'))
  expect(request).toHaveBeenCalledTimes(1)
})

it('rejects an outgoing document and avoids duplicate retries across recovery signals', () => {
  const doc = new DOMParser().parseFromString(
    '<html data-meron-media-generation="2"><body><img src="/media/a.png"></body></html>',
    'text/html',
  )
  Object.defineProperty(doc, 'readyState', { value: 'interactive' })
  const image = doc.querySelector('img')!
  Object.defineProperty(image, 'complete', { value: false })
  Object.defineProperty(image, 'naturalWidth', { value: 0 })
  const request = spyOn(image, 'setAttribute')
  const window = {}
  expect(run(doc, window, 1, 1)).toBe(false)
  expect(run(doc, window, 2, 2)).toBe(true)
  expect(run(doc, window, 2, 3)).toBe(true)
  expect(run(doc, window, 2, 3)).toBe(true)
  image.dispatchEvent(new Event('error'))
  expect(request).toHaveBeenCalledTimes(1)
})
