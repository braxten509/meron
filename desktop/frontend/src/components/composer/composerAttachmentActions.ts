// Handles picked, pasted and dropped composer attachments.
// Inline image placement follows the order in which files were supplied.
// Native clipboard reads share the paste timestamp with the editor to suppress duplicates.

import { useEditor } from '@tiptap/react'
import { useTranslation } from '../../lib/i18n'
import { showToast } from '../../states/ui'
import type { ComposeDraft, ComposerAttachment } from '../../types'
import { compose$ } from '../../states/composeState'
import { invoke } from '../../lib/bridge'
import {
  readAttachment,
  reportUnattachedFiles,
  settleAttachment,
  takeWithinAttachmentLimits,
} from '../../lib/attachFiles'
import { pickFiles, pickImageFiles } from '../../lib/nativeFilePicker'
import { ResizableImage } from './composerImage'
import {
  clipboardHasImageMarkup,
  createInlineId,
  extractClipboardImages,
  readClipboardImages,
  readNativeClipboardImage,
  type NativeClipboardImage,
} from './composerHelpers'

// A quoted reply's `<blockquote type="cite">` (see replyQuote) keeps its type
// through the editor, which would otherwise drop it: that attribute is what
// tells the conversation view the blockquote is a quote to fold.
export function composerAttachmentActions(
  tabId: string,
  editor: ReturnType<typeof useEditor>,
  draft: ComposeDraft | undefined,
  update: (partial: Partial<ComposeDraft>) => void,
  lastImagePasteAtRef: { current: number },
  t: ReturnType<typeof useTranslation>['t'],
) {
  // Inline images replace the selection; with `afterSelection` (a drop) they go
  // in after it, so a dropped image never replaces one that is still selected
  // from the last insertion. The selection is read when each image is ready:
  // the body may have been edited while its file was written.
  const addFiles = (files: ArrayLike<File>, options: { inline?: boolean; afterSelection?: boolean } = {}) => {
    const attachmentsNow = () => compose$.tabs.get().find((t) => t.id === tabId)?.compose?.attachments ?? []
    const insertImage = (source: string, alt: string, attachmentId: string) => {
      try {
        const chain = editor!.chain().focus()
        const placed = options.afterSelection
          ? chain
              .insertContentAt(editor!.state.selection.to, { type: ResizableImage.name, attrs: { src: source, alt } })
              .run()
          : chain.setImage({ src: source, alt }).run()
        if (placed) return
      } catch (requestFailure) {
        console.error('Failed to insert inline image:', requestFailure)
      }
      // Not in the body: keep it as an ordinary attachment, not an inline part
      // nothing refers to.
      update({
        attachments: attachmentsNow().map((a) => (a.id === attachmentId ? { ...a, inlineId: undefined } : a)),
      })
    }
    const taken = takeWithinAttachmentLimits(
      tabId,
      Array.from(files),
      attachmentsNow().reduce((bytes, attachment) => bytes + attachment.size, 0),
    )
    if (taken.length < files.length) reportUnattachedFiles()
    // Every file is read and attached as soon as it can be; only placing the
    // inline images waits its turn, so they keep the order they came in.
    let placing: Promise<void> = Promise.resolve()
    for (const file of taken) {
      const ready = readAttachment(file).then(async ({ attachment, dataUrl }) => {
        const shouldInline = !!options.inline && file.type.toLowerCase().startsWith('image/') && !!editor
        const next: ComposerAttachment = { ...attachment, inlineId: shouldInline ? createInlineId() : undefined }
        update({ attachments: [...attachmentsNow(), next] })
        settleAttachment(tabId, file)
        if (!shouldInline) return null
        const source = await invoke<string>('composer.writeMediaFile', {
          data: next.data,
          filename: next.filename,
        }).catch((requestFailure) => {
          console.error('Failed to write media file:', requestFailure)
          return dataUrl
        })
        return { src: source, next }
      })
      // Reported when its turn comes below; until then a failure is not one
      // nobody handles. A file that never attached is settled here.
      ready.catch(() => settleAttachment(tabId, file))
      placing = placing
        .then(() => ready)
        .then((image) => {
          if (image) insertImage(image.src, image.next.filename, image.next.id)
        })
        .catch((requestFailure) => {
          // A folder, or a file that can't be read.
          console.error('Failed to attach file:', requestFailure)
          reportUnattachedFiles()
        })
    }
  }

  const addNativeClipboardImage = (image: NativeClipboardImage, options: { inline?: boolean } = {}) => {
    const shouldInline = !!options.inline && image.mime.toLowerCase().startsWith('image/') && !!editor
    const next: ComposerAttachment = {
      id: `${Date.now()}-${Math.random().toString(36).substring(2, 9)}`,
      filename: image.filename,
      mime: image.mime || 'application/octet-stream',
      size: image.size,
      data: image.data,
      inlineId: shouldInline ? createInlineId() : undefined,
    }
    update({
      attachments: [...(compose$.tabs.get().find((t) => t.id === tabId)?.compose?.attachments ?? []), next],
    })
    if (shouldInline) {
      invoke<string>('composer.writeMediaFile', { data: image.data, filename: image.filename })
        .then((url) => {
          editor!.chain().focus().setImage({ src: url, alt: image.filename }).run()
        })
        .catch((requestFailure) => {
          console.error('Failed to write media file:', requestFailure)
          editor!
            .chain()
            .focus()
            .setImage({ src: `data:${image.mime || 'image/png'};base64,${image.data}`, alt: image.filename })
            .run()
        })
    }
  }

  const pickAttachmentFiles = async () => {
    try {
      const files = await pickFiles(t('composer.actions.attachFiles'))
      if (files.length > 0) addFiles(files)
    } catch (error) {
      showToast(error instanceof Error ? error.message : t('chat.failedToChooseFiles'), 'error')
    }
  }

  const pickInlineImages = async () => {
    try {
      const files = await pickImageFiles(t('composer.actions.insertInlineImages'))
      if (files.length > 0) addFiles(files, { inline: true })
    } catch (error) {
      showToast(error instanceof Error ? error.message : t('chat.failedToChooseImages'), 'error')
    }
  }

  const handlePaste = (event: React.ClipboardEvent) => {
    const imageFiles = extractClipboardImages(event.clipboardData)
    if (imageFiles.length > 0) {
      event.preventDefault()
      lastImagePasteAtRef.current = Date.now()
      addFiles(imageFiles)
    } else if (clipboardHasImageMarkup(event.clipboardData)) {
      event.preventDefault()
      pasteImageFromClipboard(Date.now(), { inline: false })
    }
  }

  // WebKitGTK often doesn't expose pasted images via the synchronous paste
  // event's clipboardData. As a fallback, intercept Ctrl/Cmd+V and read the
  // clipboard asynchronously.
  const handleKeyDown = async (event: React.KeyboardEvent) => {
    const isPaste = (event.ctrlKey || event.metaKey) && (event.key === 'v' || event.key === 'V')
    if (!isPaste) return

    pasteImageFromClipboard(Date.now(), { inline: !!draft?.rich })
  }

  const pasteImageFromClipboard = (pasteStartedAt: number, options: { inline: boolean }) => {
    void readClipboardImages()
      .then((images) => {
        if (lastImagePasteAtRef.current >= pasteStartedAt) return
        if (images.length === 0) {
          return readNativeClipboardImage().then((image) => {
            if (!image || lastImagePasteAtRef.current >= pasteStartedAt) return
            lastImagePasteAtRef.current = Date.now()
            addNativeClipboardImage(image, options)
          })
        }
        lastImagePasteAtRef.current = Date.now()
        addFiles(images, options)
      })
      .catch(() => {
        void readNativeClipboardImage()
          .then((image) => {
            if (!image || lastImagePasteAtRef.current >= pasteStartedAt) return
            lastImagePasteAtRef.current = Date.now()
            addNativeClipboardImage(image, options)
          })
          .catch(() => undefined)
      })
  }

  return { addFiles, pickAttachmentFiles, pickInlineImages, handlePaste, handleKeyDown, pasteImageFromClipboard }
}
