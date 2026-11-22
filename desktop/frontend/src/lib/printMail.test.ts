// Printing uses the complete message and a text
// representation of HTML-only mail. The fixtures exercise
// headers, quotes and attachment names in the generated
// print document.

import { beforeEach, afterEach, expect, spyOn, test } from 'bun:test'
import {
  loadPrintThread,
  mailPrintHtml,
  mailPrintText,
  nativePrintHtml,
  printMail,
  printThread,
  referencesRemoteContent,
} from './printMail'
import type { Message, MessageTab } from '../types'
import { CONVERSATION_PAGE_SIZE } from './pagination'
import { thread$ } from '../states/thread'
import { accounts$ } from '../states/accounts'
import type { Account } from '../types'
import { ui$ } from '../states/ui'

beforeEach(() => {
  thread$.conversationModeOverrides.set({ a: 'html' })
})

const message: Message = {
  id: 'm',
  account_id: 'a',
  folder_id: 'inbox',
  thread_id: 't',
  from_name: 'Alice',
  from_addr: 'alice@example.com',
  to: 'bob@example.com',
  subject: 'Hello <world>',
  body: 'Reply\n\n> Entire quoted message',
  preview: '',
  date: 0,
  unread: false,
  starred: false,
  has_attachments: false,
}

afterEach(() => {
  window.dispatchEvent(new Event('afterprint'))
  delete (window as any).go
  thread$.conversationModeOverrides.set({})
  accounts$.set([])
})

test('includes headers, full quoted body, and attachment names', () => {
  const text = mailPrintText({
    ...message,
    cc: 'cc@example.com',
    bcc: 'bcc@example.com',
    attachments: [{ key: 'report', filename: 'report.pdf', mime: 'application/pdf', size: 42 }],
  })
  expect(text).toContain('Alice <alice@example.com>')
  expect(text).toContain('cc@example.com')
  expect(text).toContain('bcc@example.com')
  expect(text).toContain(message.body)
  expect(text).toContain('report.pdf')
})

test('extracts HTML-only text without scripts, styles, or image requests', () => {
  const text = mailPrintText({
    ...message,
    body: '',
    body_html:
      '<style>hidden css</style><script>bad()</script><p>First &amp; second</p><p>Third</p><img src="https://tracker.test/pixel">',
  })
  expect(text).toContain('First & second')
  expect(text).toContain('Third')
  expect(text).not.toContain('bad()')
  expect(text).not.toContain('hidden css')
  expect(text).not.toContain('tracker.test')
})

test('plain attachment summaries retain embedded images and standalone files', () => {
  const inline = { key: 'logo', filename: 'logo-proton.png', mime: 'image/png', size: 100 }
  const mail = { ...message, body_html: '<img src="/media/logo">', attachments: [inline] }
  expect(mailPrintText(mail)).toContain('Attachments:')
  expect(mailPrintText(mail)).toContain(inline.filename)
  const text = mailPrintText({
    ...mail,
    attachments: [
      inline,
      { key: 'photo', filename: 'photo.png', mime: 'image/png', size: 100 },
      { key: 'report', filename: 'report.pdf', mime: 'application/pdf', size: 42 },
    ],
  })
  expect(text).toContain('photo.png')
  expect(text).toContain('report.pdf')
  expect(text).toContain(inline.filename)
})

test('message printing follows account mode and conversation overrides', () => {
  const mail = { ...message, body_html: '<b>HTML</b>' }
  thread$.conversationModeOverrides.set({})
  accounts$.set([{ id: 'a', conversation_html: false } as Account])
  expect(mailPrintHtml(mail, true)).toBeUndefined()
  thread$.conversationModeOverrides.set({ a: 'html' })
  expect(mailPrintHtml(mail, false)).toContain('<b>HTML</b>')
  accounts$.set([{ id: 'a', conversation_html: true } as Account])
  thread$.conversationModeOverrides.set({ a: 'plain' })
  expect(mailPrintHtml(mail, true)).toBeUndefined()
})

