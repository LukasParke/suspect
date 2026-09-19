import Foundation
import XCTest
import GeneratedSDK

private let ok = HTTPResponse(status: 200, headers: [HTTPHeader("Content-Type", "application/json")], body: Data(#"{"ok":true}"#.utf8))
private actor Recorder: HTTPTransport {
    var response: HTTPResponse
    var requests: [HTTPRequest] = []
    init(_ response: HTTPResponse = ok) { self.response = response }
    func set(_ response: HTTPResponse) { self.response = response }
    func send(_ request: HTTPRequest) async throws -> HTTPResponse { requests.append(request); return response }
    func last() throws -> HTTPRequest { try XCTUnwrap(requests.last) }
    func count() -> Int { requests.count }
}
private final class Closed: @unchecked Sendable {
    private let lock = NSLock()
    private var count = 0
    func close() { lock.lock(); count += 1; lock.unlock() }
    var calls: Int { lock.lock(); defer { lock.unlock() }; return count }
}
private actor Chunks {
    var chunks: [Data]
    var index = 0
    let waiting: Bool
    init(_ bytes: Data, split: Int, waiting: Bool) {
        var values: [Data] = []
        for i in stride(from: 0, to: bytes.count, by: split) { values.append(bytes.subdata(in: i..<min(i + split, bytes.count))) }
        chunks = values; self.waiting = waiting
    }
    func next() async throws -> Data? {
        if index < chunks.count { defer { index += 1 }; return chunks[index] }
        if waiting { try await Task.sleep(nanoseconds: 60_000_000_000) }
        return nil
    }
}
private struct ChunkTransport: HTTPTransport {
    let chunks: Chunks
    let closed: Closed
    let status: Int
    let contentType: String
    init(_ text: String, split: Int = 1, contentType: String = "text/event-stream", status: Int = 200, waiting: Bool = false, closed: Closed) {
        chunks = Chunks(Data(text.utf8), split: split, waiting: waiting); self.closed = closed; self.contentType = contentType; self.status = status
    }
    func send(_ request: HTTPRequest) async throws -> HTTPResponse { XCTFail("streamed operation eagerly buffered transport"); throw CancellationError() }
    func open(_ request: HTTPRequest, maxBufferedBytes: Int) async throws -> HTTPStreamResponse {
        XCTAssertGreaterThan(maxBufferedBytes, 0)
        let chunks = chunks; let closed = closed
        return HTTPStreamResponse(status: status, headers: [HTTPHeader("Content-Type", contentType)],
            body: HTTPByteStream(next: { try await chunks.next() }, cancel: { closed.close() }))
    }
}
private func header(_ request: HTTPRequest, _ name: String) -> String? { request.headers.first { $0.name.lowercased() == name.lowercased() }?.value }
private struct NativeTransport: HTTPTransport {
    func send(_ request: HTTPRequest) async throws -> HTTPResponse {
        do { return try await URLSessionTransport().send(request) }
        catch { print("SWIFT_PROTOCOL_NATIVE method=\(request.method) failure=\(error)"); throw error }
    }
    func open(_ request: HTTPRequest, maxBufferedBytes: Int) async throws -> HTTPStreamResponse {
        do { return try await URLSessionTransport().open(request, maxBufferedBytes: maxBufferedBytes) }
        catch { print("SWIFT_PROTOCOL_NATIVE path=\(request.url.path) open_failure=\(error)"); throw error }
    }
}

@MainActor
final class ProtocolTests: XCTestCase {
    func testFlatWireShapeKeysRetainExactUnicodeIdentity() async throws {
        let recorder = Recorder(); let client = Client(transport: recorder)
        _ = try await client.unicodeParams(UnicodeParamsInput(map: .init(e: false, value: 1)))
        let request = try await recorder.last()
        XCTAssertEqual(request.url.absoluteString, "https://example.test/api/unicode?map%5Be%CC%81%5D=false&map%5B%C3%A9%5D=1")
    }
    func testUnnamedSchemaFreeJSONTextAndMoreSpecificMediaParameters() async throws {
        let recorder = Recorder(); let client = Client(transport: recorder)
        let unnamed = try await client.getUnnamed(); XCTAssertTrue(unnamed.data.ok)
        let json = JsonValue.object(try JsonObject([("n", JsonValue.number(try JsonNumber("1.00e1000")))]))
        await recorder.set(HTTPResponse(status: 200, headers: [HTTPHeader("Content-Type", "APPLICATION/JSON")], body: try json.encoded()))
        let free = try await client.freeJSON(FreeJSONInput(body: json)); XCTAssertEqual(free.data, json)
        var request = try await recorder.last(); XCTAssertEqual(request.body, try json.encoded())
        await recorder.set(HTTPResponse(status: 200, headers: [HTTPHeader("Content-Type", "text/plain; charset=UTF-8")], body: Data("snow 雪".utf8)))
        let text = try await client.freeText(FreeTextInput(body: "snow 雪")); XCTAssertEqual(text.data, "snow 雪")
        request = try await recorder.last(); XCTAssertEqual(request.body, Data("snow 雪".utf8))
        let count = await recorder.count()
        do { _ = try await client.freeText(FreeTextInput(body: String(repeating: "x", count: 1_000_000))); XCTFail("schema-free text byte ceiling") } catch let error as SDKError { XCTAssertEqual(error.kind, .requestRepresentation); XCTAssertEqual(error.json?.kind, .resourceLimit) }
        let after = await recorder.count(); XCTAssertEqual(count, after)
        await recorder.set(HTTPResponse(status: 200, headers: [HTTPHeader("Content-Type", "application/json; profile=Exact; extra=yes")], body: Data(#"{"name":"specific"}"#.utf8)))
        guard case .json2(let specific) = try await client.parameterMedia().data else { return XCTFail("more declared matching parameters must win") }; XCTAssertEqual(specific.name, "specific")
        await recorder.set(ok)
        guard case .json(let broad) = try await client.parameterMedia().data else { return XCTFail("base JSON media should match") }; XCTAssertTrue(broad.ok)
    }
    func testAnonymousORANDAndCaseInsensitiveHTTPAttachments() async throws {
        let recorder = Recorder()
        let anonymous = Client(transport: recorder)
        let publicValue = try await anonymous.publicValue(); XCTAssertTrue(publicValue.data.ok)
        var request = try await recorder.last()
        XCTAssertNil(header(request, "Authorization"))
        _ = try await anonymous.optionalAuth()
        request = try await recorder.last(); XCTAssertNil(header(request, "Authorization"))
        let bearer = Client(credentials: Credentials(bearer: "token"), transport: recorder)
        _ = try await bearer.optionalAuth(); request = try await recorder.last(); XCTAssertEqual(header(request, "Authorization"), "Bearer token")
        _ = try await bearer.optionalAuth(options: RequestOptions(securityAlternative: 1))
        request = try await recorder.last(); XCTAssertNil(header(request, "Authorization"))
        let basic = Client(credentials: Credentials(basic: HTTPBasicCredential(username: "user", password: "p:ass")), transport: recorder)
        _ = try await basic.orAuth(); request = try await recorder.last(); XCTAssertEqual(header(request, "Authorization"), "Basic dXNlcjpwOmFzcw==")
        _ = try await bearer.orAuth(); request = try await recorder.last(); XCTAssertEqual(header(request, "Authorization"), "Bearer token")
        let and = Client(credentials: Credentials(cookieKey: "s+雪", headerKey: "header-token", queryKey: "q&+"), transport: recorder)
        _ = try await and.andAuth(); request = try await recorder.last()
        XCTAssertEqual(request.url.absoluteString, "https://example.test/api/and?key=q%26%2B")
        XCTAssertEqual(header(request, "X-Key"), "header-token")
        XCTAssertEqual(header(request, "Cookie"), "session=s%2B%E9%9B%AA")
        let count = await recorder.count()
        do { _ = try await anonymous.andAuth(); XCTFail("partial AND accepted") } catch let error as SDKError { XCTAssertEqual(error.kind, .requestRepresentation) }
        do { _ = try await basic.orAuth(options: RequestOptions(securityAlternative: 1)); XCTFail("missing selected credential") } catch let error as SDKError { XCTAssertEqual(error.kind, .requestRepresentation) }
        let after = await recorder.count(); XCTAssertEqual(count, after)
        guard case .alternatives(let alternatives) = OrAuthHTTP.metadata.security,
              case .roles(let roles) = alternatives[1][0].permissions else { return XCTFail("roles lost") }
        XCTAssertEqual(roles.map(\.value), ["admin"])
        guard case .disabled = PublicValueHTTP.metadata.security else { return XCTFail("disabled auth distinction lost") }
    }
    func testOAuthAndOIDCCallerHooksReceiveMetadataWithoutAcquisition() async throws {
        let recorder = Recorder(); let oauthCalls = Closed(); let oidcCalls = Closed()
        let credentials = Credentials(oauth: { context in
            oauthCalls.close(); XCTAssertEqual(context.name, "oauth")
            XCTAssertTrue(context.scheme.terminal.pointer.hasSuffix("/securitySchemes/oauth"))
            XCTAssertEqual(context.flows[0].kind, "authorizationCode")
            XCTAssertEqual(context.flows[0].authorizationURL?.value, "https://auth.example.test/authorize")
            XCTAssertEqual(context.flows[0].tokenURL?.value, "https://auth.example.test/token")
            XCTAssertEqual(context.metadataURL?.value, "https://auth.example.test/metadata")
            guard case .scopes(let scopes) = context.permissions else { throw CancellationError() }
            XCTAssertEqual(scopes.map(\.value), ["read"])
            return "Custom caller-selected-credential"
        }, oidc: { context in
            oidcCalls.close(); XCTAssertEqual(context.discoveryURL?.value, "https://auth.example.test/.well-known/openid-configuration")
            return "DPoP opaque-value"
        })
        let client = Client(credentials: credentials, transport: recorder)
        _ = try await client.oauthValue(); var request = try await recorder.last(); XCTAssertEqual(header(request, "Authorization"), "Custom caller-selected-credential")
        _ = try await client.oidcValue(); request = try await recorder.last(); XCTAssertEqual(header(request, "Authorization"), "DPoP opaque-value")
        XCTAssertEqual(oauthCalls.calls, 1); XCTAssertEqual(oidcCalls.calls, 1)
        let calls = await recorder.count(); XCTAssertEqual(calls, 2)
    }
    func testRelativeServersChoicesDefaultsVariablesAndPreflightFailures() async throws {
        let recorder = Recorder(); let client = Client(transport: recorder, options: ClientOptions(documentURL: "https://docs.example.test/spec/openapi.json"))
        _ = try await client.serverChoice(); var request = try await recorder.last(); XCTAssertEqual(request.url.absoluteString, "https://docs.example.test/v1/server")
        _ = try await client.serverChoice(options: RequestOptions(serverVariables: ["version": "v2"])); request = try await recorder.last(); XCTAssertEqual(request.url.absoluteString, "https://docs.example.test/v2/server")
        _ = try await client.serverChoice(options: RequestOptions(serverIndex: 1, serverVariables: ["region": "us", "version": "v3"])); request = try await recorder.last(); XCTAssertEqual(request.url.absoluteString, "https://us.example.test/v3/server")
        _ = try await client.serverChoice(options: RequestOptions(serverIndex: 2)); request = try await recorder.last(); XCTAssertEqual(request.url.absoluteString, "http://plain.example.test/v1/server")
        _ = try await client.defaultServer(); request = try await recorder.last(); XCTAssertEqual(request.url.absoluteString, "https://docs.example.test/default-server")
        let count = await recorder.count()
        for options in [RequestOptions(serverVariables: ["version": "bad"]), RequestOptions(serverVariables: ["unknown": "x"]), RequestOptions(serverIndex: 3), RequestOptions(serverIndex: 1, serverVariables: ["version": "../bad"]), RequestOptions(serverIndex: 1, serverVariables: ["version": "bad?query"])] {
            do { _ = try await client.serverChoice(options: options); XCTFail("invalid override accepted") } catch let error as SDKError { XCTAssertEqual(error.kind, .requestRepresentation) }
        }
        do { _ = try await Client(transport: recorder).defaultServer(); XCTFail("invented local-file origin") } catch let error as SDKError { XCTAssertEqual(error.kind, .requestRepresentation) }
        let after = await recorder.count(); XCTAssertEqual(count, after)
    }
    func testNormativePathQueryHeaderCookieContentAndReservedVectors() async throws {
        let recorder = Recorder(); let client = Client(transport: recorder)
        let color = Color(b: 150, g: 200, r: 100)
        _ = try await client.styles(StylesInput(label: ["blue", "black", "brown"], matrix: color, simple: "a/b",
            color: .value(color), multi: .value(["a,b", "雪"]), spaces: .value(["blue", "black"]), pipes: .value(["blue", "black"]),
            filter: .value(color), reserved: .value("a:/@?%26%2B%3D%23"), content: .value(Payload(name: "雪")),
            xFlag: .value(true), xTags: .value(["first", "second"]), sid: .value("a/b"), crumb: .value(["x", "y"])))
        var request = try await recorder.last()
        XCTAssertEqual(request.url.absoluteString, "https://example.test/api/styles/.blue.black.brown/;B=150;G=200;R=100/a%2Fb?color=B,150,G,200,R,100&multi=a%2Cb&multi=%E9%9B%AA&spaces=blue%20black&pipes=blue%7Cblack&filter%5BB%5D=150&filter%5BG%5D=200&filter%5BR%5D=100&reserved=a:/@?%26%2B%3D%23&content=%7B%22name%22%3A%22%E9%9B%AA%22%7D")
        XCTAssertEqual(header(request, "X-Flag"), "true"); XCTAssertEqual(header(request, "X-Tags"), "first,second")
        XCTAssertEqual(header(request, "Cookie"), "sid=a%2Fb; crumb=x; crumb=y")
        _ = try await client.scalarStyles(ScalarStylesInput(label: 12, matrix: "", simple: color))
        request = try await recorder.last(); XCTAssertEqual(request.url.absoluteString, "https://example.test/api/scalar/.12/;matrix/B=150,G=200,R=100")
        let count = await recorder.count()
        for input in [StylesInput(label: [], matrix: color, simple: "x"), StylesInput(label: ["x"], matrix: color, simple: "."),
            StylesInput(label: ["x"], matrix: color, simple: "x", spaces: .value(["a b"])),
            StylesInput(label: ["x"], matrix: color, simple: "x", pipes: .value(["a|b"])),
            StylesInput(label: ["x"], matrix: color, simple: "x", reserved: .value("a&b")),
            StylesInput(label: ["x"], matrix: color, simple: "x", xTags: .value(["one,two"])),
            StylesInput(label: ["x"], matrix: color, simple: "x", crumb: .value(["white space"]))] {
            do { _ = try await client.styles(input); XCTFail("ambiguous wire value accepted") } catch let error as SDKError { XCTAssertEqual(error.kind, .requestRepresentation) }
        }
        do {
            _ = try await client.scalarStyles(ScalarStylesInput(label: 1, matrix: "x", simple: color, flat: .value(.init(additionalProperties: try JsonObject([("nested", JsonValue.array([]))])))))
            XCTFail("nested extra style value accepted")
        } catch let error as SDKError { XCTAssertEqual(error.kind, .requestRepresentation) }
        let after = await recorder.count(); XCTAssertEqual(count, after)
    }
    func testExactRangeDefaultActualStatusMediaParametersAndNoSniffing() async throws {
        let recorder = Recorder(); let client = Client(transport: recorder)
        let selected = try await client.chooseResponse()
        guard case .status200(let response) = selected, case .json(let value) = response.data else { return XCTFail("exact JSON selection") }
        XCTAssertTrue(value.ok); XCTAssertEqual(response.declaredContentType, "Application/JSON")
        await recorder.set(HTTPResponse(status: 200, headers: [HTTPHeader("Content-Type", "Text/Plain; profile=Exact; charset=UTF-8")], body: Data("1.00e2".utf8)))
        guard case .status200(let textResponse) = try await client.chooseResponse(), case .text(let number) = textResponse.data else { return XCTFail("parameter media selection") }
        XCTAssertEqual(number.raw, "1.00e2")
        for (media, declaration) in [("application/octet-stream", "application/octet-stream"), ("image/png", "image/*"), ("audio/wav", "*/*")] {
            let binary = Data([0, 255, 128, 10]); await recorder.set(HTTPResponse(status: 200, headers: [HTTPHeader("Content-Type", media)], body: binary))
            guard case .status200(let response) = try await client.chooseResponse() else { return XCTFail("exact bytes selection") }
            XCTAssertEqual(response.declaredContentType, declaration); XCTAssertEqual(response.rawBody, binary)
        }
        await recorder.set(HTTPResponse(status: 201, headers: [HTTPHeader("Content-Type", "text/plain")], body: Data("range".utf8)))
        guard case .status2XX(let ranged) = try await client.chooseResponse(), case .text(let text) = ranged.data else { return XCTFail("range selection") }
        XCTAssertEqual(ranged.status, 201); XCTAssertEqual(text, "range")
        await recorder.set(HTTPResponse(status: 204, headers: [], body: Data("must not decode".utf8)))
        guard case .status2XX(let empty) = try await client.chooseResponse(), case .none = empty.data else { return XCTFail("range 204 suppression") }
        XCTAssertEqual(empty.rawBody, Data())
        await recorder.set(HTTPResponse(status: 202, headers: [HTTPHeader("Content-Type", "text/plain")], body: Data("success".utf8)))
        let fallback = try await client.defaultResponse(); XCTAssertEqual(fallback.status, 202)
        guard case .text(let fallbackText) = fallback.data else { return XCTFail("default success") }; XCTAssertEqual(fallbackText, "success")
        await recorder.set(HTTPResponse(status: 499, headers: [HTTPHeader("Content-Type", "application/problem+json")], body: Data(#"{"name":"failure"}"#.utf8)))
        do { _ = try await client.chooseResponse(); XCTFail("actual default failure returned success") }
        catch ChooseResponseAPIError.defaultResponse(let error) { XCTAssertEqual(error.status, 499); guard case .json(let data) = error.data else { return XCTFail() }; XCTAssertEqual(data.name, "failure") }
        for bad in [HTTPResponse(status: 201, headers: [HTTPHeader("Content-Type", "application/problem+json")], body: Data(#"{"name":"must not fall through"}"#.utf8)),
                    HTTPResponse(status: 200, headers: [], body: Data(#"{"ok":true}"#.utf8)),
                    HTTPResponse(status: 200, headers: [HTTPHeader("Content-Type", "text/plain; a=x; A=y")], body: Data()),
                    HTTPResponse(status: 201, headers: [HTTPHeader("Content-Type", "text/plain; charset=iso-8859-1")], body: Data())] {
            await recorder.set(bad)
            do { _ = try await client.chooseResponse(); XCTFail("invalid status/media was sniffed or fell through") } catch let error as SDKError { XCTAssertEqual(error.kind, .responseDecoding) }
        }
    }
    func testUndeclaredBoundedBytesAndRequiredTypedHeadersAndLinkMetadata() async throws {
        let bytes = Data([0, 255, 128]); let recorder = Recorder(HTTPResponse(status: 200, headers: [], body: bytes)); let client = Client(transport: recorder)
        guard case .status200(let response) = try await client.unspecified() else { return XCTFail() }; XCTAssertEqual(response.data, bytes)
        await recorder.set(HTTPResponse(status: 418, headers: [], body: Data(repeating: 255, count: 100)))
        do { _ = try await client.unspecified(); XCTFail() } catch UnspecifiedAPIError.status418(let response) { XCTAssertEqual(response.data.count, 100); XCTAssertEqual(response.rawBody.count, 64); XCTAssertTrue(response.rawBodyTruncated) }
        await recorder.set(HTTPResponse(status: 200, headers: [], body: Data(repeating: 255, count: 100)))
        do { _ = try await client.unspecified(options: RequestOptions(maxResponseBytes: 10)); XCTFail() } catch let error as SDKError { XCTAssertEqual(error.kind, .responseTooLarge) }
        await recorder.set(HTTPResponse(status: 200, headers: [HTTPHeader("Content-Type", "application/json"), HTTPHeader("x-count", "12"), HTTPHeader("X-Flags", "true,false"), HTTPHeader("X-Color", "B=150,G=200,R=100"), HTTPHeader("X-JSON", #"{"name":"header"}"#)], body: ok.body))
        let metadata = try await client.readMetadata()
        XCTAssertEqual(metadata.typedHeaders.xCount.raw, "12"); XCTAssertEqual(metadata.typedHeaders.xFlags, .value([true, false]))
        XCTAssertEqual(metadata.typedHeaders.xColor, .value(Color(b: 150, g: 200, r: 100)))
        XCTAssertEqual(metadata.typedHeaders.xJSON, .value(Payload(name: "header")))
        let link = try XCTUnwrap(metadata.links.first); XCTAssertEqual(link.name, "next")
        XCTAssertTrue(link.source.useSite.pointer.hasSuffix("/responses/200/links/next")); XCTAssertEqual(link.source.terminal.pointer, "/components/links/Next")
        XCTAssertEqual(link.parameters["value"]?.value, .string("$response.header.X-Count"))
        guard case .object(let literal) = link.parameters["literal"]?.value else { return XCTFail("link literal not preserved") }; XCTAssertEqual(literal["$ref"], .string("instance"))
        XCTAssertEqual(link.server?.template, "../v2")
        for fields in [[], [HTTPHeader("X-Count", "-1")], [HTTPHeader("X-Count", "x")], [HTTPHeader("X-Count", "1"), HTTPHeader("X-Count", "2")]] {
            await recorder.set(HTTPResponse(status: 200, headers: [HTTPHeader("Content-Type", "application/json")] + fields, body: ok.body))
            do { _ = try await client.readMetadata(); XCTFail("required/ambiguous typed header accepted") } catch let error as SDKError {
                XCTAssertEqual(error.kind, .responseDecoding); XCTAssertEqual(error.status, 200)
                XCTAssertTrue(error.source.pointer.contains("X-Count") || error.source.pointer.contains("/components/headers/Quota"))
            }
        }
    }
    func testTypedRequestMediaCannotBypassSpecificSchemas() async throws {
        let recorder = Recorder(); let client = Client(transport: recorder)
        _ = try await client.sendBody(SendBodyInput(body: .json(Payload(name: "direct", amount: .value(try JsonNumber("1.00e3"))))))
        var request = try await recorder.last(); XCTAssertEqual(header(request, "Content-Type"), "application/json"); XCTAssertEqual(String(decoding: request.body ?? Data(), as: UTF8.self), #"{"amount":1.00e3,"name":"direct"}"#)
        _ = try await client.sendBody(SendBodyInput(body: .text(try JsonInteger("1.00e3")))); request = try await recorder.last(); XCTAssertEqual(request.body, Data("1.00e3".utf8))
        let binary = Data([0, 255, 128]); _ = try await client.sendBody(SendBodyInput(body: .bytes(binary, contentType: "application/octet-stream"))); request = try await recorder.last(); XCTAssertEqual(request.body, binary)
        let count = await recorder.count()
        do { _ = try await client.sendBody(SendBodyInput(body: .bytes(Data("invalid JSON".utf8), contentType: "application/json"))); XCTFail("wildcard bypass") } catch let error as SDKError { XCTAssertEqual(error.kind, .requestRepresentation) }
        do { _ = try await client.sendBody(SendBodyInput(body: .json(Payload(name: "")))); XCTFail("native mutation/schema bypass") } catch let error as SDKError { XCTAssertEqual(error.kind, .requestValidation) }
        let after = await recorder.count(); XCTAssertEqual(count, after)
    }
    func testFormsMultipartBinaryRepeatedPartsTypedPartHeadersAndStructuralRules() async throws {
        let recorder = Recorder(); let client = Client(transport: recorder)
        _ = try await client.submitForm(SubmitFormInput(body: SubmitFormBody(title: "a +雪", payload: .value(Payload(name: "p")), values: .value([1, 2]))))
        var request = try await recorder.last()
        XCTAssertEqual(header(request, "Content-Type"), "application/x-www-form-urlencoded")
        XCTAssertEqual(String(decoding: request.body ?? Data(), as: UTF8.self), "payload=%7B%22name%22%3A%22p%22%7D&title=a+%2B%E9%9B%AA&values=1&values=2")
        let file = Data([0, 255, 128, 10])
        let body = UploadMultipartBody(file: UploadMultipartFilePart(value: file, headers: UploadMultipartFileHeaders(xSize: 4), filename: "a.bin"), title: HTTPPart("title"), files: .value([HTTPPart(Data([1, 0]), filename: "one.bin"), HTTPPart(Data([2, 255]), filename: "two.bin")]), payload: .value(HTTPPart(Payload(name: "p"))), tag: .value([HTTPPart(1), HTTPPart(2)]))
        _ = try await client.upload(UploadInput(body: body), options: RequestOptions(multipartBoundary: "witness-boundary"))
        request = try await recorder.last(); XCTAssertEqual(header(request, "Content-Type"), "multipart/form-data; boundary=witness-boundary")
        let raw = try XCTUnwrap(request.body)
        var expected = Data("--witness-boundary\r\nX-Size: 4\r\nContent-Disposition: form-data; name=\"file\"; filename=\"a.bin\"\r\nContent-Type: application/octet-stream\r\n\r\n".utf8)
        expected.append(file)
        expected.append(Data("\r\n--witness-boundary\r\nContent-Disposition: form-data; name=\"files\"; filename=\"one.bin\"\r\nContent-Type: application/octet-stream\r\n\r\n".utf8)); expected.append(Data([1, 0]))
        expected.append(Data("\r\n--witness-boundary\r\nContent-Disposition: form-data; name=\"files\"; filename=\"two.bin\"\r\nContent-Type: application/octet-stream\r\n\r\n".utf8)); expected.append(Data([2, 255]))
        // OAS 3.2 Appendix E.3: the name is in Content-Disposition; the body
        // contains only the style value, without URI encoding or `name=`.
        expected.append(Data("\r\n--witness-boundary\r\nContent-Disposition: form-data; name=\"payload\"\r\nContent-Type: application/json\r\n\r\n{\"name\":\"p\"}\r\n--witness-boundary\r\nContent-Disposition: form-data; name=\"tag\"\r\n\r\n1\r\n--witness-boundary\r\nContent-Disposition: form-data; name=\"tag\"\r\n\r\n2\r\n--witness-boundary\r\nContent-Disposition: form-data; name=\"title\"\r\nContent-Type: text/plain\r\n\r\ntitle\r\n--witness-boundary--\r\n".utf8))
        XCTAssertEqual(raw, expected, "binary and part framing must be byte-for-byte correct")
        var withCommentary = Data("ignored preamble\r\n".utf8); withCommentary.append(expected); withCommentary.append(Data("ignored epilogue".utf8))
        await recorder.set(HTTPResponse(status: 200, headers: [HTTPHeader("Content-Type", "multipart/form-data; boundary=witness-boundary")], body: withCommentary))
        let parts = try await client.downloadParts(); XCTAssertEqual(parts.data.file.value, file); XCTAssertEqual(parts.data.file.headers.xSize.raw, "4")
        guard case .value(let files) = parts.data.files, case .value(let tags) = parts.data.tag else { return XCTFail("repeated part state lost") }
        XCTAssertEqual(files.map(\.value), [Data([1, 0]), Data([2, 255])]); XCTAssertEqual(tags.map(\.value), [1, 2])
        await recorder.set(HTTPResponse(status: 200, headers: [HTTPHeader("Content-Type", "application/x-www-form-urlencoded")], body: Data("title=decoded+title&values=1&values=2&payload=%7B%22name%22%3A%22p%22%7D".utf8)))
        let form = try await client.readForm(); XCTAssertEqual(form.data.title, "decoded title"); XCTAssertEqual(form.data.values, .value([1, 2]))
        await recorder.set(ok)
        var invalid = body; invalid.file.value = Data(repeating: 255, count: 33)
        do { _ = try await client.upload(UploadInput(body: invalid)); XCTFail("part byte ceiling") } catch let error as SDKError { XCTAssertEqual(error.kind, .requestRepresentation) }
        invalid = body; invalid.tag = .value([HTTPPart(0)])
        do { _ = try await client.upload(UploadInput(body: invalid)); XCTFail("per-item codec") } catch let error as SDKError { XCTAssertEqual(error.kind, .requestValidation) }
        invalid = body; invalid.files = .value([HTTPPart(Data()), HTTPPart(Data()), HTTPPart(Data())])
        do { _ = try await client.upload(UploadInput(body: invalid)); XCTFail("repeated part cardinality") } catch let error as SDKError { XCTAssertEqual(error.kind, .requestRepresentation) }
        do { _ = try await client.extraParts(ExtraPartsInput(body: ExtraPartsMultipartBody(title: HTTPPart("x")))); XCTFail("aggregate minProperties") } catch let error as SDKError { XCTAssertEqual(error.kind, .requestRepresentation) }
        _ = try await client.extraParts(ExtraPartsInput(body: ExtraPartsMultipartBody(title: HTTPPart("x"), additionalProperties: try JsonObject([("score", HTTPPart(2))]))))
    }
    func testSSEEnvelopesArbitraryChunkUTF8CRLFCommentsRetryIDAndNoSentinelInference() async throws {
        let text = "\u{FEFF}: comment\r\nid: one\r\nevent: update\r\nretry: 0015\r\ndata: snow 雪\r\ndata: {\"n\":1}\r\nunknown: ignored\r\n\r\n: heartbeat\n\nid: bad\0id\nretry: invalid\ndata: [DONE]\n\ndata: incomplete"
        for split in [1, 2, 3, 5, 13, 64, 512] {
            let closed = Closed(); let client = Client(transport: ChunkTransport(text, split: split, closed: closed))
            let result = try await client.events(); var events: [Event] = []
            for try await event in result.data { events.append(event) }
            XCTAssertEqual(events.count, 2); XCTAssertEqual(events[0].data, "snow 雪\n{\"n\":1}")
            XCTAssertEqual(events[0].event, .value("update")); XCTAssertEqual(events[0].retry, .value(15)); XCTAssertEqual(events[0].id, .value("one"))
            XCTAssertEqual(events[1].data, "[DONE]"); XCTAssertEqual(events[1].id, .value("one")); XCTAssertEqual(events[1].retry, .missing)
            XCTAssertEqual(closed.calls, 1)
        }
    }
    func testJSONLinesFramingFinalLineCodecFailuresAndFiniteLimits() async throws {
        for (text, valid) in [("{\"n\":1}\r\n{\"n\":2}", true), ("{\"n\":\"wrong\"}\n", false), ("[DONE]\n", false), ("\n", false), ("\u{001E}{\"n\":1}\n", false)] {
            let closed = Closed(); let client = Client(transport: ChunkTransport(text, contentType: "application/x-ndjson", closed: closed))
            do { let result = try await client.lines(); var values: [JsonInteger] = []; for try await line in result.data { values.append(line.n) }; XCTAssertTrue(valid); XCTAssertEqual(values, [1, 2]) }
            catch let error as SDKError { XCTAssertFalse(valid); XCTAssertEqual(error.kind, .responseDecoding); XCTAssertFalse(error.body.isEmpty); XCTAssertLessThanOrEqual(error.body.count, 64) }
            XCTAssertEqual(closed.calls, 1)
        }
        let jsonlClosed = Closed(); let jsonl = try await Client(transport: ChunkTransport("{\"n\":3}\n", contentType: "application/jsonl", closed: jsonlClosed)).jsonl()
        for try await line in jsonl.data { XCTAssertEqual(line.n, 3) }; XCTAssertEqual(jsonlClosed.calls, 1)
        for text in ["data: " + String(repeating: "x", count: 256) + "\n\n", "data:\n\n"] {
            let closed = Closed(); let result = try await Client(transport: ChunkTransport(text, closed: closed)).events()
            do { for try await _ in result.data {}; XCTFail("stream item limit/codec failure") } catch let error as SDKError { XCTAssertEqual(error.kind, .responseDecoding) }
            XCTAssertEqual(closed.calls, 1)
        }
        let closed = Closed(); let result = try await Client(transport: ChunkTransport(String(repeating: "data: x\n\n", count: 100), split: 1, closed: closed)).events(options: RequestOptions(maxResponseBytes: 40))
        do { for try await _ in result.data {}; XCTFail("total stream ceiling") } catch let error as SDKError { XCTAssertEqual(error.kind, .responseTooLarge) }
        XCTAssertEqual(closed.calls, 1)
    }
    func testEarlyBreakAndTaskCancellationCloseWhileSequenceRemainsAlive() async throws {
        let closed = Closed(); let result = try await Client(transport: ChunkTransport("data: first\n\ndata: second\n\n", waiting: true, closed: closed)).events()
        for try await event in result.data { XCTAssertEqual(event.data, "first"); break }
        XCTAssertEqual(closed.calls, 1, "early break must close even while result is retained")
        result.data.close(); XCTAssertEqual(closed.calls, 1)
        let cancelled = Closed(); let pending = try await Client(transport: ChunkTransport("", waiting: true, closed: cancelled)).events()
        let task = Task { for try await _ in pending.data {} }
        try await Task.sleep(nanoseconds: 20_000_000); task.cancel()
        do { try await task.value; XCTFail("cancelled iterator returned success") } catch is CancellationError {}
        XCTAssertEqual(cancelled.calls, 1)
        let mixedClosed = Closed(); let mixed = try await Client(transport: ChunkTransport(#"{"ok":true}"#, contentType: "application/json", closed: mixedClosed)).mixedStream()
        guard case .json(let reply) = mixed.data else { return XCTFail("mixed nonstream media") }; XCTAssertTrue(reply.ok); XCTAssertEqual(mixedClosed.calls, 1)
    }
    func testURLSessionAllStandardMethodsHEADAndSocketStreamCleanup() async throws {
        let base = try XCTUnwrap(ProcessInfo.processInfo.environment["SUSPECT_SWIFT_PROTOCOL_BASE"])
        let root = try XCTUnwrap(ProcessInfo.processInfo.environment["SUSPECT_SWIFT_PROTOCOL_MARKERS"])
        let client = Client(transport: NativeTransport(), options: ClientOptions(serverURL: base + "/api", timeout: 3))
        _ = try await client.getMethod(); _ = try await client.putMethod(); _ = try await client.postMethod()
        _ = try await client.deleteMethod(); _ = try await client.optionsMethod(); _ = try await client.patchMethod()
        _ = try await client.traceMethod(); _ = try await client.queryMethod()
        let head = try await client.headMethod(); XCTAssertEqual(head.data, HTTPNoContent()); XCTAssertEqual(head.typedHeaders.xCount.raw, "7"); XCTAssertEqual(head.rawBody, Data())
        let complete = try await Client(transport: NativeTransport(), options: ClientOptions(serverURL: base + "/complete", timeout: 3)).socketEvents()
        var values: [String] = []; for try await event in complete.data { values.append(event.data) }; XCTAssertEqual(values, ["one", "two"])
        let breaking = try await Client(transport: NativeTransport(), options: ClientOptions(serverURL: base + "/break", timeout: 3)).socketEvents()
        for try await event in breaking.data { XCTAssertEqual(event.data, "first"); break }
        let cancelling = try await Client(transport: NativeTransport(), options: ClientOptions(serverURL: base + "/cancel", timeout: 3)).socketEvents()
        let task = Task { for try await _ in cancelling.data {} }
        try await Task.sleep(nanoseconds: 20_000_000); task.cancel()
        do { try await task.value; XCTFail("socket cancellation completed normally") } catch is CancellationError {}
        for name in ["stream-break", "stream-cancel", "stream-complete"] {
            let path = root + "/wire-" + name
            for _ in 0..<100 where !FileManager.default.fileExists(atPath: path) { try await Task.sleep(nanoseconds: 10_000_000) }
            XCTAssertTrue(FileManager.default.fileExists(atPath: path), "URLSession transfer leaked: \(name)")
        }
    }
    func testURLSessionStreamingBufferTotalTimeoutAndErrorCleanup() async throws {
        let base = try XCTUnwrap(ProcessInfo.processInfo.environment["SUSPECT_SWIFT_PROTOCOL_BASE"])
        let root = try XCTUnwrap(ProcessInfo.processInfo.environment["SUSPECT_SWIFT_PROTOCOL_MARKERS"])
        let recorder = Recorder(HTTPResponse(status: 200, headers: [HTTPHeader("Content-Type", "text/event-stream")], body: Data()))
        _ = try await Client(transport: recorder, options: ClientOptions(serverURL: base + "/buffer")).socketEvents()
        let request = try await recorder.last()
        let buffered = try await URLSessionTransport().open(request, maxBufferedBytes: 8)
        try await Task.sleep(nanoseconds: 50_000_000)
        do { for try await _ in buffered.body {}; XCTFail("queued bytes escaped the finite buffer") }
        catch TransportError.streamBufferExceeded(let limit) { XCTAssertEqual(limit, 8) }
        buffered.close()
        do {
            let total = try await Client(options: ClientOptions(serverURL: base + "/limit", timeout: 3)).socketEvents(options: RequestOptions(maxResponseBytes: 40))
            for try await _ in total.data {}
            XCTFail("real streaming transport exceeded total bytes")
        } catch let error as SDKError { XCTAssertEqual(error.kind, .responseTooLarge) }
        do {
            let timed = try await Client(options: ClientOptions(serverURL: base + "/timeout", timeout: 0.1)).socketEvents()
            for try await _ in timed.data {}
            XCTFail("idle stream ignored whole-transfer timeout")
        } catch let error as SDKError { XCTAssertEqual(error.kind, .timeout) }
        for name in ["buffer", "limit", "timeout"] {
            let path = root + "/wire-stream-" + name
            for _ in 0..<100 where !FileManager.default.fileExists(atPath: path) { try await Task.sleep(nanoseconds: 10_000_000) }
            XCTAssertTrue(FileManager.default.fileExists(atPath: path), "stream resource error did not release socket: \(name)")
        }
    }
}
