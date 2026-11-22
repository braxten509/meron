// Checks how the iOS host recognizes an authorization
// callback. The callback must return to the expected
// login flow with the information needed to complete
// it.

import Foundation
@testable import Meron
import MeronUI
import XCTest

final class OAuthCallbackContractTests: XCTestCase {
    func testInfoPlistRegistersSharedOAuthRedirectUriScheme() throws {
        let redirectUri = try XCTUnwrap(URL(string: OAuthFlowKt.defaultOAuthRedirectUri()))
        let urlTypes = try XCTUnwrap(Bundle.main.object(forInfoDictionaryKey: "CFBundleURLTypes") as? [[String: Any]])
        let schemes = urlTypes
            .compactMap { $0["CFBundleURLSchemes"] as? [String] }
            .flatMap { $0 }

        XCTAssertEqual(redirectUri.scheme, "jp.nonbili.meron.oauth")
        XCTAssertEqual(redirectUri.host, "oauth")
        XCTAssertTrue(schemes.contains("jp.nonbili.meron.oauth"))
    }

    func testSourceInfoPlistExposesProviderOAuthBuildSettings() throws {
        let testFile = URL(fileURLWithPath: #filePath)
        let infoPlist = testFile
            .deletingLastPathComponent()
            .deletingLastPathComponent()
            .appendingPathComponent("Meron")
            .appendingPathComponent("Info.plist")
        let data = try Data(contentsOf: infoPlist)
        let plist = try XCTUnwrap(
            PropertyListSerialization.propertyList(from: data, options: [], format: nil) as? [String: Any]
        )

        XCTAssertEqual(plist["MERON_GOOGLE_CLIENT_ID"] as? String, "$(MERON_GOOGLE_CLIENT_ID)")
        XCTAssertEqual(plist["MERON_GOOGLE_CLIENT_SECRET"] as? String, "$(MERON_GOOGLE_CLIENT_SECRET)")
        XCTAssertEqual(plist["MERON_GOOGLE_REDIRECT_URI"] as? String, "$(MERON_GOOGLE_REDIRECT_URI)")
        XCTAssertEqual(plist["MERON_OUTLOOK_CLIENT_ID"] as? String, "$(MERON_OUTLOOK_CLIENT_ID)")
        XCTAssertEqual(plist["MERON_OUTLOOK_CLIENT_SECRET"] as? String, "$(MERON_OUTLOOK_CLIENT_SECRET)")
        XCTAssertEqual(plist["MERON_OUTLOOK_REDIRECT_URI"] as? String, "$(MERON_OUTLOOK_REDIRECT_URI)")
    }

    func testEntitlementsDeclarePlaceholderAssociatedDomain() throws {
        let testFile = URL(fileURLWithPath: #filePath)
        let entitlements = testFile
            .deletingLastPathComponent()
            .deletingLastPathComponent()
            .appendingPathComponent("Meron")
            .appendingPathComponent("Meron.entitlements")
        let data = try Data(contentsOf: entitlements)
        let plist = try XCTUnwrap(
            PropertyListSerialization.propertyList(from: data, options: [], format: nil) as? [String: Any]
        )
        let domains = try XCTUnwrap(plist["com.apple.developer.associated-domains"] as? [String])

        XCTAssertTrue(domains.contains("applinks:$(MERON_ASSOCIATED_DOMAIN)"))
    }
}