test('HTML printing preserves formatting and images even when a plain body exists', () => {
  const html = mailPrintHtml(
    {
      ...message,
      body_html:
        '<style>p { color: purple }</style><p><strong>Calendar</strong></p><img src="/media/logo" width="200" height="100">',
    },
    false,
  )!
  const doc = new DOMParser().parseFromString(html, 'text/html')
  expect(doc.querySelector('strong')?.textContent).toBe('Calendar')
  expect(doc.querySelector('img')?.getAttribute('src')).toBe('/media/logo')
  expect(doc.querySelector('img')?.getAttribute('loading')).toBe('eager')
  expect(html).toContain('color: purple')
  expect(html).not.toContain(message.body)
})

test('printing honors plain mode, missing bodies, and text-only messages', () => {
  const tab = { body: 'Plain', bodyHtml: '<b>HTML</b>', viewMode: 'plain' } as MessageTab
  expect(mailPrintHtml(tab, false)).toBeUndefined()
  expect(mailPrintHtml({ ...tab, viewMode: 'html' }, false)).toContain('<b>HTML</b>')
  expect(mailPrintHtml({ ...tab, viewMode: 'html', bodyMissing: true }, false)).toBeUndefined()
  expect(mailPrintHtml({ ...message, body_html: '<b>HTML</b>', body_missing: true }, false)).toBeUndefined()
  expect(mailPrintHtml(message, false)).toBeUndefined()
  const missingTab = { ...tab, bodyMissing: true, body: 'Stale body' }
  expect(mailPrintText(missingTab)).toContain('Could not print')
  expect(mailPrintText(missingTab)).not.toContain('Stale body')
})

test('HTML print CSP respects current remote policy and isolates active content', () => {
  const mail = {
    ...message,
    body_html: `<meta http-equiv="Content-Security-Policy" content="default-src 'none'; img-src 'self' data: http: https:; style-src 'unsafe-inline'">
    <script>bad()</script><iframe></iframe>
    <img src="https://example.com/banner" width="600" height="200">`,
  }
  for (const allowed of [false, true]) {
    const doc = new DOMParser().parseFromString(mailPrintHtml(mail, allowed)!, 'text/html')
    expect(doc.querySelector('script, iframe')).toBeNull()
    const policies = Array.from(doc.querySelectorAll('meta[http-equiv="Content-Security-Policy"]'))
    expect(policies).toHaveLength(2)
    for (const policy of policies) {
      const images = policy
        .getAttribute('content')!
        .split(';')
        .find((part) => part.trim().startsWith('img-src'))!
      expect(images.includes('https:')).toBe(allowed)
    }
  }
})

test('native printing gets an escaped document, replaces previous jobs, and cleans up', async () => {
  const calls: string[] = []
  ;(window as any).go = {
    main: {
      App: {
        Invoke: async (command: string) => {
          calls.push(command)
          return true
        },
      },
    },
  }
  let printedText = ''
  ;(window as any).go.main.App.Invoke = async (command: string) => {
    calls.push(command)
    expect(document.querySelector('#meron-print-document img')).toBeNull()
    printedText = document.querySelector('#meron-print-document pre')?.textContent ?? ''
    return true
  }
  await printMail({ ...message, body: '<img src=x onerror=alert(1)>' })
  expect(calls).toEqual(['mail.print'])
  expect(printedText).toContain('<img src=x')
  expect(document.getElementById('meron-print-document')).toBeNull()
  await printMail(message)
  expect(document.querySelectorAll('#meron-print-document')).toHaveLength(0)
  window.dispatchEvent(new Event('afterprint'))
  expect(document.getElementById('meron-print-document')).toBeNull()
})

