//! Native Go 1.23 regressions for the independent M3 runtime review.
//! Consumers exercise generated public APIs; policy comes from canonical plans.

use std::{path::Path, process::Command, sync::Arc};

use serde_json::{Value, json};
use suspect_codegen::go_http::{HttpConfig, PackageConfig, emit_http, plan_http};
use suspect_codegen::{
    OutFile,
    go_codecs::{CodecConfig, plan_codecs},
};
use suspect_ir::contract::Contract;
use suspect_ref::WorkspaceBuilder;
use suspect_source::Uri;

fn contract(document: Value) -> Arc<Contract> {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("api.json");
    std::fs::write(&path, document.to_string()).unwrap();
    let workspace = Arc::new(
        WorkspaceBuilder::new()
            .root(directory.path())
            .build()
            .unwrap(),
    );
    Arc::new(Contract::from_workspace(&workspace, &Uri::from_path(&path).unwrap()).unwrap())
}

fn codecs(schemas: Value, config: CodecConfig) -> Vec<OutFile> {
    let contract = contract(
        json!({"openapi":"3.1.0","info":{"title":"Go runtime regressions","version":"1"},"paths":{},"components":{"schemas":schemas}}),
    );
    plan_codecs(contract.clone(), contract.schema_roots(), config)
        .unwrap()
        .render()
}

fn http(config: HttpConfig) -> Vec<OutFile> {
    let contract = contract(json!({
        "openapi":"3.1.0","info":{"title":"Go HTTP runtime regressions","version":"1"},
        "servers":[{"url":"https://runtime.example.test/v1"}],"security":[{"apiKey":[]}],
        "paths":{"/probe":{"post":{
            "operationId":"probe",
            "parameters":[
                {"name":"tag","in":"query","schema":{"type":"string"}},
                {"name":"enabled","in":"query","schema":{"type":"boolean"}},
                {"name":"tags","in":"query","schema":{"type":"array","items":{"type":"string"}}},
                {"name":"quantity","in":"query","schema":{"type":"integer"}}
            ],
            "requestBody":{"required":false,"content":{"application/json":{"schema":{"$ref":"#/components/schemas/Payload"}}}},
            "responses":{"200":{"description":"Accepted","content":{"application/json":{"schema":{"type":"string"}}}}}
        }}},
        "components":{"securitySchemes":{"apiKey":{"type":"http","scheme":"bearer"}},"schemas":{
            "Payload":{"type":"object","required":["name"],"properties":{"name":{"type":"string"}},"additionalProperties":false}
        }}
    }));
    let selected = contract
        .operations()
        .map(|operation| operation.source().clone())
        .collect::<Vec<_>>();
    let plan = plan_http(contract, &selected, config).unwrap();
    emit_http(&plan, &PackageConfig::default()).unwrap()
}

