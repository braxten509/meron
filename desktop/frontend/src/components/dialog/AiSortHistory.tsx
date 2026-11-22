// Shows prior AI sorting decisions and lessons learned from manual moves.
// Each list uses the provider result and current folder labels.
// The settings panel owns pagination and handles undo and forget requests.

import { ChevronLeft, Undo2, X } from 'lucide-react'
import { useTranslation } from '../../lib/i18n'
import { type AiSortLesson, type AiSortLogEntry } from '../../states/aiSort'
import { Button } from '../button/Button'
import { IconButton } from '../button/IconButton'
import { useRuleTargetLabel } from './RuleFolderPicker'

export function LessonList({
  lessons,
  shown,
  onMore,
  onBack,
  forgetting,
  forgetLesson,
}: {
  lessons: AiSortLesson[]
  shown: number
  onMore: () => void
  onBack: () => void
  forgetting: number | null
  forgetLesson: (item: AiSortLesson) => Promise<void>
}) {
  const { t } = useTranslation()
  const lessonPlace = (lesson: AiSortLesson) => {
    if (lesson.place === 'spam') return t('aiSort.lessonSpam')
    if (lesson.place === 'inbox') return t('aiSort.lessonInbox')
    return lesson.place_name
  }

  const lessonUnused = (lesson: AiSortLesson) => {
    if (lesson.used) return ''
    return lesson.place === 'spam' ? t('aiSort.lessonUnusedSpam') : t('aiSort.lessonUnusedMeron')
  }

  return (
    <div className="flex flex-col gap-3">
      <div className="flex items-center gap-2">
        <IconButton icon={ChevronLeft} iconSize={14} label={t('buttons.back')} size="sm" onClick={onBack} />
        <h3 className="text-sm font-semibold text-primary">{t('aiSort.lessons')}</h3>
      </div>
      <p className="px-1 text-xs text-secondary">{t('aiSort.lessonsHint')}</p>
      <div className="divide-y divide-border/40 overflow-hidden rounded-2xl border border-border/60 bg-raised/80 shadow-sm shadow-black/[0.03] dark:shadow-black/10">
        {lessons.length === 0 && <p className="px-3.5 py-2.5 text-xs text-secondary">{t('aiSort.lessonsEmpty')}</p>}
        {lessons.slice(0, shown).map((lesson) => {
          const name = lessonSenderName(lesson.sender)
          const tagClass = !lesson.used
            ? 'bg-secondary/10 text-secondary'
            : lesson.place === 'spam'
              ? 'bg-rose-500/10 text-rose-500'
              : lesson.place === 'inbox'
                ? 'bg-secondary/10 text-secondary'
                : 'bg-accent/10 text-accent'
          return (
            <div
              key={lesson.id}
              className="flex min-h-10 items-center gap-2 py-1 pr-1.5 pl-3.5 text-xs"
              title={[lessonUnused(lesson), lesson.sender, lesson.subject, new Date(lesson.at * 1000).toLocaleString()]
                .filter(Boolean)
                .join(' · ')}
            >
              <span className={`min-w-0 flex-1 truncate ${lesson.used ? 'text-primary' : 'text-secondary'}`}>
                {name}
                {name && lesson.subject && <span className="text-secondary">{' · '}</span>}
                <span className="text-secondary">{lesson.subject}</span>
              </span>
              <span
                className={`max-w-40 shrink-0 truncate rounded-full px-2 text-[0.625rem] font-medium leading-5 ${tagClass}`}
              >
                {lessonPlace(lesson)}
              </span>
              {/* The same room on every line, so the tags line up. */}
              <span className="flex w-7 shrink-0 justify-center">
                <IconButton
                  icon={X}
                  iconSize={13}
                  label={t('aiSort.lessonForget')}
                  size="sm"
                  disabled={forgetting !== null}
                  onClick={() => void forgetLesson(lesson)}
                />
              </span>
            </div>
          )
        })}
      </div>
      {lessons.length > shown && (
        <Button variant="ghost" size="sm" onClick={onMore}>
          {t('aiSort.viewMore')}
        </Button>
      )}
    </div>
  )
}