test('HTML print jobs wait for isolated frames and include headers and attachments', async () => {
  let printed = false
  ;(window as any).go = {
    main: {
      App: {
        Invoke: async () => {
          const frame = document.querySelector<HTMLIFrameElement>('#meron-print-document iframe')!
          expect(frame.getAttribute('sandbox')).toBe('allow-same-origin')
          expect(frame.srcdoc).toContain('<strong>Formatted body</strong>')
          const doc = new DOMParser().parseFromString(frame.srcdoc, 'text/html')
          expect(doc.body.firstElementChild?.textContent).toContain('Alice <alice@example.com>')
          expect(doc.body.lastElementChild?.textContent).toContain('report.pdf')
          expect(doc.body.textContent).not.toContain('logo-proton.png')
          // No separate header or footer can strand the body iframe on another page.
          expect(frame.parentElement?.children).toHaveLength(1)
          printed = true
          return true
        },
      },
    },
  }
  const job = printMail({
    ...message,
    body_html: '<strong>Formatted body</strong><img src="/media/logo">',
    attachments: [
      { key: 'report', filename: 'report.pdf', mime: 'application/pdf', size: 42 },
      { key: 'logo', filename: 'logo-proton.png', mime: 'image/png', size: 100 },
    ],
  })
  expect(printed).toBe(false)
  // Happy DOM does not provide browser layout; signal the frame's load explicitly.
  document.querySelector('#meron-print-document iframe')!.dispatchEvent(new Event('load'))
  await job
  expect(printed).toBe(true)
  expect(document.getElementById('meron-print-document')).toBeNull()
})

test('thread printing fetches all pages, deduplicates, and prints oldest first', async () => {
  const requests: any[] = []
  let sections: (string | null)[] = []
  ;(window as any).go = {
    main: {
      App: {
        Invoke: async (command: string, payload: any) => {
          if (command === 'mail.print') {
            sections = [...document.querySelectorAll('#meron-print-document pre')].map((section) => section.textContent)
            return true
          }
          requests.push(payload)
          return payload.before_cursor
            ? {
                messages: [
                  { ...message, id: 'old', date: 1, body: 'Oldest' },
                  { ...message, id: 'new', date: 2, body: 'Duplicate' },
                ],
              }
            : { messages: [{ ...message, id: 'new', date: 2, body: 'Newest' }], next_cursor: 'older' }
        },
      },
    },
  }
  await printThread('original-thread')
  expect(requests.map((request) => request.thread_id)).toEqual(['original-thread', 'original-thread'])
  expect(requests.every((request) => request.limit === CONVERSATION_PAGE_SIZE && request.for_print === true)).toBe(true)
  expect(requests[1].before_cursor).toBe('older')
  expect(sections).toHaveLength(2)
  expect(sections[0]).toContain('Oldest')
  expect(sections[1]).toContain('Newest')
})

test('mixed-CSP threads fall back to frames and beforeprint remeasures their height', async () => {
  let finishLoading: (() => void) | undefined
  const timeout = spyOn(window, 'setTimeout').mockImplementation(((callback: () => void, delay: number) => {
    if (delay === 15000) finishLoading = callback
    return 0
  }) as typeof window.setTimeout)
  document.documentElement.classList.add('native-scrollbars')
  let nativeScrollbarsWhilePrinting: boolean | undefined
  const print = spyOn(window, 'print').mockImplementation(() => {
    nativeScrollbarsWhilePrinting = document.documentElement.classList.contains('native-scrollbars')
    const frame = document.querySelector<HTMLIFrameElement>('#meron-print-document iframe')!
    const body = frame.contentDocument!.body
    Object.defineProperty(body, 'scrollHeight', { configurable: true, value: 1200 })
    window.dispatchEvent(new Event('beforeprint'))
    expect(frame.style.height).toBe('1200px')
    Object.defineProperty(body, 'scrollHeight', { configurable: true, value: 600 })
    window.dispatchEvent(new Event('beforeprint'))
    expect(frame.style.height).toBe('600px')
    window.dispatchEvent(new Event('afterprint'))
  })
  // One message with revealed remote content gives the thread two print CSPs.
  thread$.revealedRemote.set({ old: true })
  ;(window as any).go = {
    main: {
      App: {
        Invoke: async (command: string) => {
          if (command === 'mail.print') throw new Error('mixed CSPs must not reach native printing')
          return {
            messages: [
              {
                ...message,
                id: 'old',
                date: 1,
                body_html: '<p style="background: url(https://example.com/bg.png)">Remote allowed</p>',
              },
              { ...message, id: 'new', date: 2, body_html: '<p>Available body</p>' },
            ],
          }
        },
      },
    },
  }
  try {
    const job = printThread('t')
    while (!finishLoading) await Promise.resolve()
    finishLoading()
    await job
    expect(print).toHaveBeenCalledTimes(1)
    // WebKitGTK prints iframes blank under its overlay scrollbars.
    expect(nativeScrollbarsWhilePrinting).toBe(false)
    expect(document.documentElement.classList.contains('native-scrollbars')).toBe(true)
  } finally {
    document.documentElement.classList.remove('native-scrollbars')
    timeout.mockRestore()
    print.mockRestore()
    thread$.revealedRemote.set({})
  }
})

