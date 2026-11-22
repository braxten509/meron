// Renders the general, message and composer pages in desktop settings.
// Preference controls write through settings state and native operations use host commands.
// The dialog router provides the active page and navigation callbacks.

import { useState } from 'react'
import { useTranslation } from '../../lib/i18n'
import {
  CalendarDays,
  UnfoldVertical,
  Image as ImageIcon,
  Send,
  Inbox,
  SquareCheckBig,
  Columns3,
  Lock,
  Globe,
  SpellCheck,
  Quote,
  Bell,
  RefreshCw,
  MessagesSquare,
  Keyboard,
  PanelTop,
  PanelTopClose,
  Moon,
  Shrink,
  PanelBottom,
  Contrast,
} from 'lucide-react'
import { useValue } from '@legendapp/state/react'
import { showToast, ui$ } from '../../states/ui'
import {
  settings$,
  clampKanbanColumnWidth,
  KANBAN_COLUMN_MAX_WIDTH,
  KANBAN_COLUMN_MIN_WIDTH,
  sendShortcutLabel,
  setUnifiedInboxSideNavVisible,
  setCalendarSideNavVisible,
  setCalendarEnabled,
  setTasksEnabled,
  type ConversationLayout,
  type SendShortcut,
} from '../../states/settings'
import { update$ } from '../../states/update'
import { NumberRow, SegmentedRow, SettingRow, SettingsGroup, ToggleRow, SelectRow } from './AccountSettingsRows'
import { supportedI18nLanguages, languageNativeNames, type SupportedI18nLanguage } from '../../lib/i18n'
import { ThemeSettingsSection } from './ThemeSettingsSection'
import { FontSettingsSection } from './FontSettingsSection'
import { ProxySettingsSection } from './ProxySettingsCard'
import { SignatureSettingsSection } from './SignatureSettingsCard'
import { TagsSection } from './TagsSection'
import { RemoteSendersDialog } from './RemoteSendersDialog'
import { isMac } from '../../lib/shortcuts'
import { offersMonochromeTrayIcon } from '../../lib/monochromeTrayIcon'
import { ALERT_MINUTES, ALL_DAY_ALERT_DAYS, NO_ALERT, alertLabel } from '../../lib/eventAlerts'
import { setIntegratedTitlebar, windowChrome$ } from '../../lib/windowChrome'
import { SetupCopyRows, BackupRow, StorageRows, LogsRow } from './SettingsMaintenance'

const SEND_SHORTCUT_OPTIONS: { value: SendShortcut; label: string }[] = [
  { value: 'enter', label: sendShortcutLabel('enter') },
  { value: 'mod_enter', label: sendShortcutLabel('mod_enter') },
]

const CONVERSATION_LAYOUT_OPTIONS = (
  t: ReturnType<typeof useTranslation>['t'],
): { value: ConversationLayout; label: string }[] => [
  { value: 'chat', label: t('settings.appearance.conversationLayoutChat') },
  { value: 'traditional', label: t('settings.appearance.conversationLayoutTraditional') },
]