export function DecisionLog({
  entries,
  shown,
  onMore,
  onBack,
  movingBack,
  moveBack,
}: {
  entries: AiSortLogEntry[]
  shown: number
  onMore: () => void
  onBack: () => void
  movingBack: number | null
  moveBack: (item: AiSortLogEntry) => Promise<void>
}) {
  const { t } = useTranslation()
  const targetLabel = useRuleTargetLabel()
  const logLabel = (entry: AiSortLogEntry) => {
    const folder = targetLabel({ accountId: entry.account, folderId: entry.to_folder })
    switch (entry.state) {
      case 'moving':
      case 'moved':
        // The core already answers with the Meron folder's current name.
        return entry.meron_folder_id
          ? t('aiSort.logMovedMeron', { folder: entry.to_folder })
          : t('aiSort.logMoved', { folder })
      case 'moved_back':
        return t('aiSort.logMovedBack')
      case 'left':
        return t('aiSort.logLeft')
      case 'slow':
        return t('aiSort.logSlow')
      default:
        return t('aiSort.logFailed')
    }
  }

  const tagLabel = (entry: AiSortLogEntry) => {
    switch (entry.state) {
      case 'moving':
      case 'moved':
        return entry.meron_folder_id
          ? entry.to_folder
          : targetLabel({ accountId: entry.account, folderId: entry.to_folder })
      case 'moved_back':
        return t('aiSort.tagMovedBack')
      case 'left':
        return t('aiSort.tagLeft')
      case 'slow':
        return t('aiSort.tagSlow')
      default:
        return t('aiSort.tagFailed')
    }
  }

  return (
    <div className="flex flex-col gap-3">
      <div className="flex items-center gap-2">
        <IconButton icon={ChevronLeft} iconSize={14} label={t('buttons.back')} size="sm" onClick={onBack} />
        <h3 className="text-sm font-semibold text-primary">{t('aiSort.log')}</h3>
      </div>
      <div className="divide-y divide-border/40 overflow-hidden rounded-2xl border border-border/60 bg-raised/80 shadow-sm shadow-black/[0.03] dark:shadow-black/10">
        {entries.length === 0 && <p className="px-3.5 py-2.5 text-xs text-secondary">{t('aiSort.logEmpty')}</p>}
        {entries.slice(0, shown).map((entry) => {
          const moved = entry.state === 'moved' || entry.state === 'moving'
          const failed = entry.state === 'failed'
          const tagClass = failed
            ? 'bg-rose-500/10 text-rose-500'
            : moved
              ? 'bg-accent/10 text-accent'
              : 'bg-secondary/10 text-secondary'
          return (
            <div
              key={entry.id}
              className="flex min-h-10 items-center gap-2 py-1 pr-1.5 pl-3.5 text-xs"
              title={[logLabel(entry), entry.sender, entry.reason, new Date(entry.at * 1000).toLocaleString()]
                .filter(Boolean)
                .join(' · ')}
            >
              <span className={`min-w-0 flex-1 truncate ${moved ? 'text-primary' : 'text-secondary'}`}>
                {entry.subject || entry.sender}
              </span>
              <span
                className={`max-w-40 shrink-0 truncate rounded-full px-2 text-[0.625rem] font-medium leading-5 ${tagClass}`}
              >
                {tagLabel(entry)}
              </span>
              {/* The same room on every line, so the tags line up. */}
              <span className="flex w-7 shrink-0 justify-center">
                {entry.state === 'moved' && (
                  <IconButton
                    icon={Undo2}
                    iconSize={13}
                    label={t('aiSort.moveBack')}
                    size="sm"
                    disabled={movingBack !== null}
                    onClick={() => void moveBack(entry)}
                  />
                )}
              </span>
            </div>
          )
        })}
      </div>
      {entries.length > shown && (
        <Button variant="ghost" size="sm" onClick={onMore}>
          {t('aiSort.viewMore')}
        </Button>
      )}
    </div>
  )
}

/** The name of a "Name <address>" sender, or the address when it has none. */
function lessonSenderName(sender: string): string {
  const at = sender.lastIndexOf('<')
  const name = at > 0 ? sender.slice(0, at).trim() : ''
  return name || sender.replace(/[<>]/g, '').trim()
}
