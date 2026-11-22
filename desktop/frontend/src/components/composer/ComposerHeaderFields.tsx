// Presents the addressing, identity and subject fields for a desktop draft. Edits
// feed the same draft model used by the message editor.
// Imported by Composer.tsx.

import { useValue } from '@legendapp/state/react'
import { useRef } from 'react'
import { useTranslation } from '../../lib/i18n'
import { accounts$, isSendableAccount, accountIdentities } from '../../states/accounts'
import { settings$ } from '../../states/settings'
import type { ComposeDraft } from '../../types'
import { RecipientInput } from './RecipientInput'

const fieldClass = 'flex-1 bg-transparent text-[0.8125rem] text-primary placeholder-secondary outline-none'

const labelClass = 'w-12 shrink-0 text-[0.6875rem] font-bold text-secondary'

// The From/To/Cc/Bcc/Subject header block of the composer.
export function ComposerHeaderFields({
  draft,
  update,
  focusTo,
  onFocusBody,
}: {
  draft: ComposeDraft
  update: (partial: Partial<ComposeDraft>) => void
  focusTo: boolean
  onFocusBody: () => boolean
}) {
  const { t } = useTranslation()
  const ccRef = useRef<HTMLInputElement>(null)
  const subjectRef = useRef<HTMLInputElement>(null)
  const accounts = useValue(accounts$)
  const spellCheck = useValue(settings$.spellCheck)
  const sendable = accounts.filter(isSendableAccount)
  // Each account contributes its primary plus any aliases as pickable From
  // identities; show the selector whenever more than one exists.
  const identityCount = sendable.reduce((n, acc) => n + accountIdentities(acc).length, 0)
  const fromAccount = sendable.find((acc) => acc.id === draft.accountId)

  return (
    <div className="shrink-0 border-b border-border/60 bg-chat px-4 py-2 select-none">
      {identityCount > 1 && (
        <div className="flex items-center gap-2 border-b border-border/60 py-1.5">
          <span className={labelClass}>{t('composer.fields.from')}</span>
          <select
            value={`${draft.accountId}|${draft.fromEmail || fromAccount?.email || ''}`}
            onChange={(event) => {
              const sep = event.target.value.indexOf('|')
              const accId = event.target.value.slice(0, sep)
              const email = event.target.value.slice(sep + 1)
              update({ accountId: accId, fromEmail: email })
            }}
            className="flex-1 bg-transparent text-[0.8125rem] text-primary outline-none cursor-pointer"
          >
            {sendable.flatMap((acc) =>
              accountIdentities(acc).map((identity) => (
                <option key={`${acc.id}|${identity.email}`} value={`${acc.id}|${identity.email}`} className="bg-chats">
                  {identity.name ? `${identity.name} <${identity.email}>` : identity.email}
                </option>
              )),
            )}
          </select>
        </div>
      )}

      <div className="flex items-center gap-2 border-b border-border/60 py-1.5">
        <span className={labelClass}>{t('composer.fields.to')}</span>
        <RecipientInput
          autoFocus={focusTo}
          onTab={() => (draft.showCcBcc ? ccRef.current : subjectRef.current)?.focus()}
          value={draft.to}
          onChange={(to) => update({ to })}
          accountId={draft.accountId}
          placeholder={t('composer.placeholders.recipient')}
        />
        <button
          onClick={() => update({ showCcBcc: !draft.showCcBcc })}
          className="shrink-0 text-[0.6875rem] font-semibold text-accent hover:underline cursor-pointer"
        >
          {draft.showCcBcc ? t('composer.actions.hideCcBcc') : t('composer.actions.ccBcc')}
        </button>
      </div>

      {draft.showCcBcc && (
        <>
          <div className="flex items-center gap-2 border-b border-border/60 py-1.5">
            <span className={labelClass}>{t('composer.fields.cc')}</span>
            <RecipientInput
              inputRef={ccRef}
              value={draft.cc}
              onChange={(cc) => update({ cc })}
              accountId={draft.accountId}
              placeholder={t('composer.placeholders.commaSeparated')}
            />
          </div>
          <div className="flex items-center gap-2 border-b border-border/60 py-1.5">
            <span className={labelClass}>{t('composer.fields.bcc')}</span>
            <RecipientInput
              value={draft.bcc}
              onChange={(bcc) => update({ bcc })}
              accountId={draft.accountId}
              placeholder={t('composer.placeholders.commaSeparated')}
            />
          </div>
        </>
      )}

      <div className="flex items-center gap-2 py-1.5">
        <span className={labelClass}>{t('composer.fields.subject')}</span>
        <input
          ref={subjectRef}
          value={draft.subject}
          onChange={(event) => update({ subject: event.target.value })}
          onKeyDown={(event) => {
            if (event.key === 'Tab' && !event.shiftKey && !event.ctrlKey && !event.metaKey && !event.altKey) {
              if (onFocusBody()) event.preventDefault()
            }
          }}
          placeholder={t('composer.fields.subject')}
          spellCheck={spellCheck}
          className={fieldClass}
        />
      </div>
    </div>
  )
}
