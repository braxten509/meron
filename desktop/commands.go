// Routes desktop commands from the frontend to their owning services.
// The Wails bridge calls Invoke, which handles logging and notification reconciliation.
// Individual handlers stay with the feature that implements them.
package main

import (
	"fmt"
	"time"
)

// quietCommands succeed without a log line: they arrive at frame rate and
// would drown out everything else in the log.
var quietCommands = map[string]bool{"window.resized": true, "setupCopy.status": true}

func (a *App) Invoke(command string, payload map[string]any) (result any, err error) {
	start := time.Now()
	defer a.recoverInvoke(command, &err)
	result, err = a.invoke(command, payload)
	if err == nil && notifyReconcileCommands[command] {
		a.requestNotificationReconcile()
	}
	if err == nil && quietCommands[command] {
		return result, nil
	}
	if err != nil && aiSortMethods[command] != "" {
		// An AI service's error can echo the request or the key; only the
		// screen gets the core's message.
		a.logf("invoke %s failed after %s (message not logged)", command, time.Since(start).Round(time.Millisecond))
	} else if err != nil {
		a.logf("invoke %s failed after %s: %v", command, time.Since(start).Round(time.Millisecond), err)
	} else {
		a.logf("invoke %s ok after %s", command, time.Since(start).Round(time.Millisecond))
	}
	return result, err
}

