// Checks reader media sizing against sender-defined
// limits. Fixtures cover intrinsic sizes, wide images and
// repeated setup of resize listeners on the same element.

import { describe, expect, it } from 'bun:test'
import { fitMediaBox } from './mediaSize'
import { releaseFailedImageBox, restoreImageBox } from './bubbleHtml'

describe('fitMediaBox', () => {
  it('keeps the original sender limit and only the latest listener on repeated setup', () => {
    const image = document.createElement('img')
    image.width = 571
    image.height = 954
    fitMediaBox(image, '320px')
    const cleanup = fitMediaBox(image, '640px')
    expect(image.style.getPropertyValue('max-width')).toBe(`min(100%, calc(640px * ${571 / 954}))`)

    Object.defineProperties(image, {
      naturalWidth: { value: 954 },
      naturalHeight: { value: 571 },
    })
    cleanup()
    image.dispatchEvent(new Event('load'))
    // The first setup's listener was removed too.
    expect(image.style.getPropertyValue('max-width')).toBe(`min(100%, calc(640px * ${571 / 954}))`)
  })

  it('does not put intrinsic sizing keywords inside a CSS calculation', () => {
    for (const keyword of ['max-content', 'min-content', 'fit-content', '-webkit-fill-available', 'stretch']) {
      const image = document.createElement('img')
      image.style.maxWidth = keyword
      image.width = 571
      image.height = 954
      fitMediaBox(image, '320px')
      expect(image.style.getPropertyValue('max-width')).toBe(`min(100%, calc(320px * ${571 / 954}))`)
    }
  })

  it('preserves a smaller inline limit on a wide image', () => {
    const image = document.createElement('img')
    image.style.setProperty('max-width', '200px', 'important')
    Object.defineProperties(image, {
      naturalWidth: { value: 1600 },
      naturalHeight: { value: 400 },
    })
    fitMediaBox(image, '320px')
    expect(image.style.getPropertyValue('max-width')).toBe('min(100%, 200px, calc(320px * 4))')
  })

  it('preserves a smaller limit from a sender stylesheet', () => {
    const style = document.createElement('style')
    style.textContent = '.sender-logo { max-width: 200px !important; }'
    const image = document.createElement('img')
    image.className = 'sender-logo'
    image.width = 1600
    image.height = 400
    document.head.append(style)
    document.body.append(image)
    try {
      fitMediaBox(image, '320px')
      expect(image.style.getPropertyValue('max-width')).toBe('min(100%, 200px, calc(320px * 4))')
    } finally {
      image.remove()
      style.remove()
    }
  })

  it('releases a failed image cap and reapplies it after recovery', () => {
    const image = document.createElement('img')
    image.width = 100
    image.height = 600
    fitMediaBox(image, '320px')
    releaseFailedImageBox(image)
    expect(image.style.getPropertyValue('max-width')).toBe('100%')
    Object.defineProperties(image, {
      naturalWidth: { value: 100 },
      naturalHeight: { value: 600 },
    })
    image.dispatchEvent(new Event('load'))
    restoreImageBox(image)
    expect(image.style.getPropertyValue('max-width')).toBe(`min(100%, calc(320px * ${100 / 600}))`)
    expect(image.getAttribute('width')).toBe('100')
  })

  it('caps a declared width proportionally before the image loads', () => {
    const image = document.createElement('img')
    image.setAttribute('width', '571')
    image.setAttribute('height', '954')
    fitMediaBox(image, '320px')

    expect(image.style.getPropertyValue('max-width')).toBe(`min(100%, calc(320px * ${571 / 954}))`)
    expect(image.getAttribute('width')).toBe('571')
  })

  it('uses the loaded dimensions over incorrect attributes and stops observing on cleanup', () => {
    const image = document.createElement('img')
    image.setAttribute('width', '600')
    image.setAttribute('height', '600')
    const cleanup = fitMediaBox(image, 'var(--media-height, 48vh)')
    Object.defineProperties(image, {
      naturalWidth: { value: 571, configurable: true },
      naturalHeight: { value: 954, configurable: true },
    })
    image.dispatchEvent(new Event('load'))
    const cap = `min(100%, calc(var(--media-height, 48vh) * ${571 / 954}))`
    expect(image.style.getPropertyValue('max-width')).toBe(cap)

    cleanup()
    Object.defineProperty(image, 'naturalWidth', { value: 954 })
    image.dispatchEvent(new Event('load'))
    expect(image.style.getPropertyValue('max-width')).toBe(cap)
  })

  it('waits for dimensions when the sender uses a percentage width', () => {
    const image = document.createElement('img')
    image.setAttribute('width', '100%')
    image.setAttribute('height', '954')
    fitMediaBox(image, '320px')
    expect(image.style.getPropertyValue('max-width')).toBe('100%')
  })

  it('updates video proportions when metadata arrives', () => {
    const video = document.createElement('video')
    fitMediaBox(video, '320px')
    Object.defineProperties(video, {
      videoWidth: { value: 1920 },
      videoHeight: { value: 1080 },
    })
    video.dispatchEvent(new Event('loadedmetadata'))
    expect(video.style.getPropertyValue('max-width')).toBe(`min(100%, calc(320px * ${1920 / 1080}))`)
  })
})
