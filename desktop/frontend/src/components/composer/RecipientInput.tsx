// Edits a recipient list with address suggestions and keyboard selection.
// Accepting a suggestion must cooperate with ordinary Tab navigation.
// Callers include ComposerHeaderFields.tsx and RulesPanel.tsx.

import { useEffect, useRef, useState } from 'react'
import type { Contact } from '../../types'
import { suggestContacts, formatContact } from '../../lib/contacts'
import { useEscapeKey } from '../../lib/useEscapeKey'

type RecipientInputProps = {
  value: string
  onChange: (value: string) => void
  accountId: string
  placeholder?: string
  autoFocus?: boolean
  inputRef?: React.Ref<HTMLInputElement>
  onTab?: () => void
  /** Bare addresses, as search's `from:`/`to:` take them: a pick adds the address, not `Name <address>`. */
  bare?: boolean
  className?: string
}

// A comma-separated recipient field carries multiple addresses; autocomplete
// only ever completes the token after the final comma — outside quotes, as a
// quoted name like `"Lee, Ann"` holds one. Returns the leading portion
// (including that comma) plus the active token being typed.
function splitTail(value: string): { head: string; tail: string } {
  let index = -1
  let quoted = false
  for (let i = 0; i < value.length; i++) {
    if (value[i] === '"') quoted = !quoted
    else if (value[i] === ',' && !quoted) index = i
  }
  if (index === -1) return { head: '', tail: value }
  return { head: value.slice(0, index + 1), tail: value.slice(index + 1) }
}

const inputClass = 'w-full bg-transparent text-[0.8125rem] text-primary placeholder-secondary outline-none'

export function RecipientInput({
  value,
  onChange,
  accountId,
  placeholder,
  autoFocus,
  inputRef,
  onTab,
  bare = false,
  className = inputClass,
}: RecipientInputProps) {
  const [suggestions, setSuggestions] = useState<Contact[]>([])
  const [open, setOpen] = useState(false)
  const [active, setActive] = useState(0)
  const focusedRef = useRef(false)
  const suggestionsEnabledRef = useRef(false)
  const blurTimer = useRef<ReturnType<typeof setTimeout> | null>(null)

  const tail = splitTail(value).tail.trim()
  // The account and token the held suggestions were looked up for.
  const lookupKey = `${accountId}\n${tail}`
  const suggestionsKeyRef = useRef('')

  // Fetch suggestions (debounced) while focused and typing. An empty token
  // shows nothing: a list of top correspondents on every open composer only
  // covers the fields below it.
  useEffect(() => {
    if (!tail) {
      setOpen(false)
      setSuggestions([])
      suggestionsKeyRef.current = ''
      return
    }
    if (!focusedRef.current || !suggestionsEnabledRef.current) return
    let cancelled = false
    const timer = setTimeout(async () => {
      const results = await suggestContacts(accountId, tail)
      if (cancelled || !suggestionsEnabledRef.current) return
      // Kept even when focus has left, so coming back finds the lookup for
      // the token as it stands rather than one from before the last edit.
      suggestionsKeyRef.current = `${accountId}\n${tail}`
      setSuggestions(results)
      setActive(0)
      setOpen(focusedRef.current && results.length > 0)
    }, 120)
    return () => {
      cancelled = true
      clearTimeout(timer)
    }
  }, [tail, accountId])

  // The topmost Escape layer, so closing the list leaves a form or dialog around it open.
  useEscapeKey(() => setOpen(false), open && suggestions.length > 0)

  function accept(contact: Contact) {
    suggestionsEnabledRef.current = false
    const { head } = splitTail(value)
    const prefix = head ? `${head} ` : ''
    onChange(`${prefix}${bare ? contact.addr : formatContact(contact)}, `)
    setOpen(false)
    setSuggestions([])
    suggestionsKeyRef.current = ''
  }

  function onKeyDown(event: React.KeyboardEvent<HTMLInputElement>) {
    const plainTab = event.key === 'Tab' && !event.shiftKey && !event.ctrlKey && !event.metaKey && !event.altKey
    if (event.key === 'Tab' && (!plainTab || !(open && suggestions.length > 0))) {
      suggestionsEnabledRef.current = false
      setOpen(false)
      if (onTab && plainTab) {
        event.preventDefault()
        onTab()
      }
      return
    }
    if (open && suggestions.length > 0) {
      if (event.key === 'ArrowDown') {
        event.preventDefault()
        setActive((i) => (i + 1) % suggestions.length)
        return
      }
      if (event.key === 'ArrowUp') {
        event.preventDefault()
        setActive((i) => (i - 1 + suggestions.length) % suggestions.length)
        return
      }
      if (event.key === 'Enter' || event.key === 'Tab') {
        event.preventDefault()
        accept(suggestions[active])
        return
      }
    }
  }

  return (
    <div className="relative flex-1">
      <input
        ref={inputRef}
        autoFocus={autoFocus}
        value={value}
        placeholder={placeholder}
        spellCheck={false}
        className={className}
        onChange={(event) => {
          suggestionsEnabledRef.current = true
          onChange(event.target.value)
        }}
        onKeyDown={onKeyDown}
        onFocus={() => {
          focusedRef.current = true
          if (blurTimer.current) clearTimeout(blurTimer.current)
          // Focusing never starts suggesting; typing does (see onChange). Coming
          // back to a token that was mid-edit reopens what it had, though —
          // but only a lookup for that very token and account, never one the
          // edit overtook, or Tab would insert a recipient nobody typed.
          const resume = tail.length > 0 && suggestions.length > 0 && suggestionsKeyRef.current === lookupKey
          if (resume) suggestionsEnabledRef.current = true
          setOpen(resume)
        }}
        onBlur={() => {
          focusedRef.current = false
          // Delay so a mousedown on a suggestion registers before we close.
          blurTimer.current = setTimeout(() => {
            setOpen(false)
          }, 150)
        }}
      />
      {open && suggestions.length > 0 && (
        <ul className="absolute left-0 right-0 top-full z-50 mt-1 max-h-60 overflow-y-auto rounded-lg border border-border bg-chats py-1 shadow-xl">
          {suggestions.map((c, i) => (
            <li
              key={c.addr}
              // mousedown (not click) so it fires before the input's blur.
              onMouseDown={(event) => {
                event.preventDefault()
                accept(c)
              }}
              onMouseEnter={() => setActive(i)}
              className={`cursor-pointer px-3 py-1.5 text-[0.8125rem] ${i === active ? 'bg-accent/10' : ''}`}
            >
              {c.name.trim() && c.name.trim().toLowerCase() !== c.addr.toLowerCase() ? (
                <span>
                  <span className="text-primary">{c.name.trim()}</span>{' '}
                  <span className="text-secondary">{`<${c.addr}>`}</span>
                </span>
              ) : (
                <span className="text-primary">{c.addr}</span>
              )}
            </li>
          ))}
        </ul>
      )}
    </div>
  )
}