test('slow print preparation stays visible through native preparation and clears on return', async () => {
  const scheduledDelays: number[] = []
  let toastAtPrint = ''
  let showPreparing: (() => void) | undefined
  let finishLoading: (() => void) | undefined
  const timeout = spyOn(window, 'setTimeout').mockImplementation(((callback: () => void, delay: number) => {
    scheduledDelays.push(delay)
    if (delay === 1000) showPreparing = callback
    if (delay === 15000) finishLoading = callback
    return 0
  }) as typeof window.setTimeout)
  let printed = false
  ;(window as any).go = {
    main: {
      App: {
        Invoke: async () => {
          printed = true
          toastAtPrint = ui$.toast.peek()
          return true
        },
      },
    },
  }
  ui$.toast.set('')
  try {
    const job = printMail({ ...message, body_html: '<p>Body</p>' })
    expect(ui$.toast.peek()).toBe('')
    showPreparing!()
    expect(ui$.toast.peek()).toBe('Preparing print…')
    expect(scheduledDelays).not.toContain(2200)
    finishLoading!()
    await job
    expect(printed).toBe(true)
    expect(toastAtPrint).toBe('Preparing print…')
    expect(ui$.toast.peek()).toBe('')
  } finally {
    timeout.mockRestore()
    ui$.toast.set('')
  }
})

test('browser printing prints shadow-root markup from its own frame and cleans up afterprint', async () => {
  const originalTitle = document.title
  ;(window as any).go = { main: { App: { Invoke: async () => false } } }
  const windowPrint = spyOn(window, 'print')
  let printed: { title: string; nested: boolean; summary: string; body: string; head: string } | undefined
  // Happy DOM frames have no print(); stub it on the paged frame as it is added.
  const observer = new MutationObserver(() => {
    const frame = document.querySelector<HTMLIFrameElement>('#meron-print-document > iframe')
    if (!frame?.contentWindow || (frame.contentWindow as any).print) return
    ;(frame.contentWindow as any).print = () => {
      // Read before afterprint: cleanup removes the frame and its document.
      const doc = frame.contentDocument!
      printed = {
        title: doc.title,
        nested: !!doc.querySelector('iframe, template'),
        summary: doc.querySelector('section > pre')?.textContent ?? '',
        body: doc.querySelector('[data-print-body]')?.shadowRoot?.textContent ?? '',
        head: doc.head.textContent ?? '',
      }
      frame.contentWindow!.dispatchEvent(new Event('afterprint'))
    }
  })
  observer.observe(document.body, { childList: true, subtree: true })
  try {
    const job = printMail({ ...message, body_html: '<style>p { color: purple }</style><p>Formatted body</p>' })
    document.querySelector('#meron-print-document iframe')!.dispatchEvent(new Event('load'))
    await job
    expect(windowPrint).not.toHaveBeenCalled()
    expect(printed?.title).toBe(message.subject)
    expect(printed?.nested).toBe(false)
    expect(printed?.summary).toContain('Alice <alice@example.com>')
    expect(printed?.body).toContain('Formatted body')
    expect(printed?.head).not.toContain('purple')
    expect(printed?.head).toContain('@page { margin: 15mm; }')
    expect(document.getElementById('meron-print-document')).toBeNull()
    expect(document.title).toBe(originalTitle)
  } finally {
    observer.disconnect()
    windowPrint.mockRestore()
  }
})

