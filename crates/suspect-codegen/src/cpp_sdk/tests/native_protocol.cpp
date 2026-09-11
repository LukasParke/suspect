#include <generated_sdk/sdk.hpp>
#include <cstdlib>
#include <filesystem>
#include <iostream>
#include <thread>
using namespace generated_sdk;
__ALIASES__
#define CHECK(x) do { if (!(x)) { std::cerr << "failed line " << __LINE__ << ": " << #x << '\n'; std::abort(); } } while(false)
template<class T,class E> const SdkError& error(const Result<T,E>& result,SdkError::Kind kind){CHECK(!result);const auto* error=std::get_if<SdkError>(&result.error());CHECK(error);if(error->kind!=kind)std::cerr<<error->message<<" kind="<<static_cast<int>(error->kind)<<'\n';CHECK(error->kind==kind);return *error;}
static bool wait_file(const std::filesystem::path& path){for(unsigned i=0;i<500;++i){if(std::filesystem::exists(path))return true;std::this_thread::sleep_for(std::chrono::milliseconds(5));}return false;}
__EDGE_CASES__
int main(int argc,char** argv){
    edge_cases();
    CHECK(argc==3);std::string base=argv[1];std::filesystem::path directory=argv[2];
    Credentials credentials;credentials.bearer="native-token";credentials.basic=BasicCredentials("Aladdin","open sesame");
    credentials.header_key="header-token";credentials.query_key="a/+ b";credentials.cookie_key="cookie%20value";
    unsigned oauth_calls=0,oidc_calls=0;
    credentials.oauth=CredentialProvider([&](const CredentialRequest& request){++oauth_calls;CHECK(request.scheme_name=="oauth");CHECK(request.scopes==std::vector<std::string>{"read"});CHECK(request.roles.empty());CHECK(request.metadata.as<JsonValue::Object>().contains("flows"));return Result<Authorization,TransportError>::success(Authorization("Bearer","oauth-token"));});
    credentials.oidc=CredentialProvider([&](const CredentialRequest& request){++oidc_calls;CHECK(request.scopes==std::vector<std::string>{"openid"});CHECK(request.metadata.as<JsonValue::Object>().contains("discovery_url"));return Result<Authorization,TransportError>::success(Authorization("Bearer","oidc-token"));});
    ClientOptions settings;settings.server_url=base;settings.max_capture_bytes=32;
    auto connected=Client::with_curl(credentials,settings);CHECK(connected);auto client=std::move(connected).value();
    CHECK(client.public_call());CHECK(client.secure());CHECK(oauth_calls==0&&oidc_calls==0);
    auto unnamed=client.get_unnamed();CHECK(unnamed);CHECK(std::get<GetUnnamedStatus200>(unnamed.value()).data=="ok");
    auto free=client.schema_free();CHECK(free);CHECK(std::get<SchemaFreeStatus200>(free.value()).data.as<JsonValue::Object>().at("large").as<JsonNumber>().token()=="1e1000000000000000000");
    auto unbound=client.unbound();error(unbound,SdkError::Kind::UnexpectedResponse);
    auto text_number=client.text_number();CHECK(text_number);CHECK(std::get<TextNumberStatus200>(text_number.value()).data.token()=="9007199254740993.00000000000001");
    for(std::size_t alternative:{1u,2u,3u,4u}){CallOptions options;options.security_alternative=alternative;CHECK(client.secure(SecureInput{},options));}
    CHECK(oauth_calls==1&&oidc_calls==1);
    Credentials missing;auto absent=Client::with_curl(missing,settings);CHECK(absent);CallOptions required;required.security_alternative=1;
    error(absent.value().secure(SecureInput{},required),SdkError::Kind::RequestValidation);
    // Native source-selected parameter vectors; values and expected wire strings
    // come from the independent OpenAPI fixture, not the SDK serializer.
    __PARAMETER_CALLS__
    CHECK(client.query_text(QueryTextInput("a=b & 雪%2F")));
    CHECK(client.query_json(QueryJsonInput(TestWholeJson(true))));
    TestWholeForm whole(true,"a + b");whole.items=std::vector<JsonInteger>{JsonInteger(1),JsonInteger(2)};
    CHECK(client.query_form(QueryFormInput(whole)));
    // Actual URL selection: defaults/variables, then a relative server resolved
    // from the explicit document URL. No host is invented for the local spec.
    auto origin=base.substr(0,base.rfind('/'));auto port=origin.substr(origin.rfind(':')+1);
    ClientOptions server_options;server_options.document_url=origin+"/specs/openapi.json";
    auto servers=Client::with_curl({},server_options);CHECK(servers);
    CallOptions variables;variables.server_variables=std::map<std::string,std::string,std::less<>>{{"port",port},{"base","v2"}};
    CHECK(servers.value().servers(ServersInput{},variables));
    variables.server_variables=std::map<std::string,std::string,std::less<>>{{"port",port},{"base","wrong"}};
    error(servers.value().servers(ServersInput{},variables),SdkError::Kind::Configuration);
    CallOptions relative;relative.server_index=1;CHECK(servers.value().servers(ServersInput{},relative));
    // Media selectors preserve exact values and cannot bypass a specific model.
    Record record("json");record.value=JsonNumber::parse("9007199254740993.000000000000000001").value();
    auto json_response=client.media(MediaInput(TestJsonBody(record)));CHECK(json_response);
    CHECK(std::get<TestJsonResponse>(json_response.value()).data.value->token()=="9007199254740993.000000000000000001");
    record.message="problem";auto problem=client.media(MediaInput(TestJsonBody(record)));CHECK(problem);CHECK(std::get<TestProblemResponse>(problem.value()).data.message=="problem");
    record.message="profile";auto profile=client.media(MediaInput(TestJsonBody(record)));CHECK(profile);CHECK(std::get<TestProfileResponse>(profile.value()).data.message=="profile");
    auto text=client.media(MediaInput(TestTextBody("raw UTF-8 雪")));CHECK(text);CHECK(std::get<TestTextResponse>(text.value()).data=="raw UTF-8 雪");
    Bytes bytes{0,255,1,2,13,10};auto binary=client.media(MediaInput(TestBinaryBody(bytes)));CHECK(binary);CHECK(std::get<TestApplicationResponse>(binary.value()).data==bytes);
    auto wildcard=client.media(MediaInput(TestAnyBody(bytes,"image/png")));CHECK(wildcard);CHECK(std::get<TestAnyResponse>(wildcard.value()).data==bytes);
    auto empty_bytes=client.media(MediaInput(TestAnyBody(Bytes{},"image/png")));CHECK(empty_bytes);CHECK(std::get<TestAnyResponse>(empty_bytes.value()).data.empty());
    auto bypass=client.media(MediaInput(TestAnyBody(bytes,"application/json")));error(bypass,SdkError::Kind::RequestRepresentation);
    CallOptions want_json;want_json.response_media="application/json";error(client.media(MediaInput(TestTextBody("other")),want_json),SdkError::Kind::UnexpectedResponse);
    // Exact status wins even when a class response would match the media.
    error(client.precedence(PrecedenceInput("exact-wrong-media")),SdkError::Kind::UnexpectedResponse);
    auto ranged=client.precedence(PrecedenceInput("range"));CHECK(ranged);CHECK(std::get<TestRangeResponse>(ranged.value()).response.status==207);
    auto no_content=client.precedence(PrecedenceInput("range-empty"));CHECK(no_content);CHECK(std::get<TestRangeNone>(no_content.value()).response.status==204);
    auto fallback_success=client.default_status();CHECK(fallback_success);CHECK(std::get<TestDefaultSuccess>(fallback_success.value()).response.status==201);
    auto fallback=client.precedence(PrecedenceInput("default-error"));CHECK(!fallback);CHECK(std::get<TestDefaultResponse>(fallback.error()).response.status==404);
    CHECK(std::get<TestDefaultResponse>(fallback.error()).data==Bytes({0,255,4}));
    auto empty=client.empty();CHECK(empty);CHECK(std::get<EmptyStatus204>(empty.value()).response.status==204);
    auto head=client.head();CHECK(head);CHECK(std::get<HeadStatus200>(head.value()).headers.x_count.token()=="99999999999999999999");
    auto headers=client.headers();CHECK(headers);const auto& parsed_headers=std::get<HeadersStatus200>(headers.value());
    CHECK(parsed_headers.headers.x_rate.token()=="1.0");CHECK(*parsed_headers.headers.x_flags==std::vector<std::string>({"one","two"}));CHECK(parsed_headers.headers.x_meta->active);
    CHECK(parsed_headers.response.links.size()==1);CHECK(parsed_headers.response.links[0].target=="media");
    CHECK(parsed_headers.response.links[0].request_body->as<JsonValue::Object>().at("$ref").as<std::string>()=="literal-data");
    // Native aggregate construction, exact form outer encoding, structural rules.
    TestForm form("a + b");form.address=TestAddress("New York");form.codes=std::vector<JsonInteger>{JsonInteger(1),JsonInteger(2)};form.tags=std::vector<std::string>{"two words","a+b"};
    auto form_response=client.form(FormInput(form));CHECK(form_response);const auto& form_value=std::get<TestFormResponse>(form_response.value()).data;
    CHECK(form_value.id=="a + b");CHECK(form_value.address->city=="New York");CHECK(form_value.codes->at(1).token()=="2");CHECK(*form_value.tags==std::vector<std::string>({"two words","a+b"}));
    auto packed=client.packed_form(PackedFormInput(TestPackedForm(Coordinates(JsonInteger(2),"a,b + 雪"))));CHECK(packed);
    CHECK(std::get<TestPackedResponse>(packed.value()).data.pair.y=="a,b + 雪");
    TestStyled styled(TestCoordinatesPart(Coordinates(JsonInteger(2),"two % words")),TestDeepPart(Coordinates(JsonInteger(3),"deep value")),std::vector<TestStyledLabelPart>{TestStyledLabelPart("one"),TestStyledLabelPart("two")});
    auto styled_response=client.styled_upload(StyledUploadInput(styled));CHECK(styled_response);const auto& styled_data=std::get<TestStyledResponse>(styled_response.value()).data;
    CHECK(styled_data.coordinates.data.y=="two % words");CHECK(styled_data.deep.data.x.token()=="3");CHECK(styled_data.labels.at(1).data=="two");CHECK(!styled_data.labels.at(0).content_type);
    styled.coordinates.data.y="x&y";error(client.styled_upload(StyledUploadInput(styled)),SdkError::Kind::RequestRepresentation);
    TestFilePart file(bytes,TestPartHeaders("part-1"));file.filename="file.bin";file.content_type="image/png";
    TestUpload upload(file,TestMetadataPart(TestMetadata("native")));upload.labels=std::vector<TestLabelPart>{TestLabelPart("one"),TestLabelPart("雪")};
    auto uploaded=client.upload(UploadInput(upload));CHECK(uploaded);const auto& uploaded_value=std::get<TestUploadResponse>(uploaded.value()).data;
    CHECK(uploaded_value.file.data==bytes);CHECK(uploaded_value.file.filename=="file.bin");CHECK(uploaded_value.file.headers.x_part_id=="part-1");
    CHECK(uploaded_value.metadata.data.title=="native");CHECK(uploaded_value.labels->at(1).data=="雪");
    upload.file.data=Bytes(65,1);error(client.upload(UploadInput(upload)),SdkError::Kind::ResourceLimit);
    upload.file.data=bytes;upload.file.filename="x\r\nInjected: value";error(client.upload(UploadInput(upload)),SdkError::Kind::RequestRepresentation);
    CHECK(client.query_method());CHECK(client.copy_method());auto lower=client.lower_head();CHECK(lower);CHECK(std::get<LowerHeadStatus200>(lower.value()).data=="lowercase head has a body");
    // Stream operation returns at the head and owns the live exchange thereafter.
    auto started=std::chrono::steady_clock::now();auto events=client.events(EventsInput("normal"));CHECK(events);
    CHECK(std::chrono::steady_clock::now()-started<std::chrono::seconds(1));auto& stream=std::get<EventsStatus200>(events.value()).data;
    auto first=stream.next();CHECK(first&&first.value());CHECK(first.value()->data=="{\"n\":1}\n雪");CHECK(first.value()->id=="7");CHECK(first.value()->retry->token()=="5");
    auto second=stream.next();CHECK(second&&second.value());CHECK(second.value()->data=="[DONE]");CHECK(second.value()->id=="7");CHECK(!second.value()->retry);
    auto done=stream.next();CHECK(done&&!done.value());CHECK(stream.is_closed());
    auto breaking=client.events(EventsInput("break"));CHECK(breaking);
    for(const auto& item:std::get<EventsStatus200>(breaking.value()).data){CHECK(item);break;}
    CHECK(wait_file(directory/"sse-break-closed"));
    auto raw_transport=CurlTransport::create();CHECK(raw_transport);
    TransportOptions window;window.deadline=std::chrono::steady_clock::now()+std::chrono::seconds(3);window.max_buffer_bytes=16384;
    auto raw_stream=raw_transport.value()->open(HttpRequest{"GET",base+"/sse/window",{{"Accept","text/event-stream"}},std::nullopt},window);CHECK(raw_stream);
    auto chunk=raw_stream.value().body->next();CHECK(chunk&&chunk.value()&&chunk.value()->size()<=16384);
    raw_stream.value().body->close();CHECK(wait_file(directory/"sse-window-closed"));
    Cancellation cancellation;CallOptions stop;stop.stop=cancellation.token();auto cancelled=client.events(EventsInput("cancel"),stop);CHECK(cancelled);cancellation.cancel();
    auto cancelled_item=std::get<EventsStatus200>(cancelled.value()).data.next();CHECK(!cancelled_item&&cancelled_item.error().kind==SdkError::Kind::Cancelled);CHECK(wait_file(directory/"sse-cancel-closed"));
    CallOptions deadline;deadline.timeout=std::chrono::milliseconds(150);auto slow=client.events(EventsInput("slow"),deadline);CHECK(slow);auto slow_item=std::get<EventsStatus200>(slow.value()).data.next();CHECK(!slow_item&&slow_item.error().kind==SdkError::Kind::Timeout);CHECK(wait_file(directory/"sse-slow-closed"));
    CallOptions small;small.max_stream_item_bytes=32;auto large=client.events(EventsInput("large"),small);CHECK(large);auto large_item=std::get<EventsStatus200>(large.value()).data.next();CHECK(!large_item&&large_item.error().kind==SdkError::Kind::ResourceLimit);
    auto lines=client.lines(LinesInput("normal"));CHECK(lines);auto& line_stream=std::get<LinesStatus200>(lines.value()).data;
    CHECK(line_stream.next().value()->n.token()=="1.0");CHECK(line_stream.next().value()->n.token()=="9007199254740993");CHECK(line_stream.next().value()->n.token()=="1e100000000000000000000");CHECK(!line_stream.next().value());
    auto bad=client.lines(LinesInput("bad"));CHECK(bad);auto bad_item=std::get<LinesStatus200>(bad.value()).data.next();CHECK(!bad_item&&bad_item.error().kind==SdkError::Kind::ResponseDecoding);
    Event event("first\nsecond");event.event="tick";event.id="7";event.retry=JsonInteger::parse("1e3").value();
    CHECK(client.emit_events(EmitEventsInput(std::vector<Event>{event})));
    std::cout<<"C++ rich protocol typed models, wire, media, auth, aggregates and RAII streams passed\n";
}
