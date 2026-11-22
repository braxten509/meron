// Reads a browser File as the base64 payload expected
// by image commands. The helper strips the data URL
// prefix and reports read failures to the uploader.

/** Read a File as base64 (the payload format the image-write bridge calls expect). */
export function readFileData(file: File): Promise<string> {
  return new Promise((resolve, reject) => {
    const reader = new FileReader()
    reader.onload = () => {
      const dataUrl = String(reader.result || '')
      const [, data = ''] = dataUrl.split(',')
      data ? resolve(data) : reject(new Error('Could not read image data'))
    }
    reader.onerror = () => reject(reader.error ?? new Error('Could not read image'))
    reader.readAsDataURL(file)
  })
}
