// Routes mail notifications and reconciles their delivery with the store.
// The application delegate opens the thread selected from a notification.
// Background refresh uses the same service to update delivered notifications.

import Foundation
import MeronUI
import UIKit
import UserNotifications

extension Notification.Name {
    static let iosNotificationThreadTargetOpened = Notification.Name("iosNotificationThreadTargetOpened")
}

struct IosNotificationThreadTarget: Equatable {
    let accountId: String
    let folder: String
    let threadKey: String
    let subject: String
}

enum IosNotificationPayloadKey {
    static let accountId = "accountId"
    static let folder = "folder"
    static let threadKey = "threadKey"
    static let subject = "subject"
    /// The core store rows the notification announced, as `[folder, message
    /// id]` pairs; see [IosNewMailReconciler].
    static let storeMessages = "storeMessages"
}

func iosNotificationThreadTarget(userInfo: [AnyHashable: Any]) -> IosNotificationThreadTarget? {
    let accountId = (userInfo[IosNotificationPayloadKey.accountId] as? String ?? "").trimmingCharacters(in: .whitespacesAndNewlines)
    let folder = (userInfo[IosNotificationPayloadKey.folder] as? String ?? "").trimmingCharacters(in: .whitespacesAndNewlines)
    let threadKey = (userInfo[IosNotificationPayloadKey.threadKey] as? String ?? "").trimmingCharacters(in: .whitespacesAndNewlines)
    let subject = userInfo[IosNotificationPayloadKey.subject] as? String ?? ""
    guard !accountId.isEmpty, !folder.isEmpty, !threadKey.isEmpty else { return nil }
    return IosNotificationThreadTarget(accountId: accountId, folder: folder, threadKey: threadKey, subject: subject)
}

/// The payload a tapped notification is republished with, keyed the same way a
/// notification's own userInfo is so [iosNotificationThreadTarget] reads both.
func iosNotificationThreadTargetUserInfo(_ target: IosNotificationThreadTarget) -> [String: Any] {
    [
        IosNotificationPayloadKey.accountId: target.accountId,
        IosNotificationPayloadKey.folder: target.folder,
        IosNotificationPayloadKey.threadKey: target.threadKey,
        IosNotificationPayloadKey.subject: target.subject,
    ]
}

func iosNotificationThreadId(_ target: IosNotificationThreadTarget) -> String {
    // A feed is a thread, keyed by its subscription rather than by folder. Core
    // mints RSS account ids as "rss-<uuid>", the same prefix the desktop bridge
    // and the Android client route on.
    if target.accountId.hasPrefix("rss-") {
        return "\(target.accountId)#rss#\(target.threadKey)"
    }
    let folder = target.folder.caseInsensitiveCompare("inbox") == .orderedSame ? "INBOX" : target.folder
    if target.threadKey.hasPrefix("uid:") {
        return "\(target.accountId)#\(folder)#\(String(target.threadKey.dropFirst(4)))"
    }
    let compoundKey: String
    if target.threadKey.hasPrefix("gmthrid:") {
        compoundKey = target.threadKey
    } else {
        compoundKey = target.threadKey + "#" + iosThreadGroupingSubject(target.subject)
    }
    let encoded = Data(compoundKey.utf8)
        .base64EncodedString()
        .replacingOccurrences(of: "+", with: "-")
        .replacingOccurrences(of: "/", with: "_")
        .replacingOccurrences(of: "=", with: "")
    return "\(target.accountId)#\(folder)#t.\(encoded)"
}

func iosThreadGroupingSubject(_ subject: String) -> String {
    var value = subject.trimmingCharacters(in: .whitespacesAndNewlines)
    let prefixPattern = #"(?i)^(?:\[[^\]]+\]\s*)*(?:re|fw|fwd|aw|sv|vs|rv|res|tr|antw|wg|答复|回复|转发)(?:\[[0-9]+\]|\([0-9]+\))?[:：]\s*"#
    let tagPattern = #"^\s*\[[^\]]*\]\s*"#
    while true {
        if let range = value.range(of: prefixPattern, options: .regularExpression), range.lowerBound == value.startIndex {
            value = String(value[range.upperBound...]).trimmingCharacters(in: .whitespacesAndNewlines)
            continue
        }
        if let range = value.range(of: tagPattern, options: .regularExpression), range.lowerBound == value.startIndex {
            value = String(value[range.upperBound...]).trimmingCharacters(in: .whitespacesAndNewlines)
            continue
        }
        return value
    }
}

final class IosNotificationDelegate: NSObject, UIApplicationDelegate, UNUserNotificationCenterDelegate {
    func application(
        _: UIApplication,
        didFinishLaunchingWithOptions _: [UIApplication.LaunchOptionsKey: Any]? = nil
    ) -> Bool {
        UNUserNotificationCenter.current().delegate = self
        return true
    }

