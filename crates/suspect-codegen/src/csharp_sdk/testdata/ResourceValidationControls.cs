using System.Text.Json;
using System.Text.Json.Nodes;
using Resources.Csharp;

internal static class ResourceValidationControls
{
    internal static int Run(JsonElement cases)
    {
        var checks = 0; var results = new List<object>();
        void Check(bool value,string message) { if(!value) throw new Exception(message); checks++; }
        var sample = cases.EnumerateArray().Single(v => v.GetProperty("id").GetString()=="only-plain-dynamic-name-overrides");
        var seed = JsonNode.Parse(sample.GetProperty("program").GetRawText())!.AsObject();
        JsonArray Nodes(JsonObject p) => p["nodes"]!.AsArray();
        JsonArray Resources(JsonObject p) => p["resourceContext"]!["resources"]!.AsArray();
        JsonArray Scopes(JsonObject p) => p["resourceContext"]!["nodeScopes"]!.AsArray();
        JsonObject Dynamic(JsonObject p) => Nodes(p).SelectMany(n=>n!["checks"]!.AsArray()).Select(n=>n!.AsObject()).First(n=>(string?)n["op"]=="dynamicRef" && n["anchor"] is not null);
        JsonObject BindingResource(JsonObject p) => Resources(p).Select(r=>r!.AsObject()).First(r=>r["dynamicAnchors"]!.AsArray().Count>0);
        JsonArray Binding(JsonObject p) => BindingResource(p)["dynamicAnchors"]![0]!.AsArray();
        void Bad(string id,Action<JsonObject> change)
        {
            var program = seed.DeepClone().AsObject(); change(program);
            Directory.CreateDirectory("malformed-programs"); File.WriteAllText("malformed-programs/"+id+".json",program.ToJsonString());
            using var document = JsonDocument.Parse(program.ToJsonString()); CodecException? finding = null;
            try { _ = new ValidationSession(document.RootElement); }
            catch (CodecException error) { finding=error; }
            results.Add(new{id,kind=finding?.Kind.ToString(),source=finding?.SchemaSource});
            File.WriteAllText("admission-results.json",JsonSerializer.Serialize(results,new JsonSerializerOptions{WriteIndented=true}));
            Check(finding?.Kind==CodecErrorKind.EvaluationFailure,"Malformed v3 descriptor admitted or misclassified: "+id);
        }
        Bad("v1-envelope",p=>{p["version"]="suspect.validation.experimental.v1";p["profile"]="oas31-jsonschema202012-static";});
        Bad("v2-envelope",p=>{p["version"]="suspect.validation.experimental.v2";p["profile"]="oas31-jsonschema202012-static-applicators";});
        Bad("mixed-profile",p=>p["profile"]="oas31-jsonschema202012-static-applicators");
        Bad("future-version",p=>p["version"]="suspect.validation.experimental.v4");
        Bad("missing-context",p=>p.Remove("resourceContext"));
        Bad("null-context",p=>p["resourceContext"]=null);
        Bad("unknown-context-field",p=>p["resourceContext"]!["fetch"]=true);
        Bad("null-scopes",p=>p["resourceContext"]!["nodeScopes"]=null);
        Bad("short-scope-index",p=>Scopes(p).RemoveAt(0));
        Bad("long-scope-index",p=>Scopes(p).Add(Scopes(p)[0]!.DeepClone()));
        Bad("scope-object",p=>Scopes(p)[0]=new JsonObject());
        Bad("long-scope-tuple",p=>Scopes(p)[0]!.AsArray().Add(0));
        Bad("negative-resource-index",p=>Scopes(p)[0]![0]=-1);
        Bad("unknown-resource-index",p=>Scopes(p)[0]![0]=999999);
        Bad("foreign-schema-root",p=>Scopes(p)[0]![1]!["document"]="urn:foreign");
        Bad("schema-root-not-container",p=>Scopes(p)[0]![1]!["pointer"]="/unrelated");
        Bad("invented-canonical-address",p=>Scopes(p)[0]![2]="urn:invented#child");
        Bad("unknown-resource-kind",p=>Resources(p)[0]!["kind"]="remote");
        Bad("duplicate-resource-source",p=>Resources(p)[1]!["source"]=Resources(p)[0]!["source"]!.DeepClone());
        Bad("bad-source-pointer",p=>Resources(p)[0]!["source"]!["pointer"]="/~2");
        Bad("relative-physical-document",p=>Resources(p)[0]!["source"]!["document"]="relative.json");
        Bad("fragment-physical-document",p=>Resources(p)[0]!["source"]!["document"]="urn:source#pointer");
        Bad("malformed-physical-document",p=>Resources(p)[0]!["source"]!["document"]="urn:bad%2");
        Bad("invalid-canonical-uri",p=>Resources(p)[0]!["canonicalUri"]="https://bad example.test/id");
        Bad("nonempty-schema-id-fragment",p=>BindingResource(p)["canonicalUri"]=(string?)BindingResource(p)["baseUri"]+"#forbidden");
        Bad("mismatched-base",p=>Resources(p)[0]!["baseUri"]="urn:other");
        Bad("fragment-base",p=>Resources(p)[0]!["baseUri"]=(string?)Resources(p)[0]!["baseUri"]+"#");
        Bad("empty-aliases",p=>Resources(p)[0]!["aliases"]=new JsonArray());
        Bad("duplicate-alias",p=>Resources(p)[0]!["aliases"]!.AsArray().Add(Resources(p)[0]!["aliases"]![0]!.DeepClone()));
        Bad("relative-alias",p=>Resources(p)[0]!["aliases"]!.AsArray().Add("relative"));
        Bad("cross-resource-alias",p=>Resources(p)[1]!["aliases"]!.AsArray().Add(Resources(p)[0]!["aliases"]![0]!.DeepClone()));
        Bad("normalized-alias-collision",p=>{Resources(p)[0]!["aliases"]!.AsArray().Add("HTTP://ALIAS.EXAMPLE.test/id#v%61lue");Resources(p)[1]!["aliases"]!.AsArray().Add("http://alias.example.test/id#value");});
        Bad("non-utf8-alias-fragment",p=>Resources(p)[0]!["aliases"]!.AsArray().Add("urn:alias#%FF"));
        Bad("foreign-declaration-source",p=>BindingResource(p)["declarationSource"]!["document"]="urn:foreign");
        Bad("wrong-declaration-keyword",p=>BindingResource(p)["declarationSource"]!["pointer"]=(string?)BindingResource(p)["source"]!["pointer"]+"/$self");
        Bad("undeclared-nested-resource",p=>BindingResource(p)["declarationSource"]=null);
        Bad("duplicate-binding-name",p=>BindingResource(p)["dynamicAnchors"]!.AsArray().Add(Binding(p).DeepClone()));
        Bad("invalid-binding-name",p=>Binding(p)[0]="1bad");
        Bad("unknown-binding-target",p=>Binding(p)[2]=999999);
        Bad("wrong-binding-resource",p=>{var own=(int)Scopes(p)[(int)Binding(p)[2]! ]![0]!;var other=Enumerable.Range(0,Nodes(p).Count).First(i=>(int)Scopes(p)[i]![0]! != own);Binding(p)[2]=other;Binding(p)[1]=Nodes(p)[other]!["source"]!.DeepClone();Binding(p)[1]!["pointer"]=(string?)Binding(p)[1]!["pointer"]+"/$dynamicAnchor";});
        Bad("wrong-anchor-keyword",p=>Binding(p)[1]!["pointer"]=(string?)Nodes(p)[(int)Binding(p)[2]! ]!["source"]!["pointer"]+"/$anchor");
        Bad("short-binding-tuple",p=>Binding(p).RemoveAt(2));
        Bad("wrong-initial-resource",p=>Dynamic(p)["initialResource"]=((int)Dynamic(p)["initialResource"]!+1)%Resources(p).Count);
        Bad("unknown-initial-resource",p=>Dynamic(p)["initialResource"]=999999);
        Bad("unknown-dynamic-target",p=>Dynamic(p)["target"]=999999);
        Bad("unindexed-dynamic-anchor",p=>Dynamic(p)["anchor"]="unknown");
        Bad("boolean-dynamic-anchor",p=>Dynamic(p)["anchor"]=true);
        Bad("wrong-dynamic-keyword-source",p=>Dynamic(p)["source"]!["pointer"]=(string?)Dynamic(p)["source"]!["pointer"]+"/wrong");
        Bad("unknown-instruction",p=>Dynamic(p)["op"]="fetchSchema");
        Bad("unreferenced-resource",p=>{var resource=Resources(p)[0]!.DeepClone();resource!["source"]=new JsonObject{["document"]="urn:unused",["pointer"]=""};resource["kind"]="document";resource["declarationSource"]=null;resource["canonicalUri"]="urn:unused";resource["baseUri"]="urn:unused";resource["aliases"]=new JsonArray("urn:unused");resource["dynamicAnchors"]=new JsonArray();Resources(p).Add(resource);});
        Bad("native-depth-ceiling",p=>p["limits"]!["maxDepth"]=129);
        Bad("native-work-ceiling",p=>p["limits"]!["maxEvaluationSteps"]=1000001);
        Bad("duplicate-selected-root",p=>p["roots"]!.AsArray().Add(p["roots"]![0]!.DeepClone()));
        Bad("unknown-root-target",p=>p["roots"]![0]!["target"]=999999);
        Bad("empty-type-operand",p=>Nodes(p).SelectMany(n=>n!["checks"]!.AsArray()).First(n=>(string?)n!["op"]=="type")!["types"]=new JsonArray());

        ValidationSession owned;
        using(var document=JsonDocument.Parse(seed.ToJsonString())) owned=new ValidationSession(document.RootElement);
        var instance=JsonRuntime.Parse(JsonRuntime.Bytes(sample.GetProperty("instanceJson").GetString()!));
        owned.Check(sample.GetProperty("rootTarget").GetInt32(),instance,""); checks++;
        var fallback=cases.EnumerateArray().Single(v=>v.GetProperty("id").GetString()=="unentered-override-cannot-bind");
        owned.Check(fallback.GetProperty("rootTarget").GetInt32(),JsonRuntime.Parse("\"fallback\""u8),""); checks++;
        try { owned.Check(sample.GetProperty("rootTarget").GetInt32(),JsonRuntime.Parse("{\"dynamic\":\"invalid\"}"u8),"");throw new Exception("Bad dynamic value accepted"); }
        catch(CodecException error) { Check(error.Kind==CodecErrorKind.InvalidValue,"Independent value mismatch"); }
        owned.Check(fallback.GetProperty("rootTarget").GetInt32(),JsonRuntime.Parse("\"fallback\""u8),""); checks++;
        try { owned.Check(int.MaxValue,instance,"");throw new Exception("Unselected root accepted"); }
        catch(CodecException error) { Check(error.Kind==CodecErrorKind.EvaluationFailure,"Unselected root must fail closed"); }
        var cycle=cases.EnumerateArray().Single(v=>v.GetProperty("id").GetString()=="exact-context-nonprogress-is-failure");
        var afterCycle=new ValidationSession(cycle.GetProperty("program"));
        try { afterCycle.Check(cycle.GetProperty("rootTarget").GetInt32(),JsonRuntime.Parse("null"u8),""); throw new Exception("Cycle accepted"); }
        catch(CodecException error) { Check(error.Kind==CodecErrorKind.EvaluationFailure,"Cycle remains noninvertible"); }
        var any=cycle.GetProperty("program").GetProperty("roots").EnumerateArray().Single(r=>r.GetProperty("source").GetProperty("pointer").GetString()=="/components/schemas/Any");
        afterCycle.Check(any.GetProperty("target").GetInt32(),JsonRuntime.Parse("\"ok\""u8),""); checks++;
        File.WriteAllText("control-summary.json",JsonSerializer.Serialize(new{checks,malformed=results.Count,ownership="cloned-descriptor",restoration="success-invalidity-cycle-and-trials"}));
        Console.WriteLine($"RESOURCE CONTROLS PASS: {checks} checks; {results.Count} malformed refusals");
        return checks;
    }
}
