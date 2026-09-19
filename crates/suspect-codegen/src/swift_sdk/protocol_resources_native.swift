import Foundation
import XCTest
import GeneratedSDK

@MainActor
final class PhysicalServerTests: XCTestCase {
    private var base: String { ProcessInfo.processInfo.environment["SUSPECT_SWIFT_RESOURCE_BASE"]! }

    func testLogicalMetadataKeepsPhysicalServerOwnership() throws {
        let root = RootServerHTTP.metadata
        XCTAssertEqual(root.source.useSite.document, base + "/specs/releases/api.json")
        XCTAssertEqual(root.source.useSiteResource?.canonicalURI, "https://logical.swift.test/api.json#revision")
        XCTAssertEqual(root.servers[0].documentBase.document, base + "/specs/releases/api.json")
        XCTAssertEqual(root.servers[0].urlBase, .serverDocument)
        XCTAssertEqual(try root.servers[0].resolve().absoluteString, base + "/specs/service/v1")
        let mounted = MountedServerHTTP.metadata
        XCTAssertEqual(mounted.source.useSite.document, base + "/specs/releases/api.json")
        XCTAssertEqual(mounted.source.terminal.document, base + "/artifacts/nested/parts.json")
        XCTAssertEqual(mounted.source.terminalResource?.baseURI, "https://logical.swift.test/parts.json")
        XCTAssertEqual(mounted.source.references.count, mounted.source.referenceResources.count)
        XCTAssertFalse(mounted.source.references.isEmpty)
        XCTAssertEqual(mounted.servers[0].documentBase.document, base + "/artifacts/nested/parts.json")
        XCTAssertEqual(try mounted.servers[0].resolve().absoluteString, base + "/artifacts/owned/v1")
        XCTAssertThrowsError(try mounted.servers[0].resolve(variables: ["version":"undeclared"]))
        XCTAssertThrowsError(try mounted.servers[0].resolve(variables: ["extra":"v2"]))
    }

    func testDefaultClientAndExplicitDocumentAndServerOverridesReachExactWirePaths() async throws {
        let client = Client()
        _ = try await client.rootServer()
        _ = try await client.rootServer(options: .init(serverVariables: ["version":"v2"]))
        _ = try await client.rootServer(options: .init(documentURL: base + "/override/dir/api.json"))
        _ = try await client.rootServer(options: .init(serverURL: base + "/custom"))
        _ = try await client.mountedServer()
        _ = try await client.mountedServer(options: .init(serverVariables: ["version":"v2"]))
        _ = try await client.defaultServer()
        _ = try await client.encodedServer()
    }

    func testGenericURIResolutionPreservesEncodedDataAndRejectsRepair() throws {
        XCTAssertEqual(try AbsoluteServerHTTP.metadata.servers[0].resolve(documentURL: "file:///ignored.json").absoluteString, "https://example.test/API%2Fv1")
        XCTAssertEqual(try NetworkServerHTTP.metadata.servers[0].resolve().absoluteString, "http://other.test/API")
        XCTAssertEqual(try EncodedServerHTTP.metadata.servers[0].resolve().absoluteString, base + "/specs/releases/%2e%2e/Api%2Fv1")
        let variable = VariableServerHTTP.metadata.servers[0]
        for value in ["雪", "raw space", "bad%2", "?query", "#fragment"] { XCTAssertThrowsError(try variable.resolve(variables: ["segment":value])) }
        for document in ["file:///not-http.json", base + "/raw space.json", base + "/api.json#fragment", base + "/bad%2"] { XCTAssertThrowsError(try variable.resolve(documentURL: document)) }
        XCTAssertEqual(try variable.resolve(variables: ["segment":"a//b/../c%2Fd"]).absoluteString, base + "/specs/releases/a//c%2Fd")
    }

    func testFileSourcesNeedExplicitHTTPDocumentBase() async throws {
        let server = LocalServerHTTP.metadata.servers[0]
        XCTAssertEqual(server.documentBase.document, "file:///SwiftProvided/local.json")
        XCTAssertEqual(server.source?.terminalResource?.baseURI, "https://logical.swift.test/local.json")
        XCTAssertThrowsError(try server.resolve())
        XCTAssertEqual(try server.resolve(documentURL: base + "/explicit/specs/api.json").absoluteString, base + "/explicit/specs/local")
        do { _ = try await Client().localServer(); XCTFail("file source acquired an invented origin") }
        catch let error as SDKError { XCTAssertEqual(error.kind, .requestRepresentation) }
        _ = try await Client().localServer(options: .init(documentURL: base + "/explicit/specs/api.json"))
    }

    func testOAuthAndOIDCMetadataUseTheEffectiveServer() async throws {
        let expected = base + "/specs/service/v1"
        let credentials = Credentials(oauth: { context in
            XCTAssertEqual(context.urlBase, .effectiveServer)
            XCTAssertEqual(context.serverURL?.absoluteString, expected)
            XCTAssertEqual(context.flows[0].urlBase, .effectiveServer)
            XCTAssertEqual(context.flows[0].tokenURL?.value, "./token")
            XCTAssertEqual(context.metadataURL?.value, "../metadata")
            XCTAssertEqual(context.scheme.terminalResource?.baseURI, "https://logical.swift.test/api.json")
            XCTAssertEqual(URL(string: context.flows[0].tokenURL!.value, relativeTo: context.serverURL!)?.absoluteURL.absoluteString, expected.replacingOccurrences(of: "/v1", with: "/token"))
            return "Caller oauth"
        }, oidc: { context in
            XCTAssertEqual(context.urlBase, .effectiveServer)
            XCTAssertEqual(context.serverURL?.absoluteString, expected)
            XCTAssertEqual(context.discoveryURL?.value, "./openid")
            return "Caller oidc"
        })
        let client = Client(credentials: credentials)
        _ = try await client.oauthServer()
        _ = try await client.oidcServer()
    }

    func testLinkServersUseTheirOwnPhysicalDocumentAndRemainMetadata() async throws {
        let response = try await Client().linkServer()
        let link = try XCTUnwrap(response.links.first)
        XCTAssertEqual(link.source.terminal.document, base + "/artifacts/nested/links.json")
        XCTAssertEqual(link.source.terminalResource?.baseURI, "https://logical.swift.test/links.json")
        let server = try XCTUnwrap(link.server)
        XCTAssertEqual(server.documentBase.document, base + "/artifacts/nested/links.json")
        XCTAssertEqual(try server.resolve().absoluteString, base + "/artifacts/linked")
    }
}
