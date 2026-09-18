#pragma once
/** @file http.hpp Native HTTP ownership, credentials, controls and outcomes. */
#include "@PACKAGE@/runtime.hpp"
#include <exception>
#include <functional>

namespace @NAMESPACE@ {
using Bytes = std::vector<std::uint8_t>;
using Headers = std::vector<std::pair<std::string, std::string>>;
struct LinkMetadata {
    std::string name;
    Source source;
    std::string target_kind;
    std::string target;
    JsonValue parameters;
    Presence<JsonValue> request_body;
    Presence<JsonValue> server;
};
struct ResponseMetadata {
    int status = 0;
    std::string content_type;
    Headers headers;
    std::string body_capture;
    bool truncated = false;
    std::vector<LinkMetadata> links;
};
struct HttpRequest {
    std::string method;
    std::string url;
    Headers headers;
    Presence<std::string> body;
};
struct HttpResponse { int status = 0; Headers headers; std::string body; };
struct TransportOptions {
    std::stop_token stop;
    std::chrono::steady_clock::time_point deadline;
    std::size_t max_response_bytes = @RESPONSE_BYTES@;
    std::size_t max_header_bytes = @HEADER_BYTES@;
    std::size_t max_capture_bytes = @CAPTURE_BYTES@;
    std::size_t max_buffer_bytes = @STREAM_BUFFER_BYTES@;
};
struct TransportError {
    enum class Kind { Configuration, Network, Tls, Cancelled, Timeout, ResourceLimit, Protocol };
    Kind kind = Kind::Network;
    std::string message;
    int native_code = 0;
    std::exception_ptr cause;
    Presence<ResponseMetadata> response;
};
struct SdkError {
    enum class Kind { Configuration, RequestValidation, RequestRepresentation, Transport,
        ResourceLimit, UnexpectedResponse, ResponseDecoding, Cancelled, Timeout };
    Kind kind = Kind::Configuration;
    Source operation_source;
    std::string operation_id;
    Source source;
    std::string message;
    Presence<ResponseMetadata> response;
    Presence<CodecError> codec;
    Presence<TransportError> transport;
    std::exception_ptr cause;
};

/** Pull ownership of a live response. Each chunk is bounded by the requested
 * receive window. Destruction or close() releases I/O without draining it. */
class ResponseBody {
public:
    virtual ~ResponseBody() = default;
    virtual Result<Presence<std::string>, TransportError> next() = 0;
    virtual void close() noexcept = 0; // Idempotent, including after EOF/failure.
    virtual Headers final_headers() const { return {}; }
};
struct HttpExchange {
    int status = 0;
    Headers headers;
    std::unique_ptr<ResponseBody> body;
    HttpExchange() = default;
    HttpExchange(int code, Headers fields, std::unique_ptr<ResponseBody> reader)
        : status(code), headers(std::move(fields)), body(std::move(reader)) {}
    HttpExchange(const HttpExchange&) = delete;
    HttpExchange& operator=(const HttpExchange&) = delete;
    HttpExchange(HttpExchange&&) noexcept = default;
    HttpExchange& operator=(HttpExchange&& other) noexcept {
        if (this != &other) { close(); status=other.status; headers=std::move(other.headers); body=std::move(other.body); }
        return *this;
    }
    ~HttpExchange() { close(); }
    void close() noexcept { if (body) body->close(); body.reset(); }
};
/** An adapter owns I/O until send returns or the returned response body closes.
 * Implementations honor deadlines/stop, streaming caps and one-attempt policy.
 * The default open() adapts a bounded legacy send(); libcurl overrides it. */
class Transport {
public:
    virtual ~Transport() = default;
    virtual Result<HttpResponse, TransportError> send(const HttpRequest&, const TransportOptions&) const = 0;
    virtual Result<HttpExchange, TransportError> open(const HttpRequest&, const TransportOptions&) const;
};

struct BasicCredentials {
    std::string username;
    std::string password;
    BasicCredentials(std::string user, std::string secret) : username(std::move(user)), password(std::move(secret)) {}
};
/** OAuth/OIDC attachment is chosen by the application, including auth scheme. */
struct Authorization {
    std::string scheme;
    std::string value;
    Authorization(std::string kind, std::string credential) : scheme(std::move(kind)), value(std::move(credential)) {}
};
/** Metadata and control for a caller-owned credential provider. No URL in this
 * record is fetched by the SDK; roles and scopes are not locally proven grants. */
struct CredentialRequest {
    Source operation_source;
    Source scheme_source;
    std::string operation_id;
    std::string scheme_name;
    std::vector<std::string> scopes;
    std::vector<std::string> roles;
    JsonValue metadata;
    std::stop_token stop;
    std::chrono::steady_clock::time_point deadline;
    /** Relative OAuth/OIDC metadata URLs use this expanded effective server,
     * not a logical $self/$id or an operation URL. No acquisition is performed. */
    std::string effective_server_url;
    std::string metadata_url_base = "effective-server";
};
using CredentialProvider = std::function<Result<Authorization, TransportError>(const CredentialRequest&)>;

struct ClientOptions {
    Presence<std::string> server_url;
    Presence<std::string> document_url;
    std::size_t server_index = 0;
    std::map<std::string, std::string, std::less<>> server_variables;
    Presence<std::size_t> security_alternative;
    std::chrono::milliseconds timeout{60'000};
    std::size_t max_request_bytes = @REQUEST_BYTES@;
    std::size_t max_response_bytes = @RESPONSE_BYTES@;
    std::size_t max_capture_bytes = @CAPTURE_BYTES@;
    std::size_t max_header_bytes = @HEADER_BYTES@;
    std::size_t max_part_bytes = @PART_BYTES@;
    std::size_t max_parts = @PARTS@;
    std::size_t max_stream_item_bytes = @STREAM_ITEM_BYTES@;
    std::size_t max_stream_buffer_bytes = @STREAM_BUFFER_BYTES@;
    /** ua/v1 attribution: full User-Agent override; an explicit empty value
     * suppresses the automatic attribution header entirely. */
    Presence<std::string> user_agent;
    /** ua/v1 attribution identity replacing the SDK token in the automatic
     * header: "<name>" or "<name>/<version>" of RFC 9110 tokens. */
    Presence<std::string> application_id;
};
struct CallOptions {
    std::stop_token stop;
    Presence<std::chrono::milliseconds> timeout;
    Presence<std::size_t> max_request_bytes, max_response_bytes, max_capture_bytes, max_header_bytes;
    Presence<std::size_t> max_part_bytes, max_parts, max_stream_item_bytes, max_stream_buffer_bytes;
    Presence<std::size_t> server_index;
    Presence<std::map<std::string, std::string, std::less<>>> server_variables;
    Presence<std::size_t> security_alternative;
    /** Optional exact response representation request; checked after matching. */
    Presence<std::string> response_media;
};
#if defined(@PACKAGE@_HAS_CURL)
struct CurlOptions {
    Presence<std::string> ca_bundle;
    Presence<std::string> ca_directory;
    std::chrono::milliseconds connect_timeout{10'000};
};
class CurlTransport final : public Transport {
    struct State;
    struct Transfer;
    std::shared_ptr<const State> state_;
    explicit CurlTransport(std::shared_ptr<const State> state) : state_(std::move(state)) {}
public:
    static Result<std::shared_ptr<CurlTransport>, TransportError> create(CurlOptions options = {});
    Result<HttpResponse, TransportError> send(const HttpRequest&, const TransportOptions&) const override;
    Result<HttpExchange, TransportError> open(const HttpRequest&, const TransportOptions&) const override;
};
#endif
} // namespace @NAMESPACE@
