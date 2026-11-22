// Checks the file name offered when saving an attachment.
// MIME metadata, trusted suffixes and fallbacks for unnamed
// files determine which extension the user receives.

import { describe, expect, it } from 'bun:test'
import { attachmentSaveFilename } from './attachmentFilename'

describe('attachmentSaveFilename', () => {
  it('uses MIME metadata for misleading cache suffixes and structured image subtypes', () => {
    expect(attachmentSaveFilename('Release v1.2', 'msg/0.22', 'image/png')).toBe('Release v1.2.png')
    expect(attachmentSaveFilename('logo', 'msg/0.svg_xml', 'image/svg+xml')).toBe('logo.svg')
    expect(attachmentSaveFilename('scan 2024.10.22', 'msg/0.22', 'image/heic')).toBe('scan 2024.10.22.heic')
    expect(attachmentSaveFilename('photo', 'msg/0.22', 'image/avif')).toBe('photo.avif')
  })

  it('preserves image suffixes and avoids borrowing unrecognized cache suffixes', () => {
    expect(attachmentSaveFilename('photo.jfif', 'msg/0.jfif', 'image/jpeg')).toBe('photo.jfif')
    expect(attachmentSaveFilename('anim.apng', 'msg/0.apng', 'image/png')).toBe('anim.apng')
    expect(attachmentSaveFilename('photo', 'msg/0.22')).toBe('photo')
    expect(attachmentSaveFilename('logo', 'msg/0.png')).toBe('logo.png')
    expect(attachmentSaveFilename('report.csv', 'msg/0.csv', 'text/csv')).toBe('report.csv')
  })
  it('keeps gallery filename fallbacks for unnamed videos and uncommon formats', () => {
    expect(attachmentSaveFilename('attachment', 'msg/0.mp4', 'video/mp4')).toBe('attachment.mp4')
    expect(attachmentSaveFilename('clip.mp4', 'msg/0.mp4', 'video/mp4')).toBe('clip.mp4')
    expect(attachmentSaveFilename('attachment', 'msg/0.pjpeg', 'image/pjpeg')).toBe('attachment.pjpeg')
  })
  it('preserves extensionless ordinary attachment names', () => {
    expect(attachmentSaveFilename('README', 'msg/0.plain', 'text/plain')).toBe('README')
    expect(attachmentSaveFilename('invite', 'msg/0.calendar', 'text/calendar')).toBe('invite')
  })
})