export function GeneralSection() {
  const { t } = useTranslation()
  const showRealAvatars = useValue(settings$.showRealAvatars)
  const showUnreadAccountBadge = useValue(settings$.showUnreadAccountBadge)
  const showUnifiedInbox = useValue(settings$.showUnifiedInboxInSideNav)
  const showCalendarInSideNav = useValue(settings$.showCalendarInSideNav)
  const tasksEnabled = useValue(settings$.tasksEnabled)
  const calendarEnabled = useValue(settings$.calendarEnabled)
  const reminderLeadMinutes = useValue(settings$.reminderLeadMinutes)
  const reminderAllDayDays = useValue(settings$.reminderAllDayDays)
  const kanbanColumnWidth = useValue(settings$.kanbanColumnWidth)
  const kanbanLockScroll = useValue(settings$.kanbanLockScroll)
  const language = useValue(settings$.language)

  return (
    <div className="flex flex-col gap-4">
      <SettingsGroup title={t('settings.pages.appearance')}>
        <ThemeSettingsSection />
        <SelectRow
          icon={<Globe size={15} />}
          title={t('settings.language.label')}
          hint={t('settings.language.hint')}
          value={language || ''}
          options={[
            { value: '', label: t('settings.language.system') },
            ...supportedI18nLanguages.map((lang) => ({
              value: lang,
              label: languageNativeNames[lang],
            })),
          ]}
          onChange={(value) => {
            settings$.language.set(value === '' ? null : (value as SupportedI18nLanguage))
          }}
        />
        <ToggleRow
          icon={<ImageIcon size={15} />}
          title={t('settings.appearance.showSenderImages')}
          hint={t('settings.appearance.showSenderImagesHint')}
          checked={showRealAvatars}
          onChange={() => settings$.showRealAvatars.set(!showRealAvatars)}
        />
        <TrayIconRow />
      </SettingsGroup>

      <SettingsGroup title={t('settings.sections.typography')}>
        <FontSettingsSection part="interface" />
      </SettingsGroup>

      <SettingsGroup title={t('settings.sections.sideNav')}>
        <ToggleRow
          icon={<Inbox size={15} />}
          title={t('settings.sideNav.showUnifiedInbox')}
          checked={showUnifiedInbox}
          onChange={() => setUnifiedInboxSideNavVisible(!showUnifiedInbox)}
        />
        {calendarEnabled && (
          <ToggleRow
            icon={<CalendarDays size={15} />}
            title={t('settings.sideNav.showCalendar')}
            checked={showCalendarInSideNav}
            onChange={() => setCalendarSideNavVisible(!showCalendarInSideNav)}
          />
        )}
        <ToggleRow
          icon={<Inbox size={15} />}
          title={t('settings.appearance.showUnreadAccountBadge')}
          hint={t('settings.appearance.showUnreadAccountBadgeHint')}
          checked={showUnreadAccountBadge}
          onChange={() => settings$.showUnreadAccountBadge.set(!showUnreadAccountBadge)}
        />
      </SettingsGroup>

      <SettingsGroup title={t('settings.sections.kanban')}>
        <NumberRow
          icon={<Columns3 size={15} />}
          title={t('settings.kanban.columnWidth')}
          value={String(kanbanColumnWidth)}
          min={KANBAN_COLUMN_MIN_WIDTH}
          max={KANBAN_COLUMN_MAX_WIDTH}
          step={10}
          suffix="px"
          onChange={(value) => {
            const width = Number(value)
            if (Number.isFinite(width)) settings$.kanbanColumnWidth.set(clampKanbanColumnWidth(width))
          }}
        />
        <ToggleRow
          icon={<Lock size={15} />}
          title={t('settings.kanban.lockScroll')}
          hint={t('settings.kanban.lockScrollHint')}
          checked={kanbanLockScroll}
          onChange={() => settings$.kanbanLockScroll.set(!kanbanLockScroll)}
        />
      </SettingsGroup>

      <ProxySettingsSection />

      <SettingsGroup title={t('settings.sections.app')}>
        <SettingRow
          title={t('shortcuts.title')}
          hint={t('shortcuts.customizeHint')}
          control={
            <button
              onClick={() => ui$.shortcutsOpen.set(true)}
              className="flex items-center gap-1.5 px-2.5 py-1.5 rounded-xl bg-hover hover:bg-active text-primary font-bold text-[0.625rem] cursor-pointer transition-colors"
            >
              <Keyboard size={12} />
              {t('shortcuts.customize')}
            </button>
          }
        />
        <ToggleRow
          icon={<SquareCheckBig size={15} />}
          title={t('settings.tasks.enable')}
          hint={t('settings.tasks.enableHint')}
          checked={tasksEnabled}
          onChange={() => setTasksEnabled(!tasksEnabled)}
        />
        <ToggleRow
          icon={<CalendarDays size={15} />}
          title={t('settings.calendar.enable')}
          hint={t('settings.calendar.enableHint')}
          checked={calendarEnabled}
          onChange={() => setCalendarEnabled(!calendarEnabled)}
        />
        {calendarEnabled && (
          <SelectRow
            icon={<Bell size={15} />}
            title={t('settings.calendar.reminderLead')}
            hint={t('settings.calendar.reminderLeadHint')}
            value={String(reminderLeadMinutes)}
            options={[NO_ALERT, ...ALERT_MINUTES].map((minutes) => ({
              value: String(minutes),
              label: alertLabel(minutes),
            }))}
            onChange={(value) => settings$.reminderLeadMinutes.set(Number(value))}
          />
        )}
        {calendarEnabled && (
          <SelectRow
            icon={<Bell size={15} />}
            title={t('settings.calendar.allDayAlert')}
            hint={t('settings.calendar.allDayAlertHint')}
            value={String(reminderAllDayDays)}
            options={[NO_ALERT, ...ALL_DAY_ALERT_DAYS].map((days) => ({
              value: String(days),
              label: alertLabel(days, true),
            }))}
            onChange={(value) => settings$.reminderAllDayDays.set(Number(value))}
          />
        )}
        <CloseToTrayRow />
        <IntegratedTitlebarRow />
        <AutoUpdateRow />
      </SettingsGroup>

      <SettingsGroup title={t('settings.sections.data')}>
        <SetupCopyRows />
        <BackupRow />
        <StorageRows />
        <LogsRow />
      </SettingsGroup>
    </div>
  )
}