    func userNotificationCenter(
        _: UNUserNotificationCenter,
        didReceive response: UNNotificationResponse,
        withCompletionHandler completionHandler: @escaping () -> Void
    ) {
        if let target = iosNotificationThreadTarget(userInfo: response.notification.request.content.userInfo) {
            NotificationCenter.default.post(
                name: .iosNotificationThreadTargetOpened,
                object: nil,
                userInfo: iosNotificationThreadTargetUserInfo(target)
            )
        }
        IosReminders.refresh()
        completionHandler()
    }

    func userNotificationCenter(
        _: UNUserNotificationCenter,
        willPresent notification: UNNotification,
        withCompletionHandler completionHandler: @escaping (UNNotificationPresentationOptions) -> Void
    ) {
        let isReminder = notification.request.identifier.hasPrefix("meron-reminder:")
        if isReminder { IosReminders.refresh() }
        completionHandler(isReminder ? [.banner, .sound] : [])
    }
}

enum IosNotificationService {
    static func requestAuthorization(completion: @escaping (Bool) -> Void) {
        UNUserNotificationCenter.current().requestAuthorization(options: [.alert, .badge, .sound]) { granted, _ in
            DispatchQueue.main.async {
                if granted { IosReminders.refresh() }
                completion(granted)
            }
        }
    }

    static func refreshNotificationContent(_ body: String) -> UNMutableNotificationContent {
        let content = UNMutableNotificationContent()
        content.title = String(localized: "mobile.ios.refreshCompleteTitle")
        content.body = body
        content.sound = .default
        return content
    }

    static func notifyRefreshComplete(_ body: String) {
        let request = UNNotificationRequest(
            identifier: "meron-refresh-complete",
            content: refreshNotificationContent(body),
            trigger: nil
        )
        UNUserNotificationCenter.current().add(request)
    }

    static func notifyNewMailContentForTesting(
        accountName: String,
        from: String,
        subject: String,
        count: Int,
        accountId: String = "",
        folder: String = "",
        threadKey: String = "",
        storeMessages: [IosStoreMessage] = []
    ) -> UNMutableNotificationContent {
        let content = UNMutableNotificationContent()
        content.title = accountName.isEmpty ? String(localized: "mobile.ios.newMailTitle") : accountName
        let sender = from.trimmingCharacters(in: .whitespacesAndNewlines)
        let messageSubject = subject.trimmingCharacters(in: .whitespacesAndNewlines)
        if count > 1 {
            content.body = localizedCatalogString("mobile.ios.newMailCountBody", args: ["count": count])
        } else if !sender.isEmpty, !messageSubject.isEmpty {
            content.body = "\(sender): \(messageSubject)"
        } else {
            content.body = messageSubject.isEmpty ? String(localized: "mobile.ios.newMailBodyFallback") : messageSubject
        }
        content.sound = .default
        var userInfo: [String: Any] = [:]
        if !accountId.isEmpty, !folder.isEmpty, !threadKey.isEmpty {
            userInfo[IosNotificationPayloadKey.folder] = folder
            userInfo[IosNotificationPayloadKey.threadKey] = threadKey
            userInfo[IosNotificationPayloadKey.subject] = subject
        }
        if !accountId.isEmpty, !storeMessages.isEmpty {
            userInfo[IosNotificationPayloadKey.storeMessages] = storeMessages.map { [$0.folder, $0.messageId] }
        }
        if !userInfo.isEmpty {
            userInfo[IosNotificationPayloadKey.accountId] = accountId
            content.userInfo = userInfo
        }
        return content
    }

    static func notifyNewMail(
        accountName: String,
        from: String,
        subject: String,
        count: Int,
        accountId: String = "",
        folder: String = "",
        threadKey: String = "",
        storeMessages: [IosStoreMessage] = []
    ) {
        let content = notifyNewMailContentForTesting(
            accountName: accountName,
            from: from,
            subject: subject,
            count: count,
            accountId: accountId,
            folder: folder,
            threadKey: threadKey,
            storeMessages: storeMessages
        )
        let request = UNNotificationRequest(
            identifier: "\(iosNewMailNotificationPrefix)\(UUID().uuidString)",
            content: content,
            trigger: nil
        )
        UNUserNotificationCenter.current().add(request)
    }
}

let iosNewMailNotificationPrefix = "meron-new-mail-"

/// A core store row: mail by folder and UID, a feed entry by its subscription
/// and entry key — what `mail.stillUnread` looks up.
struct IosStoreMessage: Hashable {
    let folder: String
    let messageId: String
}

struct IosStoreRow: Hashable {
    let accountId: String
    let message: IosStoreMessage
}

/// A delivered new-mail notification and the messages it announced.
struct IosTrackedNotification: Equatable {
    let identifier: String
    let accountId: String
    let messages: [IosStoreMessage]
}

