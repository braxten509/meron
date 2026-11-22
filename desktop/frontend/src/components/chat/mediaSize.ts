// Determines how message media fits the space available in the reader. The sizing
// rules account for the dimensions provided by the sender.
// Imported by BubbleHtmlFrame.tsx and readerHtml.ts.

const fittedMedia = new WeakMap<HTMLImageElement | HTMLVideoElement, { senderMaxWidth: string; cleanup: () => void }>()

/** Keep a height-capped picture's box as narrow as the picture it contains. */
export function fitMediaBox(media: HTMLImageElement | HTMLVideoElement, maxHeight: string): () => void {
  const previous = fittedMedia.get(media)
  previous?.cleanup()
  // Readers can initialize at both parse and load. Never capture our own
  // computed cap as a sender limit: its viewport variable is already pixels.
  const senderMaxWidth =
    previous?.senderMaxWidth ??
    (media.ownerDocument.defaultView?.getComputedStyle(media).maxWidth || media.style.getPropertyValue('max-width'))
  // Intrinsic sizing keywords such as max-content cannot be used in min().
  const isLength = /^(?:[+-]?(?:\d*\.)?\d+(?:[a-z]+|%)?|(?:calc|min|max|clamp)\(.+\))$/i.test(senderMaxWidth)
  const senderLimit = isLength && senderMaxWidth !== '100%' ? `${senderMaxWidth}, ` : ''
  const update = () => {
    const width = 'naturalWidth' in media ? media.naturalWidth : media.videoWidth
    const height = 'naturalHeight' in media ? media.naturalHeight : media.videoHeight
    const declaredWidth = Number(media.getAttribute('width'))
    const declaredHeight = Number(media.getAttribute('height'))
    const ratio = width > 0 && height > 0 ? width / height : declaredWidth / declaredHeight
    // An explicit width otherwise stays fixed when max-height reduces height;
    // object-fit only shrinks the pixels inside that oversized element.
    media.style.setProperty(
      'max-width',
      Number.isFinite(ratio) && ratio > 0
        ? `min(100%, ${senderLimit}calc(${maxHeight} * ${ratio}))`
        : senderLimit
          ? `min(100%, ${senderMaxWidth})`
          : '100%',
      'important',
    )
  }
  update()
  const event = 'naturalWidth' in media ? 'load' : 'loadedmetadata'
  media.addEventListener(event, update)
  const cleanup = () => media.removeEventListener(event, update)
  fittedMedia.set(media, { senderMaxWidth, cleanup })
  return cleanup
}
