// Edits a query bound to the active desktop thread list. Its search state follows
// the current mailbox context.
// Imported by ThreadList.tsx and TitleBar.tsx.

import { useEffect, useRef } from 'react'
import { useValue } from '@legendapp/state/react'
import { useTranslation } from '../../lib/i18n'
import { isRssAccount } from '../../lib/threadActions'
import { accounts$ } from '../../states/accounts'
import { ui$ } from '../../states/ui'
import { SearchBox } from '../search/SearchBox'

type ThreadSearchInputProps = {
  /** The title bar's smaller box, sized to fit its row. */
  compact?: boolean
  onFocusChange?: (focused: boolean) => void
}

/**
 * The thread list's search box. It sits in the title bar when Meron draws one,
 * and in the thread list's header otherwise.
 */
export function ThreadSearchInput({ compact = false, onFocusChange }: ThreadSearchInputProps) {
  const { t } = useTranslation()
  const query = useValue(ui$.query)
  const selectedAccount = useValue(ui$.selectedAccount)
  const accounts = useValue(accounts$)
  const rss = isRssAccount(
    accounts.find((account) => account.id === selectedAccount),
    selectedAccount,
  )
  // Focus the search box when ⌘/Ctrl+Shift+F (or the palette) bumps the signal.
  const inputRef = useRef<HTMLInputElement | null>(null)
  const globalSearchFocus = useValue(ui$.globalSearchFocus)
  useEffect(() => {
    if (globalSearchFocus === 0) return
    const input = inputRef.current
    input?.focus()
    input?.select()
  }, [globalSearchFocus])

  return (
    <SearchBox
      value={query}
      onChange={(next) => ui$.query.set(next)}
      placeholder={rss ? t('threads.searchFeeds') : t('threads.searchMessages')}
      compact={compact}
      className="flex-1"
      inputRef={inputRef}
      onFocusChange={onFocusChange}
      // Feed search is plain text; the operators are mail's.
      options={
        rss
          ? undefined
          : {
              onSearch: (next) => {
                ui$.query.set(next)
                inputRef.current?.focus()
              },
            }
      }
    />
  )
}