fn native(files: &[OutFile], module: &str, source: &str) {
    let root = tempfile::Builder::new()
        .prefix("suspect-go-runtime-")
        .tempdir()
        .unwrap()
        .keep();
    suspect_codegen::write_files(files, &root).unwrap();
    let consumer = root.join("consumer");
    std::fs::create_dir(&consumer).unwrap();
    std::fs::write(consumer.join("go.mod"), format!("module example.com/runtime-consumer\n\ngo 1.23.0\nrequire {module} v0.0.0\nreplace {module} => ../go\n")).unwrap();
    std::fs::write(
        consumer.join("runtime_test.go"),
        source.replace("SDK_MODULE", module),
    )
    .unwrap();
    let output = go(&consumer, &["test", "-count=1", "-timeout=15s", "-v", "."]);
    assert!(
        output.status.success(),
        "native fixture {}\n{}{}",
        root.display(),
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    std::fs::remove_dir_all(root).unwrap();
}

fn go(directory: &Path, args: &[&str]) -> std::process::Output {
    Command::new("go")
        .current_dir(directory)
        .args(args)
        .env("GOWORK", "off")
        .env(
            "GOTOOLCHAIN",
            std::env::var_os("SUSPECT_GO_TOOLCHAIN").unwrap_or_else(|| "go1.23.12".into()),
        )
        .output()
        .unwrap()
}

fn string_union() -> Value {
    json!({"StringUnion":{"oneOf":[{"type":"string","minLength":3},{"type":"string","maxLength":2}]}})
}

#[test]
#[ignore = "requires native Go 1.23"]
fn selected_union_arm_is_validated_before_the_parent_union() {
    let files = codecs(string_union(), CodecConfig::default());
    native(
        &files,
        "example.com/generated-models",
        r#"package consumer
import("encoding/json";"errors";"strings";"testing";sdk "SDK_MODULE")
func TestSelectedUnionArm(t *testing.T){
 for _,value:=range []sdk.StringUnion{sdk.NewStringUnionVariant1("x"),sdk.NewStringUnionVariant2("long")}{
  t.Run("invalid-selected-arm",func(t *testing.T){
   for _,encode:=range []func()(any,error){
    func()(any,error){return sdk.Codecs.StringUnion.Encode(value)},
    func()(any,error){return sdk.Codecs.StringUnion.EncodeValue(value)},
    func()(any,error){return json.Marshal(value)},
   }{
    wire,err:=encode();var failure *sdk.CodecError
    if !errors.As(err,&failure)||failure.Kind!="invalid"{t.Fatalf("invalid selected %T encoded as %s: %v",value,wire,err)}
    if failure.Source.Document==""||!strings.Contains(failure.Source.Pointer,"/components/schemas/StringUnion/oneOf/"){t.Fatalf("lost selected-arm source: %#v",failure)}
   }
  })
 }
 for _,value:=range []sdk.StringUnion{sdk.NewStringUnionVariant1("long"),sdk.NewStringUnionVariant2("x")}{
  encoded,err:=sdk.Codecs.StringUnion.Encode(value);if err!=nil{t.Fatal(err)}
  decoded,err:=sdk.Codecs.StringUnion.Decode(encoded);if err!=nil{t.Fatal(err)}
  switch value.(type){case sdk.StringUnionVariant1:if _,ok:=decoded.(sdk.StringUnionVariant1);!ok{t.Fatalf("arm changed: %T -> %T",value,decoded)};case sdk.StringUnionVariant2:if _,ok:=decoded.(sdk.StringUnionVariant2);!ok{t.Fatalf("arm changed: %T -> %T",value,decoded)}}
 }
}
"#,
    );
}

#[test]
#[ignore = "requires native Go 1.23"]
fn non_nil_pointer_union_wrappers_encode_the_same_selected_arm() {
    let files = codecs(string_union(), CodecConfig::default());
    native(
        &files,
        "example.com/generated-models",
        r#"package consumer
import("bytes";"encoding/json";"errors";"testing";sdk "SDK_MODULE")
func TestPointerUnion(t *testing.T){
 pointer:=&sdk.StringUnionVariant1{Value:"long"}
 var value sdk.StringUnion=pointer
 explicit,err:=sdk.Codecs.StringUnion.Encode(value);if err!=nil{t.Fatalf("public union rejects its non-nil pointer wrapper: %v",err)}
 adapter,err:=json.Marshal(pointer);if err!=nil{t.Fatal(err)}
 if !bytes.Equal(explicit,adapter)||string(explicit)!=`"long"`{t.Fatalf("pointer boundaries disagree: %s vs %s",explicit,adapter)}
 generic,err:=sdk.Codecs.StringUnion.EncodeValue(value);if err!=nil||generic!="long"{t.Fatalf("pointer EncodeValue: %v %v",generic,err)}
 decoded,err:=sdk.Codecs.StringUnion.Decode(explicit);if err!=nil{t.Fatal(err)}
 if arm,ok:=decoded.(sdk.StringUnionVariant1);!ok||arm.Value!="long"{t.Fatalf("pointer selected the wrong arm: %#v",decoded)}
 pointer.Value="x"
 var invalid *sdk.CodecError
 if _,err=sdk.Codecs.StringUnion.Encode(value);!errors.As(err,&invalid)||invalid.Kind!="invalid"{t.Fatalf("mutated pointer skipped selected-arm validation: %v",err)}
 var absent *sdk.StringUnionVariant1
 value=absent
 var conversion *sdk.CodecError
 if _,err=sdk.Codecs.StringUnion.Encode(value);!errors.As(err,&conversion)||conversion.Kind!="conversion"{t.Fatalf("nil pointer union accepted: %v",err)}
}
"#,
    );
}

#[test]
#[ignore = "requires native Go 1.23"]
fn nullable_and_presence_value_states_cannot_encode_inner_null() {
    let files = codecs(
        json!({
            "AnyMap":{"type":"object","additionalProperties":{}},
            "AnyRecord":{"type":"object","required":["requiredAny"],"properties":{"requiredAny":{},"maybe":{}},"additionalProperties":false}
        }),
        CodecConfig::default(),
    );
    native(
        &files,
        "example.com/generated-models",
        r#"package consumer
import("errors";"strings";"testing";sdk "SDK_MODULE")
func TestNonNullStates(t *testing.T){
 badMap:=sdk.AnyMap{"key":sdk.NullableValue[sdk.Value](nil)}
 badRequired:=sdk.NewAnyRecord(sdk.NullableValue[sdk.Value](nil))
 badPresence:=sdk.NewAnyRecord(sdk.NullableNull[sdk.Value]());badPresence.Maybe=sdk.PresenceSome[sdk.Value](nil)
 for _,item:=range []struct{name,path string;encode func()(any,error)}{
  {"map nullable","/key",func()(any,error){return sdk.Codecs.AnyMap.Encode(badMap)}},
  {"map nullable value API","/key",func()(any,error){return sdk.Codecs.AnyMap.EncodeValue(badMap)}},
  {"required nullable","/requiredAny",func()(any,error){return sdk.Codecs.AnyRecord.Encode(badRequired)}},
  {"optional presence","/maybe",func()(any,error){return sdk.Codecs.AnyRecord.Encode(badPresence)}},
 }{t.Run(item.name,func(t *testing.T){
  wire,err:=item.encode();var failure *sdk.CodecError
  if !errors.As(err,&failure)||failure.Kind!="conversion"{t.Fatalf("non-null state silently became null: %s %v",wire,err)}
  if failure.Path!=item.path||!strings.Contains(failure.Source.Pointer,"/components/schemas/"){t.Fatalf("lost wrapper source/path: %#v",failure)}
 })}
 valid:=sdk.AnyMap{"null":sdk.NullableNull[sdk.Value](),"false":sdk.NullableValue[sdk.Value](false),"object":sdk.NullableValue[sdk.Value](map[string]sdk.Value{"nested":nil})}
 wire,err:=sdk.Codecs.AnyMap.Encode(valid);if err!=nil{t.Fatal(err)}
 decoded,err:=sdk.Codecs.AnyMap.Decode(wire);if err!=nil{t.Fatal(err)}
 if decoded["null"].IsValue||!decoded["false"].IsValue||decoded["false"].Value!=false||!decoded["object"].IsValue{t.Fatalf("explicit states changed: %#v",decoded)}
 present:=sdk.NewAnyRecord(sdk.NullableNull[sdk.Value]());present.Maybe=sdk.PresenceNull[sdk.Value]()
 wire,err=sdk.Codecs.AnyRecord.Encode(present);if err!=nil{t.Fatal(err)}
 back,err:=sdk.Codecs.AnyRecord.Decode(wire);if err!=nil||!back.Maybe.IsSet||!back.Maybe.Null||back.RequiredAny.IsValue{t.Fatalf("explicit null rejected or changed: %#v %v",back,err)}
}
"#,
    );
}

#[test]
#[ignore = "requires native Go 1.23"]
fn conversion_work_is_shared_for_strings_keys_and_generic_json() {
    let files = codecs(
        json!({
            "Text":{"type":"string"},
            "OpenRecord":{"type":"object","required":["name"],"properties":{"name":{"type":"string"}}},
            "AnyMap":{"type":"object","additionalProperties":{}}
        }),
        CodecConfig {
            max_conversion_steps: 128,
            ..CodecConfig::default()
        },
    );
    native(
        &files,
        "example.com/generated-models",
        r#"package consumer
import("errors";"strings";"testing";sdk "SDK_MODULE")
func record(t *testing.T,extras map[string]sdk.Value)sdk.OpenRecord{
 t.Helper();value:=sdk.NewOpenRecord("a");for key,item:=range extras{if err:=value.SetExtra(key,item);err!=nil{t.Fatal(err)}};return value
}
func limited(t *testing.T,call func()(any,error)){
 t.Helper();wire,err:=call();var failure *sdk.CodecError
 if !errors.As(err,&failure)||failure.Kind!="resource"||!failure.ResourceLimited(){t.Fatalf("conversion budget bypassed: result %T error %v",wire,err)}
 if failure.Source.Document==""||!strings.HasPrefix(failure.Source.Pointer,"/components/schemas/"){t.Fatalf("resource failure lost model source: %#v",failure)}
}
func TestDeclaredStringWork(t *testing.T){
 for _,text:=range []string{strings.Repeat("x",65536),strings.Repeat("雪",50)}{
  limited(t,func()(any,error){return sdk.Codecs.Text.Encode(text)})
  limited(t,func()(any,error){return sdk.Codecs.Text.EncodeValue(text)})
  limited(t,func()(any,error){return sdk.Codecs.Text.DecodeValue(text)})
  limited(t,func()(any,error){return sdk.Codecs.Text.Decode([]byte(`"`+text+`"`))})
  value:=sdk.NewOpenRecord(text)
  limited(t,func()(any,error){return sdk.Codecs.OpenRecord.Encode(value)})
 }
 // Exhaustion belongs to one public call, not to the reusable Codec registry.
 encoded,err:=sdk.Codecs.Text.Encode("small");if err!=nil||string(encoded)!=`"small"`{t.Fatalf("budget leaked across calls: %s %v",encoded,err)}
 decoded,err:=sdk.Codecs.Text.Decode(encoded);if err!=nil||decoded!="small"{t.Fatal(decoded,err)}
}
func TestGenericJSONWork(t *testing.T){
 huge:=strings.Repeat("x",65536)
 many:=make([]sdk.Value,256)
 shared:=map[string]sdk.Value{"key":strings.Repeat("s",72)}
 for _,item:=range []struct{name string;extras map[string]sdk.Value}{
  {"copied string",map[string]sdk.Value{"large":huge}},
  {"nested copied key",map[string]sdk.Value{"nested":map[string]sdk.Value{huge:true}}},
  {"copied extra key",map[string]sdk.Value{huge:true}},
  {"descendant visits",map[string]sdk.Value{"items":many}},
  // Each string fits separately. Resetting at each generic clone would accept
  // this object even though their combined copied bytes exceed the allowance.
  {"shared across clones",map[string]sdk.Value{"first":strings.Repeat("a",72),"second":strings.Repeat("b",72)}},
  {"shared container copied twice",map[string]sdk.Value{"first":shared,"second":shared}},
 }{t.Run(item.name,func(t *testing.T){
  value:=record(t,item.extras)
  limited(t,func()(any,error){return sdk.Codecs.OpenRecord.Encode(value)})
  limited(t,func()(any,error){return sdk.Codecs.OpenRecord.EncodeValue(value)})
  input:=map[string]sdk.Value{"name":"a"};for key,value:=range item.extras{input[key]=value}
  limited(t,func()(any,error){return sdk.Codecs.OpenRecord.DecodeValue(input)})
  wire,err:=sdk.Encode(input,sdk.DefaultLimits());if err!=nil{t.Fatal(err)}
  limited(t,func()(any,error){return sdk.Codecs.OpenRecord.Decode(wire)})
 })}
 value:=record(t,map[string]sdk.Value{"single":strings.Repeat("a",72)})
 encoded,err:=sdk.Codecs.OpenRecord.Encode(value);if err!=nil{t.Fatalf("single clone should fit: %v",err)}
 if _,err=sdk.Codecs.OpenRecord.Decode(encoded);err!=nil{t.Fatalf("single decoded clone should fit: %v",err)}
 mapValue:=sdk.AnyMap{huge:sdk.NullableNull[sdk.Value]()}
 limited(t,func()(any,error){return sdk.Codecs.AnyMap.Encode(mapValue)})
 // The pre-copy meter must preserve ordinary shared containers and the exact
 // JSON cycle error, rather than treating all repeated identities as cycles.
 leaf:=map[string]sdk.Value{"leaf":"ok"}
 if _,err=sdk.Codecs.OpenRecord.Encode(record(t,map[string]sdk.Value{"a":leaf,"b":leaf}));err!=nil{t.Fatalf("acyclic alias rejected: %v",err)}
 cycle:=map[string]sdk.Value{};cycle["self"]=cycle
 var cyclic *sdk.JSONError
 if _,err=sdk.Codecs.OpenRecord.Encode(record(t,map[string]sdk.Value{"cycle":cycle}));!errors.As(err,&cyclic)||cyclic.Kind!=sdk.JSONCycle{t.Fatalf("cycle boundary changed: %v",err)}
}
"#,
    );
}

#[test]
#[ignore = "requires native Go 1.23 or current Go"]
fn value_codecs_reject_invalid_utf8_and_preserve_unicode_roundtrips() {
    let files = codecs(
        json!({
            "Text":{"type":"string"},
            "TextAlias":{"$ref":"#/components/schemas/Text"},
            "TextList":{"type":"array","items":{"type":"string"}},
            "TextMap":{"type":"object","additionalProperties":{"type":"string"}},
            "Record":{"type":"object","required":["text"],"properties":{"text":{"type":"string"}}},
            "GenericRecord":{"type":"object","required":["data"],"properties":{"data":{}},"additionalProperties":false}
        }),
        CodecConfig::default(),
    );
    native(
        &files,
        "example.com/generated-models",
        r#"package consumer
import("errors";"fmt";"reflect";"strings";"testing";sdk "SDK_MODULE")
func rejected[T any](t *testing.T,codec sdk.Codec[T],value T,wire sdk.Value,path string){
 t.Helper()
 for _,boundary:=range []struct{name string;call func()(any,error)}{
  {"EncodeValue",func()(any,error){return codec.EncodeValue(value)}},
  {"Encode",func()(any,error){return codec.Encode(value)}},
  {"DecodeValue",func()(any,error){return codec.DecodeValue(wire)}},
 }{t.Run(boundary.name,func(t *testing.T){
  result,err:=boundary.call();var failure *sdk.CodecError;var unicode *sdk.JSONError
  if err==nil{t.Fatalf("invalid UTF-8 escaped the public value boundary: %T",result)}
  if !errors.As(err,&unicode)||unicode.Kind!=sdk.JSONBadUnicode{t.Fatalf("UTF-8 classification lost: %v",err)}
  if !errors.As(err,&failure)||failure.Kind!="conversion"||failure.ResourceLimited(){t.Fatalf("UTF-8 conversion failure lost: %v",err)}
  if failure.Source.Document==""||!strings.HasPrefix(failure.Source.Pointer,"/components/schemas/")||failure.Path!=path{t.Fatalf("source/path lost: %#v",failure)}
 })}
}
func roundtrip[T any](t *testing.T,codec sdk.Codec[T],value T){
 t.Helper();wire,err:=codec.EncodeValue(value);if err!=nil{t.Fatal(err)}
 fromValue,err:=codec.DecodeValue(wire);if err!=nil||!reflect.DeepEqual(value,fromValue){t.Fatalf("value round-trip changed: %#v -> %#v (%v)",value,fromValue,err)}
 encoded,err:=codec.Encode(value);if err!=nil{t.Fatal(err)}
 fromBytes,err:=codec.Decode(encoded);if err!=nil||!reflect.DeepEqual(value,fromBytes){t.Fatalf("byte round-trip changed: %#v -> %#v (%v)",value,fromBytes,err)}
 exact,err:=sdk.Parse(encoded,sdk.DefaultLimits());if err!=nil||!reflect.DeepEqual(wire,exact){t.Fatalf("EncodeValue disagrees with encoded JSON: %#v %s %v",wire,encoded,err)}
}
func TestInvalidUTF8Scalars(t *testing.T){
 for _,bytes:=range [][]byte{{0xff},{0x80},{0xc0,0xaf},{0xe2,0x82},{0xed,0xa0,0x80},{0xf4,0x90,0x80,0x80}}{
  t.Run(fmt.Sprintf("%x",bytes),func(t *testing.T){value:=string(bytes);rejected(t,sdk.Codecs.Text,value,value,"")})
 }
}
func TestInvalidUTF8NestedValuesAndKeys(t *testing.T){
 bad:=string([]byte{0xff})
 t.Run("alias",func(t *testing.T){rejected(t,sdk.Codecs.TextAlias,bad,bad,"")})
 t.Run("array",func(t *testing.T){rejected(t,sdk.Codecs.TextList,sdk.TextList{"ok",bad},[]sdk.Value{"ok",bad},"/1")})
 t.Run("map value",func(t *testing.T){rejected(t,sdk.Codecs.TextMap,sdk.TextMap{"key":bad},map[string]sdk.Value{"key":bad},"/key")})
 t.Run("map key",func(t *testing.T){rejected(t,sdk.Codecs.TextMap,sdk.TextMap{bad:"ok"},map[string]sdk.Value{bad:"ok"},"")})
 t.Run("declared field",func(t *testing.T){rejected(t,sdk.Codecs.Record,sdk.NewRecord(bad),map[string]sdk.Value{"text":bad},"/text")})
 t.Run("extra value",func(t *testing.T){value:=sdk.NewRecord("ok");if err:=value.SetExtra("extra",bad);err!=nil{t.Fatal(err)};rejected(t,sdk.Codecs.Record,value,map[string]sdk.Value{"text":"ok","extra":bad},"/extra")})
 t.Run("extra key",func(t *testing.T){value:=sdk.NewRecord("ok");if err:=value.SetExtra(bad,"ok");err!=nil{t.Fatal(err)};rejected(t,sdk.Codecs.Record,value,map[string]sdk.Value{"text":"ok",bad:"ok"},"")})
 t.Run("generic descendant",func(t *testing.T){data:=map[string]sdk.Value{"nested":[]sdk.Value{bad}};rejected(t,sdk.Codecs.GenericRecord,sdk.NewGenericRecord(sdk.NullableValue[sdk.Value](data)),map[string]sdk.Value{"data":data},"/data/nested/0")})
 t.Run("generic key",func(t *testing.T){data:=map[string]sdk.Value{bad:true};rejected(t,sdk.Codecs.GenericRecord,sdk.NewGenericRecord(sdk.NullableValue[sdk.Value](data)),map[string]sdk.Value{"data":data},"/data")})
}
func TestValidUnicodeAndControlsRoundTrip(t *testing.T){
 for _,value:=range []string{"","雪🙂 café","e\u0301 é","\x00\t\n\r\b\f\"\\","\uFFFD\uFFFE\uFFFF"}{
  roundtrip(t,sdk.Codecs.Text,value)
  roundtrip(t,sdk.Codecs.TextAlias,value)
  roundtrip(t,sdk.Codecs.TextList,sdk.TextList{value})
  key:="雪/\x00~\n"
  roundtrip(t,sdk.Codecs.TextMap,sdk.TextMap{key:value})
  record:=sdk.NewRecord(value);if err:=record.SetExtra(key,value);err!=nil{t.Fatal(err)};roundtrip(t,sdk.Codecs.Record,record)
  data:=map[string]sdk.Value{key:[]sdk.Value{value}}
  roundtrip(t,sdk.Codecs.GenericRecord,sdk.NewGenericRecord(sdk.NullableValue[sdk.Value](data)))
 }
}
"#,
    );

    let limited = codecs(
        json!({"Text":{"type":"string"}}),
        CodecConfig {
            max_conversion_steps: 8,
            ..CodecConfig::default()
        },
    );
    native(
        &limited,
        "example.com/generated-models",
        r#"package consumer
import("errors";"strings";"testing";sdk "SDK_MODULE")
func TestStringAdmissionPreservesResourcePolicy(t *testing.T){
 value:=strings.Repeat(string([]byte{0xff}),128)
 for _,call:=range []func()(any,error){
  func()(any,error){return sdk.Codecs.Text.EncodeValue(value)},
  func()(any,error){return sdk.Codecs.Text.Encode(value)},
  func()(any,error){return sdk.Codecs.Text.DecodeValue(value)},
 }{
  _,err:=call();var failure *sdk.CodecError
  if !errors.As(err,&failure)||failure.Kind!="resource"||!failure.ResourceLimited(){t.Fatalf("unmetered Unicode scan or resource reclassification: %v",err)}
  if failure.Source.Document==""||failure.Source.Pointer!="/components/schemas/Text"{t.Fatalf("resource source lost: %#v",failure)}
 }
 wire,err:=sdk.Codecs.Text.EncodeValue("ok");if err!=nil||wire!="ok"{t.Fatalf("failed admission leaked its budget: %v %v",wire,err)}
 if decoded,err:=sdk.Codecs.Text.DecodeValue(wire);err!=nil||decoded!="ok"{t.Fatal(decoded,err)}
}
"#,
    );
}

#[test]
#[ignore = "requires native Go 1.23"]
fn absent_http_optional_values_are_rejected_before_transport() {
    let files = http(HttpConfig::default());
    native(
        &files,
        "example.com/generated-sdk",
        r#"package consumer
import("context";"errors";"io";"net/http";"strings";"testing";sdk "SDK_MODULE")
type doer func(*http.Request)(*http.Response,error)
func(f doer)Do(r *http.Request)(*http.Response,error){return f(r)}
func TestAbsentOptionalInputs(t *testing.T){
 zero,err:=sdk.ParseInteger("0");if err!=nil{t.Fatal(err)}
 for _,item:=range []struct{name,source string;input sdk.ProbeInput}{
  {"string","/parameters/0",sdk.ProbeInput{Tag:sdk.Optional[string]{Value:"lost"}}},
  {"boolean","/parameters/1",sdk.ProbeInput{Enabled:sdk.Optional[bool]{Value:true}}},
  {"array","/parameters/2",sdk.ProbeInput{Tags:sdk.Optional[[]string]{Value:[]string{"lost"}}}},
  {"allocated empty array","/parameters/2",sdk.ProbeInput{Tags:sdk.Optional[[]string]{Value:[]string{}}}},
  {"exact zero token","/parameters/3",sdk.ProbeInput{Quantity:sdk.Optional[sdk.Integer]{Value:zero}}},
  {"body","/requestBody",sdk.ProbeInput{Body:sdk.Optional[sdk.Payload]{Value:sdk.NewPayload("lost")}}},
 }{t.Run(item.name,func(t *testing.T){
  calls:=0
  client,err:=sdk.NewClient(sdk.ApiKey("token"),sdk.ClientOptions{Transport:doer(func(r *http.Request)(*http.Response,error){calls++;return &http.Response{StatusCode:200,Header:http.Header{"Content-Type":{"application/json"}},Body:io.NopCloser(strings.NewReader(`"ok"`))},nil})});if err!=nil{t.Fatal(err)}
  _,err=client.Probe(context.Background(),item.input);var failure *sdk.SDKError
  if !errors.As(err,&failure)||failure.Kind!="request-validation"||calls!=0{t.Fatalf("hidden Optional value reached transport: calls=%d error=%v",calls,err)}
  if failure.Operation.Pointer!="/paths/~1probe/post"||failure.Source.Pointer!="/paths/~1probe/post"+item.source{t.Fatalf("lost input source: %#v",failure)}
 })}
 calls:=0
 client,err:=sdk.NewClient(sdk.ApiKey("token"),sdk.ClientOptions{Transport:doer(func(r *http.Request)(*http.Response,error){
  calls++;var body []byte;if r.Body!=nil{body,_=io.ReadAll(r.Body)}
  if calls==1 && (r.URL.RawQuery!=""||len(body)!=0){t.Errorf("zero Optional values must be omitted: %s %s",r.URL.String(),body)}
  if calls==2 && (r.URL.Query().Get("enabled")!="false"||!r.URL.Query().Has("tag")||string(body)!=`{"name":"sent"}`){t.Errorf("present zero values or body lost: %s %s",r.URL.String(),body)}
  return &http.Response{StatusCode:200,Header:http.Header{"Content-Type":{"application/json"}},Body:io.NopCloser(strings.NewReader(`"ok"`))},nil
 })});if err!=nil{t.Fatal(err)}
 if _,err=client.Probe(context.Background(),sdk.NewProbeInput());err!=nil{t.Fatal(err)}
 if _,err=client.Probe(context.Background(),sdk.NewProbeInput().WithTag("").WithEnabled(false).WithBody(sdk.NewPayload("sent")));err!=nil{t.Fatal(err)}
 if calls!=2{t.Fatal(calls)}
}
"#,
    );
}

#[test]
#[ignore = "requires native Go 1.23"]
fn client_timeouts_cover_injected_transports_and_response_body_lifetime() {
    let files = http(HttpConfig::default());
    native(
        &files,
        "example.com/generated-sdk",
        r#"package consumer
import("context";"errors";"io";"net/http";"net/http/httptest";"strings";"testing";"time";sdk "SDK_MODULE")
type doer func(*http.Request)(*http.Response,error)
func(f doer)Do(r *http.Request)(*http.Response,error){return f(r)}
type contextBody struct{ctx context.Context;started,closed bool}
func(b *contextBody)Read(p []byte)(int,error){if !b.started{b.started=true;return copy(p,`"`),nil};<-b.ctx.Done();return 0,b.ctx.Err()}
func(b *contextBody)Close()error{b.closed=true;return nil}
type trackedBody struct{io.Reader;closed bool}
func(b *trackedBody)Close()error{b.closed=true;return nil}
func response(body io.ReadCloser)*http.Response{return &http.Response{StatusCode:200,Header:http.Header{"Content-Type":{"application/json"}},Body:body}}
func deadline(t *testing.T,err error)*sdk.SDKError{
 t.Helper();var failure *sdk.SDKError
 if !errors.As(err,&failure)||failure.Kind!="cancelled"||!errors.Is(err,context.DeadlineExceeded)||errors.Is(err,context.Canceled){t.Fatalf("deadline classification/cause lost: %#v %v",failure,err)}
 return failure
}
func TestInjectedDeadlineCoversBothPhases(t *testing.T){
 for _,phase:=range []string{"headers","body","late successful response"}{t.Run(phase,func(t *testing.T){
  var body *contextBody;var late *trackedBody
  client,err:=sdk.NewClient(sdk.ApiKey("token"),sdk.ClientOptions{Timeout:20*time.Millisecond,Transport:doer(func(r *http.Request)(*http.Response,error){
   if _,ok:=r.Context().Deadline();!ok{t.Error("ClientOptions.Timeout did not reach injected Doer");return response(io.NopCloser(strings.NewReader(`"ok"`))),nil}
   switch phase{
   case "headers":<-r.Context().Done();return nil,r.Context().Err()
   case "late successful response":<-r.Context().Done();late=&trackedBody{Reader:strings.NewReader(`"ok"`)};return response(late),nil
   default:body=&contextBody{ctx:r.Context()};return response(body),nil
   }
  })});if err!=nil{t.Fatal(err)}
  _,err=client.Probe(context.Background(),sdk.NewProbeInput());failure:=deadline(t,err)
  if phase=="body" && (body==nil||!body.closed||failure.Status!=200||string(failure.RawCapture)!=`"`||!failure.Truncated){t.Fatalf("body deadline lost cleanup or metadata: %#v %#v",body,failure)}
  if phase=="late successful response" && (late==nil||!late.closed){t.Fatal("late response body leaked")}
 })}
}
func TestTimeoutWithInjectedStandardHTTPClient(t *testing.T){
 server:=httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter,r *http.Request){
  select{case <-r.Context().Done():return;case <-time.After(150*time.Millisecond):}
  w.Header().Set("Content-Type","application/json");_,_=io.WriteString(w,`"ok"`)
 }));defer server.Close()
 client,err:=sdk.NewClient(sdk.ApiKey("token"),sdk.ClientOptions{ServerURL:server.URL,Timeout:20*time.Millisecond,Transport:server.Client()});if err!=nil{t.Fatal(err)};defer client.CloseIdleConnections()
 _,err=client.Probe(context.Background(),sdk.NewProbeInput());deadline(t,err)
}
func TestCallerDeadlineIsNeverExtended(t *testing.T){
 ctx,cancel:=context.WithTimeout(context.Background(),30*time.Millisecond);defer cancel();want,_:=ctx.Deadline()
 client,err:=sdk.NewClient(sdk.ApiKey("token"),sdk.ClientOptions{Timeout:time.Hour,Transport:doer(func(r *http.Request)(*http.Response,error){
  got,ok:=r.Context().Deadline();if !ok||!got.Equal(want){t.Errorf("caller deadline extended: %v != %v",got,want)}
  <-r.Context().Done();return nil,r.Context().Err()
 })});if err!=nil{t.Fatal(err)}
 _,err=client.Probe(ctx,sdk.NewProbeInput());deadline(t,err)
}
"#,
    );
}

