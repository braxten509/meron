// Chooses the suggested save name for an attachment using the message metadata and
// available filename. Native save dialogs use the result as their initial
// suggestion.
// Imported by mail.ts.

const imageExtension = /\.(?:a?png|jpe?g|jpe|jfif|gif|webp|bmp|tiff?|heic|heif|avif|svg|ico|jxl|jp2|jpx)$/i
const mimeExtensions: Record<string, string> = {
  'image/png': '.png',
  'image/jpeg': '.jpg',
  'image/gif': '.gif',
  'image/webp': '.webp',
  'image/bmp': '.bmp',
  'image/x-ms-bmp': '.bmp',
  'image/tiff': '.tiff',
  'image/heic': '.heic',
  'image/heif': '.heif',
  'image/avif': '.avif',
  'image/svg+xml': '.svg',
  'image/x-icon': '.ico',
  'image/vnd.microsoft.icon': '.ico',
}

// MIME metadata also handles cache keys inherited from dotted part names.
export function attachmentSaveFilename(filename: string, key: string, mime?: string): string {
  const mediaMime = mime?.split(';')[0].trim().toLowerCase() ?? ''
  const isMedia = mediaMime.startsWith('image/') || mediaMime.startsWith('video/')
  if (mediaMime && !isMedia) return filename
  if (imageExtension.test(filename)) return filename
  const cachedSuffix = key.match(/\.[a-z0-9]+$/i)?.[0] ?? ''
  const mimeSuffix = mimeExtensions[mediaMime]
  if (imageExtension.test(cachedSuffix)) return filename + cachedSuffix
  if (mimeSuffix) return filename + mimeSuffix
  // Keep the gallery's extension fallback for videos and other attachments,
  // while avoiding numeric suffixes inherited from dated MIME filenames.
  if (isMedia && !/\.[a-z0-9]+$/i.test(filename) && /[a-z]/i.test(cachedSuffix)) return filename + cachedSuffix
  return filename
}