// How received mail is shown and handled: layout, rendering, the reader's
// actions, message typography, and whose remote content loads.
export function MessagesSection() {
  const { t } = useTranslation()
  const [remoteSendersOpen, setRemoteSendersOpen] = useState(false)
  const remoteImageSenders = useValue(settings$.remoteImageSenders)
  const conversationLayout = useValue(settings$.conversationLayout)
  const chatFullMessages = useValue(settings$.chatFullMessages)
  const darkMessageBodies = useValue(settings$.darkMessageBodies)
  const autoFitMessages = useValue(settings$.autoFitMessages)
  const readerBottomActions = useValue(settings$.readerBottomActions)

  return (
    <div className="flex flex-col gap-4">
      <SettingsGroup title={t('settings.pages.appearance')}>
        <SegmentedRow
          icon={<MessagesSquare size={15} />}
          title={t('settings.appearance.conversationLayout')}
          hint={t('settings.appearance.conversationLayoutHint')}
          value={conversationLayout}
          options={CONVERSATION_LAYOUT_OPTIONS(t)}
          onChange={(value) => settings$.conversationLayout.set(value)}
        />
        {conversationLayout === 'chat' && (
          <ToggleRow
            icon={<UnfoldVertical size={15} />}
            title={t('settings.appearance.chatFullMessages')}
            hint={t('settings.appearance.chatFullMessagesHint')}
            checked={chatFullMessages}
            onChange={() => settings$.chatFullMessages.set(!chatFullMessages)}
          />
        )}
        <ToggleRow
          icon={<Moon size={15} />}
          title={t('settings.appearance.darkMessageBodies')}
          hint={t('settings.appearance.darkMessageBodiesHint')}
          checked={darkMessageBodies}
          onChange={() => settings$.darkMessageBodies.set(!darkMessageBodies)}
        />
        <ToggleRow
          icon={<Shrink size={15} />}
          title={t('settings.appearance.autoFitMessages')}
          hint={t('settings.appearance.autoFitMessagesHint')}
          checked={autoFitMessages}
          onChange={() => settings$.autoFitMessages.set(!autoFitMessages)}
        />
        <ToggleRow
          icon={<PanelBottom size={15} />}
          title={t('settings.appearance.readerBottomActions')}
          hint={t('settings.appearance.readerBottomActionsHint')}
          checked={readerBottomActions}
          onChange={() => settings$.readerBottomActions.set(!readerBottomActions)}
        />
      </SettingsGroup>

      <SettingsGroup title={t('settings.sections.typography')}>
        <FontSettingsSection part="message" />
      </SettingsGroup>

      <TagsSection />

      <SettingsGroup title={t('settings.sections.privacy')}>
        <SettingRow
          title={t('settings.privacy.remoteSenders')}
          hint={t('settings.privacy.remoteSendersHint')}
          control={
            <button
              onClick={() => setRemoteSendersOpen(true)}
              className="flex items-center gap-1.5 px-2.5 py-1.5 rounded-xl bg-hover hover:bg-active text-primary font-bold text-[0.625rem] cursor-pointer transition-colors"
            >
              {remoteImageSenders.length > 0
                ? t('settings.privacy.remoteSendersCount', { count: remoteImageSenders.length })
                : t('settings.privacy.remoteSendersNone')}
            </button>
          }
        />
      </SettingsGroup>
      {remoteSendersOpen && <RemoteSendersDialog onClose={() => setRemoteSendersOpen(false)} />}
    </div>
  )
}