#[test]
#[ignore = "requires native Go 1.23"]
fn transport_and_body_errors_share_cancellation_classification_and_causes() {
    let files = http(HttpConfig::default());
    native(
        &files,
        "example.com/generated-sdk",
        r#"package consumer
import("context";"errors";"fmt";"io";"net/http";"strings";"testing";sdk "SDK_MODULE")
type doer func(*http.Request)(*http.Response,error)
func(f doer)Do(r *http.Request)(*http.Response,error){return f(r)}
type failedBody struct{err error;sent,closed bool}
func(b *failedBody)Read(p []byte)(int,error){if !b.sent{b.sent=true;return copy(p,"private-body"),nil};return 0,b.err}
func(b *failedBody)Close()error{b.closed=true;return nil}
func TestClassifiedCausesInBothPhases(t *testing.T){
 for _,cause:=range []error{context.Canceled,context.DeadlineExceeded,io.ErrUnexpectedEOF}{
  for _,phase:=range []string{"headers","body"}{t.Run(cause.Error()+"/"+phase,func(t *testing.T){
   wrapped:=fmt.Errorf("private-cause: %w",cause);var body *failedBody
   client,err:=sdk.NewClient(sdk.ApiKey("token"),sdk.ClientOptions{Transport:doer(func(r *http.Request)(*http.Response,error){
    // A transport's own timeout can expire while the caller context is live.
    if r.Context().Err()!=nil{t.Fatal("test requires a live caller context")}
    if phase=="headers"{return nil,wrapped}
    body=&failedBody{err:wrapped};return &http.Response{StatusCode:200,Header:http.Header{"Content-Type":{"application/json"}},Body:body},nil
   })});if err!=nil{t.Fatal(err)}
   _,err=client.Probe(context.Background(),sdk.NewProbeInput());var failure *sdk.SDKError
   kind:="cancelled";if cause==io.ErrUnexpectedEOF{kind="transport"}
   if !errors.As(err,&failure)||failure.Kind!=kind||!errors.Is(err,cause)||!errors.Is(err,wrapped){t.Fatalf("phase-dependent classification/cause: %#v %v",failure,err)}
   if cause==context.Canceled && errors.Is(err,context.DeadlineExceeded)||cause==context.DeadlineExceeded && errors.Is(err,context.Canceled){t.Fatal("cancel and deadline causes collapsed")}
   if phase=="body"&&(body==nil||!body.closed||failure.Status!=200||string(failure.RawCapture)!="private-body"||!failure.Truncated){t.Fatalf("body metadata/cleanup lost: %#v %#v",failure,body)}
   if formatted:=fmt.Sprintf("%+v",err);strings.Contains(formatted,"private-"){t.Fatalf("error formatting exposed diagnostics: %q",formatted)}
  })}
 }
}
"#,
    );
}