/// Read a delivered notification back, or nil for one that is not new mail or
/// predates `storeMessages` — those are left to the user.
func iosTrackedNotification(identifier: String, userInfo: [AnyHashable: Any]) -> IosTrackedNotification? {
    guard identifier.hasPrefix(iosNewMailNotificationPrefix),
          let accountId = userInfo[IosNotificationPayloadKey.accountId] as? String, !accountId.isEmpty,
          let pairs = userInfo[IosNotificationPayloadKey.storeMessages] as? [[String]]
    else { return nil }
    let messages = pairs.compactMap { pair in
        pair.count == 2 ? IosStoreMessage(folder: pair[0], messageId: pair[1]) : nil
    }
    guard !messages.isEmpty else { return nil }
    return IosTrackedNotification(identifier: identifier, accountId: accountId, messages: messages)
}

/// The notifications none of whose messages is still unread. One notification
/// covers a whole batch, so it stays while any arrival in it is unread.
func iosStaleNotificationIds(_ tracked: [IosTrackedNotification], stillUnread: Set<IosStoreRow>) -> [String] {
    tracked
        .filter { notification in
            !notification.messages.contains { stillUnread.contains(IosStoreRow(accountId: notification.accountId, message: $0)) }
        }
        .map(\.identifier)
}

/// Commands after which a message may no longer be unread, or no longer there:
/// a sync that brought in another device's changes, or the app's own read,
/// move and delete writes. Matches the Android client's list.
func iosCommandMayStaleNotifications(_ command: String) -> Bool {
    [
        "mail.sync", "mail.markRead", "mail.markAllRead", "mail.archive", "mail.delete", "mail.deleteMany",
        "mail.move", "mail.emptyFolder", "mail.folderDelete", "account.remove", "rss.sync", "rss.markRead",
        "feed.remove",
    ].contains(command)
}

/// Withdraws new-mail notifications whose mail is no longer unread, the way
/// Gmail does: read, archived or deleted here, or — once a sync has brought the
/// change in — on another device. The delivered notifications are compared
/// against core's store rather than tracking each action.
enum IosNewMailReconciler {
    private static let queue = DispatchQueue(label: "jp.nonbili.meron.new-mail-reconcile", qos: .utility)
    private static var pending: DispatchWorkItem?

    /// Coalesces a burst of syncs and writes into one pass.
    static func request() {
        queue.async {
            pending?.cancel()
            let work = DispatchWorkItem { reconcile() }
            pending = work
            queue.asyncAfter(deadline: .now() + 0.5, execute: work)
        }
    }

    /// Background tasks reconcile before reporting completion to iOS.
    static func reconcileNow() {
        queue.sync {
            pending?.cancel()
            pending = nil
            reconcile()
        }
    }

    private static func reconcile() {
        let center = UNUserNotificationCenter.current()
        let delivered = DispatchSemaphore(value: 0)
        var tracked: [IosTrackedNotification] = []
        center.getDeliveredNotifications { notifications in
            tracked = notifications.compactMap {
                iosTrackedNotification(identifier: $0.request.identifier, userInfo: $0.request.content.userInfo)
            }
            delivered.signal()
        }
        guard delivered.wait(timeout: .now() + 5) == .success, !tracked.isEmpty,
              let stillUnread = fetchStillUnread(tracked)
        else { return }
        let stale = iosStaleNotificationIds(tracked, stillUnread: stillUnread)
        if !stale.isEmpty {
            center.removeDeliveredNotifications(withIdentifiers: stale)
        }
    }

    /// The rows core still holds unread, or nil when it could not say — in
    /// which case nothing is withdrawn.
    private static func fetchStillUnread(_ tracked: [IosTrackedNotification]) -> Set<IosStoreRow>? {
        let messages = tracked.flatMap { notification in
            notification.messages.map {
                ["account_id": notification.accountId, "folder_id": $0.folder, "message_id": $0.messageId]
            }
        }
        let request: [String: Any] = ["id": 94, "method": "mail.stillUnread", "params": ["messages": messages]]
        guard let data = try? JSONSerialization.data(withJSONObject: request),
              let json = String(data: data, encoding: .utf8),
              let responseData = RustCoreBridge.invokeJson(json).data(using: .utf8),
              let response = try? JSONSerialization.jsonObject(with: responseData) as? [String: Any],
              let result = response["result"] as? [String: Any],
              let unread = result["unread"] as? [[String: Any]]
        else { return nil }
        return Set(unread.map {
            IosStoreRow(
                accountId: $0["account_id"] as? String ?? "",
                message: IosStoreMessage(folder: $0["folder_id"] as? String ?? "", messageId: $0["message_id"] as? String ?? "")
            )
        })
    }
}