func (a *App) invoke(command string, payload map[string]any) (any, error) {
	if payload == nil {
		payload = map[string]any{}
	}
	switch command {
	case "mcp.status", "mcp.enable", "mcp.setPort", "mcp.clientSave", "mcp.clientRegenerate", "mcp.clientRevoke", "mcp.pending", "mcp.resolve":
		return a.mcpSettings(command, payload)
	case "system.check":
		return a.systemCheck()
	case "system.appearance":
		return a.systemAppearance()
	case "system.pickImageFile":
		return a.pickImageFile(payload)
	case "system.pickFiles":
		return a.pickFiles(payload, false, true)
	case "system.pickImageFiles":
		return a.pickFiles(payload, true, true)
	case "system.readDroppedFiles":
		return a.readDroppedFilesCommand(payload)
	case "mailto.consumePending":
		return a.consumePendingMailto(), nil
	case "core.status":
		return map[string]any{"migrating": a.sidecar != nil && a.sidecar.Migrating()}, nil
	case "app.prefsGet":
		return a.appPrefsGet(payload)
	case "app.prefsSet":
		return a.appPrefsSet(payload)
	case "contacts.noteGet", "contacts.noteSet":
		if a.sidecar == nil || !a.sidecar.Started() {
			return nil, a.engineUnavailable()
		}
		method := "app.prefsGet"
		if command == "contacts.noteSet" {
			method = "app.prefsSet"
		}
		return a.sidecar.Call(method, payload)
	case "tray.setUnread":
		return a.traySetUnread(payload)
	case "tray.getStyle":
		return a.trayGetStyle(), nil
	case "tray.setStyle":
		return a.traySetStyle(payload)
	case "window.resized":
		return a.windowResized()
	case "window.ready":
		a.revealWindow(false)
		return map[string]any{"ok": true}, nil
	case "window.chrome":
		return a.windowChrome()
	case "window.setTitlebar":
		return a.windowSetTitlebar(payload)
	case "window.minimise", "window.toggleMaximise", "window.close":
		return a.windowControl(command)
	case "window.setCloseToTray":
		return a.windowSetCloseToTray(payload)
	case "app.quit":
		a.requestQuit()
		return map[string]any{"ok": true}, nil
	case "window.setAppearance":
		return a.setWindowAppearance(payload)
	case "i18n.setNativeLabels":
		return a.setNativeLabels(payload)
	case "changelog.fetch":
		return a.changelogFetch()
	case "search.parse", "search.build":
		return a.searchQuery(command, payload)
	case "update.status":
		return a.updateStatus()
	case "update.check":
		return a.updateCheck()
	case "update.download":
		return a.updateDownload()
	case "update.install":
		return a.updateInstall()
	case "log.read":
		return a.logRead()
	// A screen-side problem worth finding later in the log (no reply sent, ...).
	case "log.note":
		text, _ := payload["text"].(string)
		if len(text) > 2000 {
			text = text[:2000]
		}
		a.logf("ui: %s", text)
		return map[string]any{"ok": true}, nil
	case "log.export":
		return a.logExport()
	case "storage.usage":
		return a.storageUsage()
	case "storage.clearCache":
		return a.storageClearCache()
	case "account.list":
		return a.accountList()
	case "account.addPassword":
		return a.accountAddPassword(payload)
	case "account.autodiscover":
		return a.autodiscover(payload)
	case "account.probeCert":
		return a.accountProbeCert(payload)
	case "account.setCertPin":
		return a.accountSetCertPin(payload)
	case "account.addGmailOAuth", "oauth.gmailComplete":
		return a.accountAddGmailOAuth(payload)
	case "account.addOutlookOAuth", "oauth.outlookComplete":
		return a.accountAddOutlookOAuth(payload)
	case "account.addRSS":
		return a.accountAddRSS(payload)
	case "account.addLocal":
		return a.accountAddLocal(payload)
	case "account.localStats":
		return a.accountLocalStats()
	case "account.localBackup":
		return a.accountLocalBackup(payload)
	case "account.localRestore":
		return a.accountLocalRestore(payload)
	case "rss.addFeed":
		return a.feedAdd(payload)
	case "rss.removeFeed":
		return a.feedRemove(payload)
	case "rss.moveFeed":
		return a.feedMove(payload)
	case "rss.exportOpml":
		return a.exportOpml(payload)
	case "rss.importOpml":
		return a.importOpml(payload)
	case "tasks.lists", "tasks.listCreate", "tasks.listRename", "tasks.listDelete",
		"tasks.listReorder", "tasks.items", "tasks.create", "tasks.update",
		"tasks.setDone", "tasks.delete", "tasks.reorder", "tasks.clearCompleted",
		"tasks.restore", "tasks.forThread":
		return a.tasksInvoke(command, payload)
	case "invite.respond", "calendar.events", "calendar.get", "calendar.create",
		"calendar.update", "calendar.delete", "calendar.restore", "invite.removeFromCalendar",
		"invite.addToCalendar":
		return a.calendarInvoke(command, payload)
	case "backup.export":
		return a.exportBackup(payload)
	case "backup.import":
		return a.importBackup(payload)
	case "account.remove":
		return a.accountRemove(payload)
	case "account.setImages":
		return a.accountSetImages(payload)
	case "account.setConversationHtml":
		return a.accountSetPref(payload, "account.setConversationHtml")
	case "account.setChatWallpaper":
		return a.accountSetChatWallpaper(payload)
	case "account.setProxy":
		return a.accountSetProxy(payload)
	case "account.setName":
		return a.accountSetName(payload)
	case "account.setSenderName":
		return a.accountSetSenderName(payload)
	case "account.setAvatar":
		return a.accountSetAvatar(payload)
	case "account.setAliases":
		return a.accountSetAliases(payload)
	case "account.setSignature":
		return a.accountSetSignature(payload)
	case "account.setUnified":
		return a.accountSetPref(payload, "account.setUnified")
	case "account.setMuted":
		return a.accountSetPref(payload, "account.setMuted")
	case "account.setPaused":
		return a.accountSetPref(payload, "account.setPaused")
	case "account.setSaveSentCopy":
		return a.accountSetSaveSentCopy(payload)
	case "account.setRSSSyncInterval":
		return a.accountSetRSSSyncInterval(payload)
	case "account.reorder":
		return a.accountReorder(payload)
	case "oauth.gmailBegin":
		return a.gmailBegin()
	case "oauth.outlookBegin":
		return a.outlookBegin()
	case "oauth.gmailPollProfile", "oauth.outlookPollProfile":
		return a.oauthPollProfile(), nil
	case "oauth.cancel":
		a.cancelOAuth()
		return map[string]any{"ok": true}, nil
	case "oauth.reopen":
		return map[string]any{"ok": a.reopenOAuth()}, nil
	case "mail.sync":
		return a.mailSync(payload)
	case "watch.start", "watch.stop":
		return a.watchFolder(command, payload)
	case "mail.folderList":
		return a.folderList(payload)
	case "mail.folderCreate":
		return a.folderCreate(payload)
	case "mail.threadList", "mail.search":
		return a.threadList(payload)
	case "mail.starredItems":
		return a.starredItems(payload)
	case "mail.allocateIdentity":
		if a.sidecar == nil || !a.sidecar.Started() {
			return nil, a.engineUnavailable()
		}
		return a.sidecar.Call("identity.allocate", payload)
	case "mail.suggestContacts":
		return a.suggestContacts(payload)
	case "mail.contactThreads":
		if a.sidecar == nil || !a.sidecar.Started() {
			return nil, a.engineUnavailable()
		}
		res, err := a.sidecar.Call("contacts.threads", payload)
		if err != nil {
			return nil, err
		}
		return threadsJSON("", "INBOX", res), nil
	case "mail.threadRead":
		return a.threadRead(payload)
	case "mail.send":
		return a.mailSend(payload)
	case "mail.saveDraft":
		return a.mailSaveDraft(payload)
	case "mail.discardDraft":
		return a.mailDiscardDraft(payload)
	case "mail.markRead":
		return a.markRead(payload)
	case "mail.markStarred":
		return a.markStarred(payload)
	case "mail.setTags":
		return a.setTags(payload)
	case "mail.tagList":
		return a.tagList()
	case "mail.replaceTag":
		return a.replaceTag(payload)
	case "mail.rules":
		return a.rules()
	case "mail.setRules":
		return a.setRules(payload)
	case "mail.ruleMatches":
		return a.ruleMatches(payload)
	case "mail.runRule":
		return a.runRule(payload)
	case "mail.ruleStatus":
		return a.ruleStatus()
	case "mail.stopRule":
		return a.stopRule(payload)
	case "mail.aiSortConfig", "mail.setAiSortConfig", "mail.setAiSortKey", "mail.checkAiSortKey",
		"mail.aiSortModels", "mail.aiSortEstimate", "mail.runAiSort", "mail.stopAiSort",
		"mail.aiSortStatus", "mail.aiSortMoveBack", "mail.aiSortLessons", "mail.aiSortForgetLesson":
		return a.aiSort(command, payload)
	case "mail.meronFolders", "mail.createMeronFolder", "mail.renameMeronFolder", "mail.setMeronFolderNotify", "mail.deleteMeronFolder",
		"mail.moveToMeronFolder", "mail.moveOutOfMeronFolder", "mail.meronFolderThreads",
		"mail.markMeronFolderRead":
		return a.meronFolders(command, payload)
	case "setupCopy.host", "setupCopy.join", "setupCopy.status", "setupCopy.setKeep",
		"setupCopy.copy", "setupCopy.cancel", "setupCopy.safetyInfo", "setupCopy.restoreSafety":
		return a.setupCopy(command, payload)
	case "mail.markAllRead":
		return a.markAllRead(payload)
	case "mail.emptyFolder":
		return a.emptyFolder(payload)
	case "mail.folderDelete":
		return a.folderDelete(payload)
	case "mail.folderRename":
		return a.folderRename(payload)
	case "mail.folderSetNotify":
		return a.folderSetNotify(payload)
	case "mail.move":
		return a.mailMove(payload)
	case "mail.copy":
		return a.mailCopy(payload)
	case "mail.saveAttachment":
		return a.saveAttachment(payload)
	case "mail.cacheRemoteImage":
		return a.cacheRemoteImage(payload)
	case "mail.openAttachment":
		return a.openAttachment(payload)
	case "mail.print":
		html, _ := payload["html"].(string)
		printed, fallback, err := printNativeMail(html)
		if fallback != "" {
			a.logf("mail.print: native print declined (%s), using the browser fallback", fallback)
		}
		return printed, err
	case "mail.saveEml":
		return a.saveMessageEml(payload)
	case "mail.exportEml":
		return a.exportEml(payload)
	case "mail.exportEmlCancel":
		a.cancelEmlExport()
		return map[string]any{"ok": true}, nil
	case "mail.copyImage":
		return a.copyImage(payload)
	case "mail.readAttachment":
		return a.readAttachment(payload)
	case "composer.readClipboardImage":
		return a.readClipboardImageAttachment(payload)
	case "composer.writeMediaFile":
		return a.writeMediaFile(payload)
	case "composer.openAttachment":
		return a.openComposerAttachment(payload)
	case "avatar.resolve":
		return a.sidecar.Call("avatar.resolve", payload)
	case "account.writeAvatarFile":
		return a.writeAvatarFile(payload)
	case "account.writeChatWallpaperFile":
		return a.writeChatWallpaperFile(payload)
	case "composer.pruneMedia":
		return a.pruneComposerMedia(payload)
	case "mail.archive":
		return a.mailArchive(payload)
	case "mail.delete":
		return a.mailDelete(payload)
	case "mail.deleteMany":
		return a.mailDeleteMany(payload)
	case "mail.moveMany":
		return a.mailMoveMany(payload)
	default:
		return nil, fmt.Errorf("unknown command: %s", command)
	}
}