#[test]
#[ignore = "requires native Go 1.23"]
fn bearer_padding_requires_a_non_padding_token_before_transport() {
    let files = http(HttpConfig::default());
    native(
        &files,
        "example.com/generated-sdk",
        r#"package consumer
import("context";"errors";"io";"net/http";"strings";"testing";sdk "SDK_MODULE")
type doer func(*http.Request)(*http.Response,error)
func(f doer)Do(r *http.Request)(*http.Response,error){return f(r)}
func TestBearerGrammar(t *testing.T){
 for _,item:=range []struct{token string;valid bool}{
  {"",false},{"=",false},{"====",false},{"=token",false},{"to=ken",false},{"token space",false},{"雪",false},
  {"A",true},{"a=",true},{"a==",true},{"aZ09-._~+/",true},{"aZ09-._~+/===",true},
 }{t.Run(item.token,func(t *testing.T){
  calls:=0
  client,err:=sdk.NewClient(sdk.ApiKey(item.token),sdk.ClientOptions{Transport:doer(func(r *http.Request)(*http.Response,error){
   calls++;if r.Header.Get("Authorization")!="Bearer "+item.token{t.Errorf("bearer token changed: %q",r.Header.Get("Authorization"))}
   return &http.Response{StatusCode:200,Header:http.Header{"Content-Type":{"application/json"}},Body:io.NopCloser(strings.NewReader(`"ok"`))},nil
  })});if err!=nil{t.Fatal(err)}
  _,err=client.Probe(context.Background(),sdk.NewProbeInput())
  if item.valid{if err!=nil||calls!=1{t.Fatalf("valid bearer rejected: calls=%d %v",calls,err)};return}
  var failure *sdk.SDKError
  if !errors.As(err,&failure)||failure.Kind!="request-validation"||calls!=0{t.Fatalf("invalid bearer reached transport: calls=%d %v",calls,err)}
  if failure.Source.Pointer!="/components/securitySchemes/apiKey"{t.Fatalf("lost bearer source: %#v",failure)}
 })}
}
"#,
    );
}
