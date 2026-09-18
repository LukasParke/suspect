#pragma once
/** @file protocol.hpp Generated protocol descriptors and bounded implementation seams. */
#include "@PACKAGE@/http.hpp"
#include "@PACKAGE@/attribution.hpp"

namespace @NAMESPACE@::detail {
struct HttpFailure { SdkError error; };
[[noreturn]] void http_fail(SdkError::Kind kind, const Source& source, std::string message);
enum class Location { Path, Query, Querystring, Header, Cookie };
enum class Style { Simple, Label, Matrix, Form, SpaceDelimited, PipeDelimited, DeepObject, Cookie, Content };
enum class Encoding { Component, Reserved, None, Form };
enum class Scalar { Any, String, Boolean, Integer, Number };
enum class ShapeKind { Scalar, Array, Object };
struct WireShape {
    ShapeKind kind = ShapeKind::Scalar;
    Scalar scalar = Scalar::String;
    std::map<std::string, Scalar, std::less<>> properties;
    bool additional = false;
    Scalar extra = Scalar::Any;
};
struct Serialization {
    Style style = Style::Simple;
    bool explode = false;
    WireShape shape;
    Encoding encoding = Encoding::Component;
    bool content_json = false;
};
struct Media {
    std::string declared;
    std::string type;
    std::string subtype;
    std::map<std::string, std::string, std::less<>> parameters;
    bool utf8 = false;
};
struct ServerVariable {
    Source source;
    std::string name;
    std::string default_value;
    Presence<std::vector<std::string>> values;
};
struct Server {
    Source source;
    std::string document_url;
    std::string url;
    std::vector<ServerVariable> variables;
};
enum class CredentialKind { Bearer, Basic, HeaderKey, QueryKey, CookieKey, Provider };
struct Requirement {
    Source source;
    std::string field;
    std::string scheme;
    CredentialKind kind = CredentialKind::Bearer;
    std::string wire_name;
    std::vector<std::string> scopes;
    std::vector<std::string> roles;
    JsonValue metadata;
};
struct Operation {
    Source source;
    std::string id;
    std::string method;
    std::string path;
    std::vector<Server> servers;
    std::vector<std::vector<Requirement>> security;
    std::vector<Media> accept;
};
using CredentialValue = std::variant<std::reference_wrapper<const std::string>,
    std::reference_wrapper<const BasicCredentials>, std::reference_wrapper<const CredentialProvider>>;
using CredentialValues = std::map<std::string, CredentialValue, std::less<>>;
struct ParameterValue {
    Source source;
    std::string name;
    Location location = Location::Query;
    bool required = false;
    Serialization serialization;
    JsonValue value;
    Presence<std::string> encoded_form;
};
struct Settings {
    std::size_t max_request_bytes = @REQUEST_BYTES@;
    std::size_t max_part_bytes = @PART_BYTES@;
    std::size_t max_parts = @PARTS@;
    std::size_t max_item_bytes = @STREAM_ITEM_BYTES@;
    Presence<std::string> server_url, document_url, response_media;
    /** ua/v1 attribution: the caller override/suppression and application identity. */
    Presence<std::string> user_agent, application_id;
    std::size_t server_index = 0;
    std::map<std::string, std::string, std::less<>> variables;
    Presence<std::size_t> security_alternative;
    TransportOptions transfer;
    Control control() const { return {transfer.stop, transfer.deadline}; }
};
struct EncodedBody { std::string bytes; std::string content_type; };
struct RawPart { std::string name; Headers headers; std::string bytes; };
struct ObjectRules {
    Source source;
    std::vector<std::string> required;
    Presence<std::size_t> minimum, maximum;
};
enum class PartEncoding { Json, Text, Binary, Style };
struct PartRules {
    Source source;
    std::string name;
    bool repeated = false, required = false;
    Presence<std::size_t> minimum, maximum;
    std::size_t max_bytes = @PART_BYTES@;
    PartEncoding encoding = PartEncoding::Text;
    Scalar scalar = Scalar::String;
    Serialization serialization;
    Encoding outer = Encoding::None;
    std::vector<Media> content_types;
};
Settings settings(const ClientOptions&, const CallOptions&, const Source&);
HttpRequest prepare_request(const Operation&, const std::vector<ParameterValue>&, Presence<EncodedBody>,
    const CredentialValues&, const Settings&, Context&);
Result<HttpExchange, SdkError> open_exchange(const std::shared_ptr<const Transport>&, const HttpRequest&, const Settings&, const Source&);
Result<HttpResponse, SdkError> collect(HttpExchange exchange, const Settings&, const Source&);
Result<HttpResponse, SdkError> exchange(const std::shared_ptr<const Transport>&, const HttpRequest&, const Settings&, const Source&);
std::string lower_ascii(std::string_view);
bool header_name(std::string_view);
bool header_value(std::string_view);
Presence<Media> parse_media(std::string_view, bool ranges = false);
Presence<std::string> media_type(const Headers&);
Presence<std::string> content_type(const Headers&);
bool matches_media(const Media&, const Media&);
std::size_t select_media(const std::vector<Media>&, std::string_view, const Source&);
bool forbidden_body(std::string_view method, int status);
ResponseMetadata metadata(const HttpResponse&, std::size_t capture_limit, bool interrupted = false);
void bound_metadata(ResponseMetadata&, const TransportOptions&);
SdkError codec_error(const Operation&, CodecError, Presence<ResponseMetadata>);
SdkError transport_error(TransportError, const Source&);
std::string scalar_text(const JsonValue&, Scalar, const Source&);
JsonValue parse_scalar(std::string_view, Scalar, Context&, const Source&);
std::string encode_parameter(std::string_view, Location, const Serialization&, const JsonValue&, Context&, const Source&, std::size_t limit);
JsonValue decode_header(const Headers&, std::string_view, const Serialization&, Context&, const Source&);
Presence<std::string> find_header(const Headers&, std::string_view, const Source&, bool allow_list);
void append_bounded(std::string&, std::string_view, std::size_t, const Source&, Context&);
std::string encode_component(std::string_view, Encoding, Context&, const Source&, std::size_t limit);
std::string decode_component(std::string_view, bool form, Context&, const Source&);
std::string request_bytes(const Bytes&, Context&, const Source&, std::size_t limit);
Bytes response_bytes(std::string_view, Context&, const Source&, std::size_t limit);
void aggregate_rules(const ObjectRules&, const std::set<std::string>& names);
void part_count(const PartRules&, std::size_t count);
std::string part_json_bytes(const PartRules&, const JsonValue&, Context&, std::size_t limit);
JsonValue part_json_value(const PartRules&, std::string_view, Context&);
EncodedBody encode_multipart(std::vector<RawPart>, std::string content_type, Context&, const Settings&, const Source&);
std::vector<RawPart> decode_multipart(std::string_view bytes, std::string_view content_type, Context&, const Settings&, const Source&);
std::vector<RawPart> decode_form(std::string_view, Context&, const Settings&, const Source&);
std::string encode_form_field(const PartRules&, const JsonValue&, std::string_view name, Context&, std::size_t limit);
std::string select_part_media(const PartRules&, const Presence<std::string>& supplied);
JsonValue form_value(const PartRules&, const std::vector<RawPart>&, Context&);
JsonValue styled_part_value(const PartRules&, std::string_view, Context&);
Presence<std::string> part_filename(const RawPart&, const Source&);
std::string disposition(std::string_view name, const Presence<std::string>& filename, const Source&);
std::string integer_digits(const JsonNumber&, std::size_t limit, const Source&);
} // namespace @NAMESPACE@::detail