export function ComposerSection() {
  const { t } = useTranslation()
  const sendShortcut = useValue(settings$.sendShortcut)
  const spellCheck = useValue(settings$.spellCheck)
  const quoteReplies = useValue(settings$.quoteReplies)

  return (
    <div className="flex flex-col gap-4">
      <SettingsGroup title={t('settings.sections.general')}>
        <ToggleRow
          icon={<SpellCheck size={15} />}
          title={t('settings.composer.spellCheck')}
          hint={t('settings.composer.spellCheckHint')}
          checked={spellCheck}
          onChange={() => settings$.spellCheck.set(!spellCheck)}
        />
        <ToggleRow
          icon={<Quote size={15} />}
          title={t('settings.composer.quoteReplies')}
          hint={t('settings.composer.quoteRepliesHint')}
          checked={quoteReplies}
          onChange={() => settings$.quoteReplies.set(!quoteReplies)}
        />
        <SegmentedRow
          icon={<Send size={15} />}
          title={t('settings.composer.sendMessageWith')}
          hint={
            sendShortcut === 'enter'
              ? t('settings.composer.sendShortcutEnterHint')
              : t('settings.composer.sendShortcutModHint', { shortcut: sendShortcutLabel('mod_enter') })
          }
          value={sendShortcut}
          options={SEND_SHORTCUT_OPTIONS}
          onChange={(value) => settings$.sendShortcut.set(value)}
        />
      </SettingsGroup>

      <SignatureSettingsSection />
    </div>
  )
}

// macOS and Linux only: Windows keeps the colour icon.
function TrayIconRow() {
  const { t } = useTranslation()
  const monochromeTrayIcon = useValue(settings$.monochromeTrayIcon)

  if (!offersMonochromeTrayIcon(navigator.userAgent, navigator.platform ?? '')) return null

  return (
    <ToggleRow
      icon={<Contrast size={15} />}
      title={t('settings.appearance.monochromeTrayIcon')}
      hint={t('settings.appearance.monochromeTrayIconHint')}
      checked={monochromeTrayIcon}
      onChange={() => settings$.monochromeTrayIcon.set(!monochromeTrayIcon)}
    />
  )
}

// macOS has its own split between closing a window and quitting (⌘Q), so the
// close button's behaviour is only a choice elsewhere.
function CloseToTrayRow() {
  const { t } = useTranslation()
  const closeToTray = useValue(settings$.closeToTray)

  if (isMac) return null

  return (
    <ToggleRow
      icon={<PanelTopClose size={15} />}
      title={t('settings.window.closeToTray')}
      hint={t('settings.window.closeToTrayHint', { quit: t('tray.quitMeron') })}
      checked={closeToTray}
      onChange={() => settings$.closeToTray.set(!closeToTray)}
    />
  )
}

// Linux and Windows. The choice lives in Go (window.json), since it is
// needed before the window exists. Windows and Linux desktops with native
// server-side decorations apply it on restart, so the switch shows the saved choice.
function IntegratedTitlebarRow() {
  const { t } = useTranslation()
  const supported = useValue(windowChrome$.supported)
  const wanted = useValue(windowChrome$.wanted)

  if (!supported) return null

  return (
    <ToggleRow
      icon={<PanelTop size={15} />}
      title={t('settings.window.integratedTitlebar')}
      hint={t('settings.window.integratedTitlebarHint')}
      checked={wanted}
      onChange={() =>
        void setIntegratedTitlebar(!wanted)
          .then(() => {
            if (windowChrome$.wanted.peek() !== windowChrome$.integrated.peek()) {
              showToast(t('settings.window.restartToApply'), 'success', 4000)
            }
          })
          .catch(() => {})
      }
    />
  )
}

// Only meaningful where the app can actually replace itself; store-managed and
// dev builds get no toggle at all rather than one that does nothing.
function AutoUpdateRow() {
  const { t } = useTranslation()
  const status = useValue(update$.status)
  const autoUpdateCheck = useValue(settings$.autoUpdateCheck)

  if (!status.supported) return null

  return (
    <ToggleRow
      icon={<RefreshCw size={15} />}
      title={t('settings.updates.autoCheck')}
      hint={t('settings.updates.autoCheckHint')}
      checked={autoUpdateCheck}
      onChange={() => settings$.autoUpdateCheck.set(!autoUpdateCheck)}
    />
  )
}