test('native document removes iframe pagination while retaining isolated markup and CSP', () => {
  const root = document.createElement('div')
  root.dataset.printTitle = message.subject
  root.innerHTML = '<section><iframe></iframe></section><section><pre>Plain &lt;text&gt;</pre></section>'
  document.body.append(root)
  try {
    const frame = root.querySelector('iframe')!
    const html = mailPrintHtml(
      {
        ...message,
        body_html:
          '<style>body { color: purple }</style><div>hi<img width="562" height="562" src="/media/cat"></div><div>s2</div>',
      },
      false,
    )!
    frame.contentDocument!.write(html)
    const doc = new DOMParser().parseFromString(nativePrintHtml(root)!, 'text/html')
    expect(doc.title).toBe(message.subject)
    expect(doc.querySelector('iframe')).toBeNull()
    expect(doc.body.children).toHaveLength(2)
    const template = doc.querySelector('template')!
    const mail = new DOMParser().parseFromString(template.content.textContent!, 'text/html')
    expect(doc.querySelector('section > pre')?.textContent).toContain('Alice <alice@example.com>')
    expect(mail.body.textContent).not.toContain('Alice <alice@example.com>')
    expect(mail.body.textContent).toContain('s2')
    expect(mail.querySelector('img')?.getAttribute('src')).toBe('/media/cat')
    expect(
      Array.from(mail.querySelectorAll('style'))
        .map((style) => style.textContent)
        .join('\n'),
    ).toContain('body { color: purple }')
    expect(mail.querySelector('meta')).toBeNull()
    expect(doc.head.textContent).not.toContain('purple')
    expect(doc.head.querySelectorAll('meta[http-equiv="Content-Security-Policy"]')).toHaveLength(2)
    expect(doc.body.lastElementChild?.textContent).toBe('Plain <text>')
    // Run the native attachment step, rather than only inspecting inert markup.
    const host = template.parentElement!
    host.attachShadow({ mode: 'open' }).append(doc.importNode(mail.documentElement, true))
    template.remove()
    expect(host.matches('[data-print-body]')).toBe(true)
    expect(host.parentElement?.shadowRoot).toBeNull()
    expect(host.previousElementSibling?.textContent).toContain('Alice <alice@example.com>')
    expect(host.shadowRoot?.textContent).toContain('s2')
  } finally {
    root.remove()
  }
})

test('native printing falls back for mixed CSPs and rejects inaccessible frames', () => {
  const root = document.createElement('div')
  root.innerHTML = '<section><iframe></iframe></section><section><iframe></iframe></section>'
  document.body.append(root)
  try {
    const frames = root.querySelectorAll('iframe')
    for (const [index, frame] of Array.from(frames).entries()) {
      frame.contentDocument!.write(mailPrintHtml({ ...message, body_html: '<p>Body</p>' }, index === 0)!)
    }
    expect(nativePrintHtml(root)).toBeUndefined()
    Object.defineProperty(frames[0], 'contentDocument', { configurable: true, value: null })
    expect(() => nativePrintHtml(root)).toThrow('Print HTML unavailable')
  } finally {
    root.remove()
  }
})

