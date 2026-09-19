//! Native reference and source-bound executable recipes over the same rich plan.
use super::{
    PlannedOperation, PlannedResponse, SdkPlan,
    emit::{file, quote, xml},
    models::CsDecl,
    protocol::{PlannedMedia, PlannedPart, PlannedParts, PlannedPositionalParts},
};
use crate::{
    OutFile,
    examples::{ExampleEntry, ExampleRole},
    http_examples, http_protocol as p,
};
use serde_json::{Value, json};
use suspect_ir::contract::SourceId;

fn address(id: &SourceId) -> Value {
    json!({"document":id.document().as_str(),"pointer":id.pointer()})
}
pub(super) fn render(plan: &SdkPlan) -> Vec<OutFile> {
    let ns = &plan.config.namespace;
    let mut symbols = Vec::new();
    let mut add = |name: String,
                   kind: &str,
                   signature: String,
                   id: &SourceId,
                   xml_id: String,
                   description: String| {
        symbols.push(json!({"id":format!("symbol-{}",symbols.len()),"name":name,"kind":kind,"signature":signature,"source":address(id),"xmlId":xml_id,"description":description}));
    };
    if let Some(environment) = plan.credential_env() {
        let source = environment.bindings()[0].scheme().use_site().source();
        add("Client.FromEnvironment".into(),"factory",format!("{ns}.Client Client.FromEnvironment(ClientOptions? options = null, HttpClient? httpClient = null)"),source,format!("M:{ns}.Client.FromEnvironment({ns}.ClientOptions,System.Net.Http.HttpClient)"),"Snapshot configured process-environment values at client creation. Whole explicit credentials remain authoritative; missing credentials fail only when the selected operation requires them.".into());
    }
    for (key, decl) in plan.models.declarations() {
        let name = &plan.models.names()[key];
        if !matches!(decl, CsDecl::Alias(_)) {
            add(
                name.clone(),
                "type",
                format!("{ns}.{name}"),
                &key.0,
                format!("T:{ns}.{name}"),
                plan.contract
                    .source(&key.0)
                    .and_then(|v| v.get("description"))
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .into(),
            );
        }
        add(
            format!("Codecs.Decode{name}"),
            "codec",
            format!(
                "{} Codecs.Decode{name}(string json)",
                plan.models.native_type(&key.0)
            ),
            &key.0,
            format!("M:{ns}.Codecs.Decode{name}(System.String)"),
            if matches!(
                plan.program.version,
                suspect_schema::OwnedProgram::V2_VERSION | suspect_schema::OwnedProgram::V3_VERSION
            ) && plan.models.is_json_carrier(&key.0)
            {
                "Checked JSON-value carrier. This exact source codec validates decode and encode; conditional/intersection constraints are runtime obligations.".into()
            } else {
                "Exact, source-validated codec. Encode validates mutations.".into()
            },
        );
        match decl {
            CsDecl::Record { fields, extras } => {
                for f in fields {
                    add(
                        format!("{name}.{}", f.name),
                        "property",
                        format!(
                            "{} {}",
                            if f.required {
                                plan.models.render_type(&f.ty)
                            } else {
                                format!("Optional<{}>", plan.models.render_type(&f.ty))
                            },
                            f.name
                        ),
                        &f.source,
                        format!("P:{ns}.{name}.{}", f.name),
                        format!(
                            "{} Wire: {}. Required: {}. Nullable: {}.",
                            f.description, f.wire, f.required, f.nullable
                        ),
                    );
                }
                if extras.is_some() {
                    add(
                        format!("{name}.Extra"),
                        "property",
                        "Typed extra properties".into(),
                        &key.0,
                        format!("P:{ns}.{name}.Extra"),
                        if matches!(
                            plan.program.version,
                            suspect_schema::OwnedProgram::V2_VERSION
                                | suspect_schema::OwnedProgram::V3_VERSION
                        ) {
                            "Decoded keys and current values are preserved. The complete source program applies every matching pattern, additional/unevaluated rule and declared-name collision check.".into()
                        } else {
                            "Mutations and declared-name collisions are validated.".into()
                        },
                    );
                }
            }
            CsDecl::Literals { values } => {
                for (member, token) in values {
                    add(
                        format!("{name}.{member}"),
                        "literal",
                        format!("{name}.{member}"),
                        &key.0,
                        format!("F:{ns}.{name}.{member}"),
                        format!("Exact source literal {token}"),
                    );
                }
            }
            CsDecl::Union { branches } => {
                for b in branches {
                    add(
                        format!("{name}.{}", b.name),
                        "union-arm",
                        format!(
                            "new {name}.{}({} value)",
                            b.name,
                            plan.models.render_type(&b.ty)
                        ),
                        &b.source,
                        format!("T:{ns}.{name}.{}", b.name),
                        "Selected arm and parent constraints both validate.".into(),
                    );
                }
            }
            _ => {}
        }
    }
    for op in &plan.operations {
        add(
            format!("Client.{}", op.method_name),
            "operation",
            format!(
                "Task<{}> {}({} input, RequestOptions? requestOptions = null, CancellationToken cancellationToken = default)",
                op.result_type, op.method_name, op.input_type
            ),
            &op.source,
            format!(
                "M:{ns}.Client.{}({ns}.{},{}.RequestOptions,System.Threading.CancellationToken)",
                op.method_name, op.input_type, ns
            ),
            op.description.clone(),
        );
        add(
            op.input_type.clone(),
            "input",
            op.input_type.clone(),
            &op.source,
            format!("T:{ns}.{}", op.input_type),
            format!("{} {}", op.http_method, op.path),
        );
        for p in &op.parameters {
            add(
                format!("{}.{}", op.input_type, p.property_name),
                "input-property",
                p.native_type.clone(),
                &p.source,
                format!("P:{ns}.{}.{}", op.input_type, p.property_name),
                format!("{} {:?}; required {}", p.wire_name, p.location, p.required),
            );
        }
        if let Some(body) = &op.body {
            add(
                format!("{}.Body", op.input_type),
                "input-property",
                body.native_type.clone(),
                &body.source,
                format!("P:{ns}.{}.Body", op.input_type),
                "Source media choice; omission and null differ.".into(),
            );
        }
        for r in &op.responses {
            if r.may_succeed() {
                add(
                    r.type_name.clone(),
                    "result",
                    r.native_type.clone(),
                    &r.source,
                    format!("T:{ns}.{}", r.type_name),
                    format!(
                        "Declared {}, actual status determines success.",
                        r.wire.status_key()
                    ),
                );
            }
            if r.may_fail() {
                add(
                    r.error_type_name.clone(),
                    "exception",
                    r.native_type.clone(),
                    &r.source,
                    format!("T:{ns}.{}", r.error_type_name),
                    format!("Declared {} non-success response.", r.wire.status_key()),
                );
            }
            if let Some(h) = &r.header_type {
                add(
                    h.clone(),
                    "headers",
                    h.clone(),
                    &r.source,
                    format!("T:{ns}.{h}"),
                    "Required headers are decoded and validated.".into(),
                );
            }
        }
        for m in op
            .body
            .iter()
            .flat_map(|b| &b.media)
            .chain(op.responses.iter().flat_map(|r| &r.media))
        {
            if let Some(parts) = &m.parts {
                add(
                    parts.native_type.clone(),
                    "parts",
                    parts.native_type.clone(),
                    parts.rules.schema().id(),
                    format!("T:{ns}.{}", parts.native_type),
                    "Named parts retain raw bytes separately from source JSON/text codecs.".into(),
                );
            }
            if let Some(parts) = &m.positional {
                add(parts.native_type.clone(),"positional-parts",parts.native_type.clone(),parts.schema.id(),format!("T:{ns}.{}",parts.native_type),"Ordered prefix and typed remaining parts. Count constraints and false-prefix barriers are enforced.".into());
                for part in &parts.prefix {
                    add(
                        format!("{}.{}", parts.native_type, part.property_name),
                        "positional-property",
                        part.native_type.clone(),
                        part.wire.schema().id(),
                        format!("P:{ns}.{}.{}", parts.native_type, part.property_name),
                        "A later position cannot be supplied after an absent prefix value.".into(),
                    );
                }
                if let Some(items) = &parts.items {
                    add(format!("{}.Items",parts.native_type),"positional-items",format!("List<{}>",items.native_type),items.wire.schema().id(),format!("P:{ns}.{}.Items",parts.native_type),"Typed remaining items after the complete prefix, validated on every encode/decode.".into());
                }
                for part in parts
                    .prefix
                    .iter()
                    .chain(parts.items.iter().map(|p| p.as_ref()))
                {
                    if let Some(name) = &part.wrapper_type {
                        add(name.clone(),"positional-part",part.value_type.clone(),part.wire.schema().id(),format!("T:{ns}.{name}"),"Actual positional JSON, text or byte payload with explicit MIME metadata.".into());
                    }
                    if let Some(name) = &part.header_type {
                        add(name.clone(),"part-headers",name.clone(),part.wire.source().use_site().source(),format!("T:{ns}.{name}"),"Source Encoding Object headers; required fields retain native construction requirements.".into());
                        for h in &part.headers {
                            add(
                                format!("{name}.{}", h.property_name),
                                "part-header-property",
                                h.native_type.clone(),
                                h.wire.codec().schema().id(),
                                format!("P:{ns}.{name}.{}", h.property_name),
                                format!(
                                    "Wire header {}; required {}.",
                                    h.wire.name(),
                                    h.wire.required()
                                ),
                            );
                        }
                    }
                }
            }
        }
    }
    let mut html = format!(
        "<!doctype html><html lang=\"en\"><head><meta charset=\"utf-8\"><meta name=\"viewport\" content=\"width=device-width\"><title>{} native reference</title><style>body{{max-width:85rem;margin:2rem auto;padding:1rem;font:16px/1.6 system-ui}}pre{{white-space:pre-wrap;overflow-wrap:anywhere;background:#eee;padding:1rem}}section{{border-top:1px solid #ccc}}nav{{columns:3}}</style></head><body><h1>{} — C# native SDK</h1><p>Task, CancellationToken, explicit credentials and native constructors. Sequential responses implement IAsyncEnumerable and IAsyncDisposable. See README for operational recipes.</p>",
        xml(&plan.config.name),
        xml(&plan.config.name)
    );
    let (program, quickstart) = examples(plan);
    html.push_str(&format!(
        "<h2>First request</h2><pre>{}</pre><h2>Reference</h2><nav>",
        xml(&quickstart)
    ));
    for s in &symbols {
        html.push_str(&format!(
            "<div><a href=\"#{}\">{}</a></div>",
            s["id"].as_str().unwrap(),
            xml(s["name"].as_str().unwrap())
        ));
    }
    html.push_str("</nav>");
    for s in &symbols {
        html.push_str(&format!("<section id=\"{}\"><h2>{}</h2><pre>{}</pre><p>{}</p><details><summary>Native/source binding</summary><code>{}</code><br><code>{}#{}</code></details></section>",s["id"].as_str().unwrap(),xml(s["name"].as_str().unwrap()),xml(s["signature"].as_str().unwrap()),xml(s["description"].as_str().unwrap()),xml(s["xmlId"].as_str().unwrap()),xml(s["source"]["document"].as_str().unwrap()),xml(s["source"]["pointer"].as_str().unwrap())));
    }
    html.push_str("</body></html>\n");
    let operations=plan.operations.iter().map(|op|json!({"source":address(&op.source),"operationId":op.operation_id,"methodName":op.method_name,"inputType":op.input_type,"resultType":op.result_type,"errorType":op.error_type,"protocol":op.wire,
        "parameters":op.parameters.iter().map(|p|json!({"source":address(&p.source),"schema":address(&p.schema),"propertyName":p.property_name,"nativeType":p.native_type,"required":p.required,"wireName":p.wire_name,"serialization":p.wire.serialization()})).collect::<Vec<_>>(),
        "body":op.body.as_ref().map(|b|json!({"source":address(&b.source),"nativeType":b.native_type,"required":b.required,"union":b.union,"media":b.media.iter().map(media_record).collect::<Vec<_>>()})),
        "responses":op.responses.iter().map(|r|json!({"source":address(&r.source),"status":r.wire.status(),"nativeType":r.native_type,"typeName":r.type_name,"errorTypeName":r.error_type_name,"alwaysEmpty":r.always_empty,"mayBeEmpty":r.may_be_empty,"union":r.union,"headerType":r.header_type,"headers":r.headers.iter().map(|h|json!({"name":h.property_name,"type":h.native_type,"wire":h.wire})).collect::<Vec<_>>(),"media":r.media.iter().map(media_record).collect::<Vec<_>>()})).collect::<Vec<_>>() })).collect::<Vec<_>>();
    let mut reference =
        json!({"format":"suspect-csharp-reference-v2","namespace":ns,"symbols":symbols});
    if matches!(
        plan.program.version,
        suspect_schema::OwnedProgram::V2_VERSION | suspect_schema::OwnedProgram::V3_VERSION
    ) {
        reference["validation"] = json!({"version":plan.program.version,"profile":plan.program.profile,"scope":"fresh-per-subschema","encode":"current-mutable-value","jsonCarriers":"checked-by-source-codecs","patternExtras":"preserved-and-validated"});
        html = html.replace(
            "</body>",
            &format!(
                "<h2>Scoped source validation</h2><p>{}</p></body>",
                xml(scoped_guide(plan))
            ),
        );
    }
    if let Some(resources) = &plan.program.resource_context {
        reference["validation"]["resourceContext"] = serde_json::to_value(resources).unwrap();
        reference["validation"]["dynamicScope"] = json!({"binding":"outermost-actually-entered-resource","lookup":"indexed-only-no-acquisition","nullability":"context-sensitive-native-domains-are-conservative","dynamicUnions":"checked-json-carriers"});
        html = html.replace(
            "</body>",
            &format!(
                "<h2>Indexed schema resources</h2><pre>{}</pre></body>",
                xml(&serde_json::to_string_pretty(resources).unwrap())
            ),
        );
    }
    if let Some(environment) = plan.credential_env() {
        reference["credentialEnv"] =
            serde_json::to_value(environment.semantic_descriptor()).unwrap();
        html = html.replace(
            "</body>",
            &format!(
                "<h2>Runtime environment credentials</h2><pre>{}</pre></body>",
                xml(&super::credential_env::guide(plan))
            ),
        );
    }
    let mut files=vec![file("README.md",readme(plan,&quickstart)+&super::credential_env::guide(plan)),file("docs/index.html",html),file("docs/reference.json",serde_json::to_string_pretty(&reference).unwrap()),
        file("http-manifest.json",serde_json::to_string_pretty(&json!({"format":"suspect-csharp-http-v2","package":{"name":plan.config.name,"version":plan.config.version,"namespace":ns,"targetFramework":"net8.0"},"capabilities":plan.protocol.capabilities(),"operations":operations,"credentials":plan.credential_bindings.iter().map(|c|json!({"key":c.key,"property":c.property_name,"type":c.native_type,"requirement":c.requirement})).collect::<Vec<_>>()})).unwrap()),
        file("examples/examples.json",http_examples::manifest(&plan.examples)),file("examples/README.md",http_examples::markdown(&plan.examples,"dotnet run --project examples/Examples.csproj --no-restore")),
        file("examples/Examples.csproj",format!("<Project Sdk=\"Microsoft.NET.Sdk\"><PropertyGroup><TargetFramework>net8.0</TargetFramework><OutputType>Exe</OutputType><LangVersion>12.0</LangVersion><Nullable>enable</Nullable><ImplicitUsings>enable</ImplicitUsings><TreatWarningsAsErrors>true</TreatWarningsAsErrors></PropertyGroup><ItemGroup><PackageReference Include=\"{}\" Version=\"[{}]\" /></ItemGroup></Project>",xml(&plan.config.name),xml(&plan.config.version))),file("examples/Program.cs",program),file("examples/Quickstart.cs",quickstart),
        file("examples/native-fixtures.json",json!({"format":"suspect-csharp-native-fixtures-v1","binary":{"origin":"explicit-native-fixture","bytes":[0,255,42],"policy":"truncate to declared byte limit; never JSON null"},"credentials":{"origin":"application-owned sample fixture","token":"example-token"}}).to_string())];
    if let Some(environment) = plan.credential_env() {
        files.push(file(
            "credential-env.json",
            serde_json::to_string_pretty(environment).unwrap(),
        ));
    }
    files
}
fn media_record(m: &PlannedMedia) -> Value {
    json!({"variant":m.variant_name,"type":m.native_type,"source":m.wire.source(),"mediaType":m.wire.media_type(),"representation":m.wire.representation(),"parts":m.parts.as_ref().map(|p|json!({"type":p.native_type,"fields":p.fields.iter().map(|f|json!({"name":f.property_name,"type":f.native_type,"valueType":f.value_type,"partType":f.wrapper_type,"headerType":f.header_type,"wire":f.wire})).collect::<Vec<_>>()})),
        "positional":m.positional.as_ref().map(|p|json!({"type":p.native_type,"schema":p.schema,"minItems":p.min_items,"maxItems":p.max_items,"closedAfterPrefix":p.items.is_none(),"prefix":p.prefix.iter().map(part_record).collect::<Vec<_>>(),"items":p.items.as_ref().map(|p|part_record(p))}))})
}
fn part_record(p: &PlannedPart) -> Value {
    json!({"name":p.property_name,"type":p.native_type,"valueType":p.value_type,"partType":p.wrapper_type,"headerType":p.header_type,"wire":p.wire})
}
fn readme(plan: &SdkPlan, quickstart: &str) -> String {
    format!(
        "# {} {}\n\nA source-selected .NET 8/C# 12 SDK. Metadata is in `http-manifest.json`; browsable native documentation is `docs/index.html`. Native compiler XML docs ship with the DLL.\n\n```sh\ndotnet pack -c Release -o packages\ndotnet restore examples/Examples.csproj --source packages\ndotnet run --project examples/Examples.csproj --no-restore\n```\n\n## First request\n\nThis exact native-constructor recipe is packaged and executed against an installed package. Example values retain declared/synthesized origins; byte samples are explicitly labeled native fixtures.\n\n```csharp\n{quickstart}```\n\n## Credentials and servers\n\nCredentials are source-allocated properties. Bearer strings, `BasicCredential`, header/query/cookie API keys and `AuthorizationProvider` hooks attach only to a selected source security alternative. `RequestOptions.SecurityAlternative` chooses an OR alternative explicitly; otherwise the first complete alternative is used. AND requirements must all be supplied. Scopes/roles and OAuth/OIDC URLs are metadata passed to hooks. Hooks return `AuthorizationValue` with an explicit scheme: the SDK never discovers, acquires or refreshes tokens or infers bearer types. Basic is an explicit ASCII wire profile.\n\n`ClientOptions` and `RequestOptions` support `ServerUrl`, source `ServerIndex`, `ServerVariables` and `DocumentUrl`. Relative servers in local files require an explicit HTTP(S) document base. Unknown variables, invalid enums and malformed URLs fail before sending. `Client.Operations` exposes source server metadata.\n\n## Native data and response dispatch\n\n`Optional<T>` defaults to absent; `Optional<T?>.Present(null)` is present JSON null. `JsonNumber`/`JsonInteger` preserve tokens and mathematical values. Use generated `Codecs`; encoding validates all current mutable values. Sealed union arms validate both their payload and their parent.\n\nSingle-success operations return a direct result. Status ranges/defaults preserve the actual status and success classification. Exact status beats range then default; media is chosen only inside that response. Concrete media beats type wildcards then `*/*`, with matching declared parameters breaking ties. No sniffing or fallback to a less-specific status occurs.\n\nBody alternatives are sealed native classes. JSON, UTF-8 text, `byte[]`, named form/multipart records, and `HttpNoContent` are distinct. HEAD/1xx/204/304 suppress body decoding; missing response content retains bounded bytes. Required typed headers are checked; Links remain inert `ResponseLinkInfo` metadata. Wildcard request media require a concrete Content-Type and cannot bypass a more-specific schema.\n\n## Parts and streaming\n\nNamed multipart records use native part values and explicit filename/media/typed header properties. Filenames are bounded ASCII metadata, never filesystem reads. Form/part structural requiredness, extras and cardinalities are checked separately from item codecs. Binary bytes never enter a JSON validator as null. Positional/streamed multipart and undefined inverse styled response groupings are declined.\n\nA sequential response's `Data` contains `HttpStream<T>` (or a typed media arm containing it). Use `await foreach` and dispose the result or stream if it is never enumerated. Early iteration disposal closes the response. JSON lines validate each actual JSON item. OAS 3.2 SSE validates the HTML-framed event envelope; `data` stays a string, ids persist, multiline data is joined and numeric retry is metadata. No JSON-in-data, `[DONE]`, retry, reconnection or pagination behavior is inferred.\n\n## Failure and lifetime policy\n\nTask methods accept `CancellationToken` and `RequestOptions`. Cancellation remains `OperationCanceledException`; whole-call timeout is `SdkErrorKind.Timeout`, including response streaming. `SdkException` distinguishes request/auth/transport/response/resource failures. Declared API error classes expose source-typed `Data`. Captures are bounded copies excluded from default exception formatting.\n\nThe default HttpClient disables cookies, redirects, proxy discovery, decompression and drain-after-close. Injected clients remain caller-owned and must have empty DefaultRequestHeaders; their handler policies are explicitly application-controlled. No SDK retries are performed.\n\nDefault ceilings: 8 MiB body/total stream, 1 MiB part/item, 64 KiB headers, 4096-byte capture, 30-second whole-call lifetime, 128 JSON/conversion depth, 4096-byte number operands and finite shared validation/conversion work. Options may lower transport limits. Cleanup preserves primary failures; late completion after cancellation is observed and disposed.\n",
        plan.config.name, plan.config.version
    ).replace("Positional/streamed multipart", "Streamed multipart") + "\n## Finite positional multipart\n\nOrdered native records expose `Item1`, `Item2`, … and a typed `Items` list when the source permits a tail. Optional prefix values default to absent; later values cannot skip a missing earlier slot. `minItems`, `maxItems`, closed tails and false-prefix barriers are checked before transport. JSON/text item codecs and raw byte policies remain separate. Positional MIME retains source Content-Disposition headers and permits absent/headerless parts where their declared media allows them. Undefined positional RFC6570 name expansion is a located refusal.\n" + scoped_guide(plan) + "\n## Physical document server bases\n\nRelative servers resolve against ServerInfo.DocumentUrl, the effective physical retrieval document containing the declaration. An implicit default belongs to the entry document; an explicit empty override belongs to its declaring document. Requested redirect aliases and logical ResourceInfo addresses remain separate metadata. Local files require an explicit HTTP(S) DocumentUrl override. ServerInfo.ResolveUrl uses the same RFC 3986 resolver as native requests, preserving encoded dots, slashes and path case. Literal dot segments are removed without decoding percent escapes.\n\nClient/per-call ServerUrl overrides are absolute; DocumentUrl explicitly overrides only relative resolution. OAuth/OIDC hooks receive the selected CredentialContext.ServerUrl and EffectiveServer base classification; their raw endpoint strings are not rebased to logical resource names or fetched by the SDK.\n"
}
fn scoped_guide(plan: &SdkPlan) -> &'static str {
    if plan.program.version == suspect_schema::OwnedProgram::V3_VERSION {
        return "\n## Resource-scoped source validation\n\nThis package explicitly compiles suspect.validation.experimental.v3 / oas31-jsonschema202012-resources-dynamic. It retains v2 conditional, dependency, contains, pattern/name and unevaluated instructions, with fresh annotation scopes and noninvertible evaluation failures. DynamicRef selects the outermost actually entered matching resource; entering a nested schema enters its indexed resource without evaluating the resource root. Pointer, empty-fragment and static-anchor fallbacks stay static. Unentered candidates are inert and every return/trial restores scope. Cycle identity includes the exact ordered resource context.\n\nDeclared fields retain native types, requiredness, Optional presence and exact numeric values. Dynamic-reference leaves and context-sensitive unions use JsonElement carriers: a standalone branch trial would lose its enclosing resource scope. Context-sensitive null domains are represented conservatively; the complete root codec validates their exact membership on decode and every mutable encode. No static fallback or candidate declaration is substituted as a body input.\n\nThe embedded validation-program.json and docs/reference.json retain physical resource/declaration sources, canonical/base/alias URIs and aligned node scopes. Physical source IDs never become logical URIs. Logical metadata is inert; the SDK performs no schema acquisition. API servers independently use their physical retrieval document bases.\n\nNative limits remain 128 schema/conversion depth, 4096 numeric operand bytes, 8 MiB input/output and checked shared work/equality budgets. Evaluation charges each new distinct resource and every scanned resource/binding in addition to v2 instruction/annotation costs. Native program admission checks the complete v3 envelope, scope alignment, physical containment, URI/alias identity, original declaration and binding sources, finite targets and initial-resource agreement.\n";
    }
    if plan.program.version != suspect_schema::OwnedProgram::V2_VERSION {
        return "";
    }
    "\n## Scoped source validation\n\nThis package executes the checked v2 static-applicator program: if/then/else, dependentRequired, dependentSchemas, contains with exact min/max counts, patternProperties, pattern-aware additionalProperties, propertyNames, unevaluatedProperties and unevaluatedItems. Each child starts a fresh evaluated-property/item scope; only successful contributions propagate. Trials share work/equality/numeric/depth budgets and never invert evaluation failures.\n\nDeclared fields keep their native types, requiredness and Optional/null states. Patterned extras remain JSON values even when additionalProperties is false; every matching source schema still applies. JsonElement carriers and heterogeneous prefix arrays use complete source codecs on decode and encode. Models are mutable: use Codecs or Client calls to validate current values, including mutations to Extra and lists. Constructing a record or JSON carrier alone does not prove its conditional constraints.\n\nEvaluation reports the first source-located mismatch, but keeps evaluating to preserve later failures. Object visits and annotation merges use decoded Unicode-scalar key order without normalization. Counts and exact numeric tokens never pass through floating point. Native limits are 128 schema/conversion depth, 4096 numeric operand bytes, finite input/output bytes and checked shared work/equality budgets. Dynamic resource scope remains a located admission refusal.\n"
}
fn entries<'a>(plan: &'a SdkPlan, op: &PlannedOperation) -> &'a [ExampleEntry] {
    plan.examples
        .operations()
        .iter()
        .find(|e| e.source == op.source)
        .map_or(&[], |e| &e.entries)
}
fn expression(plan: &SdkPlan, op: &PlannedOperation, index: usize) -> String {
    super::samples::render(&plan.samples[&(op.source.clone(), index)], &plan.models)
}
fn bytes_fixture(max: u64) -> Vec<u8> {
    [0_u8, 255, 42]
        .into_iter()
        .take(max.min(3) as usize)
        .collect()
}
fn byte_expr(bytes: &[u8]) -> String {
    format!(
        "new byte[] {{ {} }}",
        bytes
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
            .join(", ")
    )
}
fn part_expr(plan: &SdkPlan, op: &PlannedOperation, p: &PlannedPart) -> Option<String> {
    let value = match p.wire.representation() {
        p::PartRepresentation::Binary { bytes } => byte_expr(&bytes_fixture(bytes.max_bytes())),
        _ => {
            let (index, _) = entries(plan, op).iter().enumerate().find(|(_, e)| {
                matches!(e.role, ExampleRole::RequestPart { .. })
                    && e.container == *p.wire.source().use_site().source()
            })?;
            expression(plan, op, index)
        }
    };
    let item = if let Some(wrapper) = &p.wrapper_type {
        let mut fields = vec![format!("Value = {value}")];
        if p.wire.content_types().len() > 1
            || p.wire
                .content_types()
                .first()
                .is_some_and(|m| !matches!(m.range(), p::MediaRange::Concrete { .. }))
        {
            fields.push(format!(
                "ContentType = {}",
                quote(&concrete(&p.wire.content_types()[0]))
            ));
        }
        if let Some(h) = &p.header_type {
            let mut headers = Vec::new();
            for header in &p.headers {
                if let Some((i, _)) = entries(plan, op).iter().enumerate().find(|(_, e)| {
                    matches!(e.role, ExampleRole::RequestPartHeader { .. })
                        && e.container == *header.wire.source().use_site().source()
                }) {
                    headers.push(format!(
                        "{} = {}",
                        header.property_name,
                        expression(plan, op, i)
                    ));
                } else if header.wire.required() {
                    return None;
                }
            }
            fields.push(format!("Headers = new {h} {{ {} }}", headers.join(", ")));
        }
        format!("new {wrapper} {{ {} }}", fields.join(", "))
    } else {
        value
    };
    if p.wire.multiplicity() == p::PartMultiplicity::RepeatedArrayItems {
        let count = p.wire.min_items().map_or(1, |v| (*v.value()).max(1));
        if count > 16 {
            return None;
        }
        let ty = p.wrapper_type.as_ref().unwrap_or(&p.value_type);
        Some(format!(
            "new List<{ty}> {{ {} }}",
            vec![item; count as usize].join(", ")
        ))
    } else {
        Some(item)
    }
}
fn parts_expr(plan: &SdkPlan, op: &PlannedOperation, parts: &PlannedParts) -> Option<String> {
    let mut fields = Vec::new();
    for p in &parts.fields {
        if let Some(value) = part_expr(plan, op, p) {
            fields.push(format!("{} = {value}", p.property_name));
        } else if p.wire.required() {
            return None;
        }
    }
    Some(format!(
        "new {} {{ {} }}",
        parts.native_type,
        fields.join(", ")
    ))
}
fn positional_expr(
    plan: &SdkPlan,
    op: &PlannedOperation,
    parts: &PlannedPositionalParts,
) -> Option<String> {
    let count = parts.min_items.as_ref().map_or(0, |n| *n.value());
    if count > 16
        || parts.max_items.as_ref().is_some_and(|n| count > *n.value())
        || parts.items.is_none() && count > parts.prefix.len() as u64
    {
        return None;
    }
    let mut fields = Vec::new();
    for part in parts.prefix.iter().take(count as usize) {
        fields.push(format!(
            "{} = {}",
            part.property_name,
            part_expr(plan, op, part)?
        ));
    }
    if count > parts.prefix.len() as u64 {
        let items = parts.items.as_ref()?;
        let value = part_expr(plan, op, items)?;
        let count = count as usize - parts.prefix.len();
        fields.push(format!(
            "Items = new List<{}> {{ {} }}",
            items.native_type,
            vec![value; count].join(", ")
        ));
    }
    Some(format!(
        "new {} {{ {} }}",
        parts.native_type,
        fields.join(", ")
    ))
}
fn request_expr(plan: &SdkPlan, op: &PlannedOperation) -> Option<String> {
    let mut fields = Vec::new();
    for p in &op.parameters {
        if let Some((i, _)) = entries(plan, op).iter().enumerate().find(|(_, e)| {
            matches!(e.role, ExampleRole::Parameter { .. })
                && e.container == p.source
                && p.wire.serialize(&e.value).is_ok()
        }) {
            fields.push(format!("{} = {}", p.property_name, expression(plan, op, i)));
        } else if p.required {
            return None;
        }
    }
    if let Some(body) = &op.body {
        let mut chosen = None;
        for m in &body.media {
            let value = if let Some(parts) = &m.parts {
                parts_expr(plan, op, parts)
            } else if let Some(parts) = &m.positional {
                positional_expr(plan, op, parts)
            } else {
                match m.wire.representation() {
                    p::Representation::Binary { bytes, .. } => {
                        Some(byte_expr(&bytes_fixture(bytes.max_bytes())))
                    }
                    _ => entries(plan, op)
                        .iter()
                        .enumerate()
                        .find(|(_, e)| {
                            matches!(e.role, ExampleRole::RequestBody)
                                && e.container == *m.wire.source().use_site().source()
                        })
                        .map(|(i, _)| expression(plan, op, i)),
                }
            };
            if let Some(value) = value {
                chosen = Some(if body.union {
                    format!(
                        "new {}.{}({value}, {})",
                        body.native_type,
                        m.variant_name,
                        quote(&concrete(m.wire.media_type()))
                    )
                } else if value == "null" && !body.required {
                    format!("Optional<{}>.Present(null)", body.native_type)
                } else {
                    value
                });
                break;
            }
        }
        if let Some(body) = chosen {
            fields.push(format!("Body = {body}"));
        } else if body.required {
            return None;
        }
    }
    Some(format!("new {} {{ {} }}", op.input_type, fields.join(", ")))
}
fn concrete(m: &p::MediaType) -> String {
    match m.range() {
        p::MediaRange::Concrete { .. } => m.declared().into(),
        p::MediaRange::Type { type_name } => format!("{type_name}/x-suspect-fixture"),
        p::MediaRange::Any => "application/octet-stream".into(),
    }
}
type ResponseFixture = (u16, Vec<u8>, Option<String>, Vec<(String, String)>);
fn positional_fixture(
    plan: &SdkPlan,
    op: &PlannedOperation,
    parts: &PlannedPositionalParts,
) -> Option<Vec<u8>> {
    let count = parts.min_items.as_ref().map_or(0, |n| *n.value());
    if count > 16
        || parts.max_items.as_ref().is_some_and(|n| count > *n.value())
        || parts.items.is_none() && count > parts.prefix.len() as u64
    {
        return None;
    }
    let mut output = Vec::new();
    for index in 0..count as usize {
        let part = parts.prefix.get(index).or(parts.items.as_deref())?;
        let value = entries(plan, op).iter().find(|e| {
            matches!(e.role, ExampleRole::ResponsePart { .. })
                && e.container == *part.wire.source().use_site().source()
        });
        let bytes = match part.wire.representation() {
            p::PartRepresentation::Binary { bytes } => bytes_fixture(bytes.max_bytes()),
            p::PartRepresentation::Json { .. } => value?.value.to_string().into_bytes(),
            p::PartRepresentation::Text { .. } => {
                let v = &value?.value;
                v.as_str()
                    .map(str::to_owned)
                    .unwrap_or_else(|| v.to_string())
                    .into_bytes()
            }
            p::PartRepresentation::Style { .. } => return None,
        };
        output.extend_from_slice(b"--example-positional\r\n");
        output.extend_from_slice(
            format!(
                "Content-Type: {}\r\n",
                concrete(part.wire.content_types().first()?)
            )
            .as_bytes(),
        );
        for header in &part.headers {
            let value = entries(plan, op).iter().find(|e| {
                matches!(e.role, ExampleRole::ResponsePartHeader { .. })
                    && e.container == *header.wire.source().use_site().source()
            });
            if let Some(value) = value {
                output.extend_from_slice(
                    format!(
                        "{}: {}\r\n",
                        header.wire.name(),
                        header.wire.serialize(&value.value).ok()?.value()
                    )
                    .as_bytes(),
                );
            } else if header.wire.required() {
                return None;
            }
        }
        output.extend_from_slice(b"\r\n");
        output.extend_from_slice(&bytes);
        output.extend_from_slice(b"\r\n");
    }
    output.extend_from_slice(b"--example-positional--\r\n");
    Some(output)
}
fn response_fixture(
    plan: &SdkPlan,
    op: &PlannedOperation,
    r: &PlannedResponse,
) -> Option<ResponseFixture> {
    let candidate_media = r.media.first().map(|m| concrete(m.wire.media_type()));
    let status = match r.wire.status() {
        p::ResponseStatus::Exact(s) => s,
        _ => (200..300).find(|status| {
            op.wire
                .match_response(*status, candidate_media.as_deref())
                .is_ok_and(|selected| selected.response().source() == r.wire.source())
        })?,
    };
    let mut headers = Vec::new();
    for h in &r.headers {
        if let Some(e) = entries(plan, op).iter().find(|e| {
            matches!(e.role, ExampleRole::ResponseHeader { .. })
                && e.container == *h.wire.source().use_site().source()
        }) {
            let value = h.wire.serialize(&e.value).ok()?;
            headers.push((h.wire.name().into(), value.value().into()));
        } else if h.wire.required() {
            return None;
        }
    }
    if r.always_empty {
        return Some((status, Vec::new(), None, headers));
    }
    if r.media.is_empty() {
        return Some((status, vec![0, 255, 42], None, headers));
    }
    for m in &r.media {
        if let Some(parts) = &m.positional {
            let content_type = format!(
                "{}; boundary=example-positional",
                concrete(m.wire.media_type())
            );
            if op
                .wire
                .match_response(status, Some(&content_type))
                .is_ok_and(|selected| {
                    selected.response().source() == r.wire.source()
                        && selected
                            .media()
                            .is_some_and(|media| media.source() == m.wire.source())
                })
                && let Some(body) = positional_fixture(plan, op, parts)
            {
                return Some((status, body, Some(content_type), headers));
            }
        }
        let body = match m.wire.representation() {
            p::Representation::Binary { bytes, .. } => Some(bytes_fixture(bytes.max_bytes())),
            p::Representation::Json { .. } | p::Representation::Text { .. } => entries(plan, op)
                .iter()
                .find(|e| {
                    matches!(
                        e.role,
                        ExampleRole::Response { .. } | ExampleRole::ResponsePattern { .. }
                    ) && e.container == *m.wire.source().use_site().source()
                })
                .map(|e| {
                    if matches!(m.wire.representation(), p::Representation::Text { .. }) {
                        e.value
                            .as_str()
                            .map(str::to_owned)
                            .unwrap_or_else(|| e.value.to_string())
                            .into_bytes()
                    } else {
                        e.value.to_string().into_bytes()
                    }
                }),
            _ => None,
        };
        if let Some(body) = body {
            let media = concrete(m.wire.media_type());
            if op
                .wire
                .match_response(status, Some(&media))
                .is_ok_and(|selected| {
                    selected.response().source() == r.wire.source()
                        && selected
                            .media()
                            .is_some_and(|chosen| chosen.source() == m.wire.source())
                })
            {
                return Some((status, body, Some(media), headers));
            }
        }
    }
    None
}
fn credentials_expr(plan: &SdkPlan, op: &PlannedOperation, token: &str) -> String {
    let Some(alt) = op.wire.security().alternatives().first() else {
        return "new Credentials()".into();
    };
    let values = alt.requirements().iter().map(|r| {
        let key = plan.credential_key(r);
        let name = &plan.credentials[&key];
        let value = match r.credential() {
            p::CredentialHook::Basic => format!("new BasicCredential(\"example-user\", {token})"),
            p::CredentialHook::OAuth2 { .. } | p::CredentialHook::OpenIdConnect { .. } => "static (context, cancellationToken) => ValueTask.FromResult(new AuthorizationValue(\"Example\", \"example-token\"))".into(),
            _ => token.into(),
        };
        format!("{name} = {value}")
    }).collect::<Vec<_>>();
    format!("new Credentials {{ {} }}", values.join(", "))
}
fn examples(plan: &SdkPlan) -> (String, String) {
    let mut program = format!(
        "#nullable enable\n#pragma warning disable CS0618\nusing {};\nusing System.Net;\nusing System.Text.Json;\n\n",
        plan.config.namespace
    );
    let mut count = 0;
    let mut calls = 0;
    for op in &plan.operations {
        for (i, e) in entries(plan, op).iter().enumerate() {
            let codec = plan.models.codec_name(&e.schema);
            program.push_str(&format!("{} example{count} = {};\n_ = Codecs.Decode{codec}(Codecs.Encode{codec}(example{count}));\n",plan.models.native_type(&e.schema),expression(plan,op,i)));
            count += 1;
        }
    }
    let mut candidates = plan.operations.iter().enumerate().collect::<Vec<_>>();
    candidates.sort_by_key(|(i, op)| (!op.body.as_ref().is_some_and(|b| b.required), *i));
    let mut first = None;
    for (i, op) in candidates {
        let Some(input) = request_expr(plan, op) else {
            continue;
        };
        let Some(fixture) = op
            .responses
            .iter()
            .filter(|r| r.may_succeed())
            .find_map(|r| response_fixture(plan, op, r))
        else {
            continue;
        };
        if first.is_none() {
            first = Some((i, input.clone(), fixture.clone()));
        }
        let (status, bytes, media, headers) = fixture;
        let header_expr = format!(
            "new Dictionary<string,string> {{ {} }}",
            headers
                .iter()
                .map(|(k, v)| format!("[{}] = {}", quote(k), quote(v)))
                .collect::<Vec<_>>()
                .join(", ")
        );
        let options = document_options(op);
        program.push_str(&format!("using (var transport{i} = new HttpClient(new ExampleHandler({status}, {}, {}, {header_expr})))\nusing (var client{i} = new Client({}, {options}, transport{i}))\n{{\n    await using var result{i} = await client{i}.{}({input});\n    if (result{i}.Status != {status}) throw new Exception(\"Example status differs\");\n}}\n",byte_expr(&bytes),media.as_deref().map(quote).unwrap_or("null".into()),credentials_expr(plan,op,"\"example-token\""),op.method_name));
        calls += 1;
    }
    let mut quick = format!(
        "#nullable enable\n#pragma warning disable CS0618\nusing {};\nusing System;\nusing System.Collections.Generic;\nusing System.Net.Http;\nusing System.Text.Json;\nusing System.Threading;\nusing System.Threading.Tasks;\n\npublic static class Quickstart\n{{\n",
        plan.config.namespace
    );
    if let Some((i, input, (status, bytes, media, headers))) = first {
        let op = &plan.operations[i];
        quick.push_str(&format!("    public static async Task<{}> FirstRequestAsync(string token, CancellationToken cancellationToken = default, HttpClient? httpClient = null)\n    {{\n        using var client = new Client({}, {}, httpClient);\n        var input = {input};\n        try {{ return await client.{}(input, cancellationToken: cancellationToken); }}\n",op.result_type,credentials_expr(plan,op,"token"),document_options(op),op.method_name));
        if let Some(r) = op.responses.iter().find(|r| r.may_fail()) {
            quick.push_str(&format!("        catch ({} error) {{ _ = error.Data; Console.Error.WriteLine(error.Response?.Status); throw; }}\n",r.error_type_name));
        }
        quick.push_str("        catch (OperationCanceledException) { throw; }\n        catch (SdkException error) { Console.Error.WriteLine(error.Kind); throw; }\n    }\n");
        let hs = format!(
            "new Dictionary<string,string> {{ {} }}",
            headers
                .iter()
                .map(|(k, v)| format!("[{}] = {}", quote(k), quote(v)))
                .collect::<Vec<_>>()
                .join(", ")
        );
        program.push_str(&format!("using (var transport = new HttpClient(new ExampleHandler({status}, {}, {}, {hs})))\n{{ await using var result = await Quickstart.FirstRequestAsync(\"example-token\", httpClient: transport); if (result.Status != {status}) throw new Exception(\"Quickstart differs\"); }}\n",byte_expr(&bytes),media.as_deref().map(quote).unwrap_or("null".into())));
    }
    quick.push_str("    internal static JsonElement Json(string text) { using var doc = JsonDocument.Parse(text); return doc.RootElement.Clone(); }\n}\n");
    program.push_str(&format!("Console.WriteLine(\"Validated {count} source-bound values; executed {calls} typed protocol operations and available native quickstart. Byte recipes are explicit native fixtures.\");\nsealed class ExampleHandler(int status, byte[] body, string? media, Dictionary<string,string> headers) : HttpMessageHandler\n{{\n    protected override Task<HttpResponseMessage> SendAsync(HttpRequestMessage request, CancellationToken cancellationToken)\n    {{\n        cancellationToken.ThrowIfCancellationRequested();\n        var response = new HttpResponseMessage((HttpStatusCode)status) {{ Content = new ByteArrayContent(body) }};\n        if (media is not null) response.Content.Headers.TryAddWithoutValidation(\"Content-Type\", media);\n        foreach (var header in headers) response.Headers.TryAddWithoutValidation(header.Key, header.Value);\n        return Task.FromResult(response);\n    }}\n}}\n"));
    (program, quick)
}
fn document_options(op: &PlannedOperation) -> &'static str {
    if op.wire.servers().candidates().iter().any(|s| {
        !s.template().starts_with("https://")
            && !s.template().starts_with("http://")
            && !s
                .document_base()
                .source()
                .document()
                .as_str()
                .starts_with("https://")
            && !s
                .document_base()
                .source()
                .document()
                .as_str()
                .starts_with("http://")
    }) {
        "new ClientOptions { DocumentUrl = \"https://example.test/spec/openapi.json\" }"
    } else {
        "null"
    }
}
