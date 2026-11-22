// Resolves the iOS directories used by Meron's host and
// shared services. Central path selection keeps the core
// database and platform storage in their intended
// locations.

import Foundation

enum IosAppPaths {
    static func mobileDataDirectory() -> String {
        let fileManager = FileManager.default
        let base = fileManager.urls(for: .applicationSupportDirectory, in: .userDomainMask).first
            ?? fileManager.temporaryDirectory
        let directory = base.appendingPathComponent("Meron", isDirectory: true)
        try? fileManager.createDirectory(at: directory, withIntermediateDirectories: true)
        return directory.path
    }
}