test('remote content detection ignores links and text but not resources', () => {
  expect(referencesRemoteContent(undefined)).toBe(false)
  expect(referencesRemoteContent('<p>See https://example.com</p><a href="https://example.com">link</a>')).toBe(false)
  expect(referencesRemoteContent('<img src="/media/cat" alt="cat">')).toBe(false)
  // GitHub notifications carry schema.org metadata that loads nothing.
  expect(
    referencesRemoteContent(
      '<div itemscope itemtype="http://schema.org/EmailMessage"><link itemprop="url" href="https://github.com/x"><meta itemprop="name" content="https://x"></div>',
    ),
  ).toBe(false)
  expect(referencesRemoteContent('<svg><image href="https://example.com/a.png"/></svg>')).toBe(true)
  // Inline data loads nothing, even when its base64 contains "//".
  expect(
    referencesRemoteContent(
      '<img src="data:image/gif;base64,R0lGODlhAQABAIAAAAAAAP///yH5BAEAAAAALAAAAAABAAEAAAIBRAA7" width="1" height="1">',
    ),
  ).toBe(false)
  expect(referencesRemoteContent('<div style="background: url(data:image/png;base64,AP///y)"></div>')).toBe(false)
  expect(referencesRemoteContent('<img srcset="/media/a 1x, https://example.com/b.png 2x">')).toBe(true)
  expect(referencesRemoteContent('<div style="background: url( //example.com/bg.png)"></div>')).toBe(true)
  // CSS escapes can spell the scheme or slashes; browsers resolve them to remote URLs.
  expect(referencesRemoteContent('<div style="background-image:url(\\68 ttps://example.com/bg.png)"></div>')).toBe(true)
  expect(referencesRemoteContent('<style>p { background: url("\\2f\\2f example.com/bg.png") }</style>')).toBe(true)
  expect(referencesRemoteContent('<style>p { background: url(h\\ttp://example.com/bg.png) }</style>')).toBe(true)
  // A line continuation inside a string vanishes, joining the scheme back up.
  for (const newline of ['\n', '\r\n', '\r', '\f']) {
    expect(
      referencesRemoteContent(`<style>p { background: url("ht\\${newline}tps://example.com/bg.png") }</style>`),
    ).toBe(true)
    expect(referencesRemoteContent(`<p style='background: url("ht\\${newline}tps://example.com/bg.png")'></p>`)).toBe(
      true,
    )
  }
  expect(referencesRemoteContent('<img src="https://example.com/cat.png" width="100" height="100">')).toBe(true)
  expect(referencesRemoteContent('<img srcset="//example.com/cat.png 2x">')).toBe(true)
  expect(referencesRemoteContent('<div style="background: url(http://example.com/bg.png)"></div>')).toBe(true)
  expect(referencesRemoteContent('<style>@import "https://example.com/a.css";</style>')).toBe(true)
})

test('mixed remote policies print as one document when the allowed message loads nothing remote', async () => {
  let printed = ''
  thread$.revealedRemote.set({ old: true })
  ;(window as any).go = {
    main: {
      App: {
        Invoke: async (command: string, payload: any) => {
          if (command === 'mail.print') {
            printed = payload.html
            return true
          }
          return {
            messages: [
              { ...message, id: 'old', date: 1, body_html: '<p>No remote <a href="https://example.com">link</a></p>' },
              { ...message, id: 'new', date: 2, body_html: '<p>Blocked</p>' },
            ],
          }
        },
      },
    },
  }
  try {
    const job = printThread('t')
    for (let frames: NodeListOf<HTMLIFrameElement>; ; await Promise.resolve()) {
      frames = document.querySelectorAll('#meron-print-document iframe')
      if (frames.length === 2) {
        frames.forEach((frame) => frame.dispatchEvent(new Event('load')))
        break
      }
    }
    await job
    const doc = new DOMParser().parseFromString(printed, 'text/html')
    expect(doc.querySelectorAll('template[data-print-message]')).toHaveLength(2)
    // Tightened to the stricter policy rather than broadened.
    expect(doc.head.innerHTML).not.toContain('https:')
  } finally {
    thread$.revealedRemote.set({})
  }
})

test('prints unavailable-body notices and rejects repeated cursors', async () => {
  let printed = false
  ;(window as any).go = {
    main: {
      App: {
        Invoke: async (command: string) => {
          if (command === 'mail.print') printed = true
          return { messages: [{ ...message, body_missing: true }] }
        },
      },
    },
  }
  expect(await loadPrintThread('t')).toHaveLength(1)
  expect(mailPrintText({ ...message, body_missing: true })).not.toContain(message.body)
  await printThread('t')
  expect(printed).toBe(true)
  ;(window as any).go.main.App.Invoke = async () => ({ messages: [message], next_cursor: 'same' })
  await expect(loadPrintThread('t')).rejects.toThrow('Repeated thread cursor')
})
