//! Literal independent OAS 3.2 fixtures for the remaining native protocol gates.
use serde_json::{Value, json};

pub fn document() -> Value {
    let reply = json!({"200":{"description":"result","content":{"application/json":{"schema":{"$ref":"#/components/schemas/Reply"}}}}});
    let op = |name: &str| json!({"operationId":name,"responses":reply});
    let mut custom = serde_json::Map::new();
    for (method, name) in [
        ("COPY", "copyResource"),
        ("MiXeD", "mixedMethod"),
        ("x-PING", "extensionMethod"),
        ("get", "lowerGet"),
        ("GeT", "mixedGet"),
        ("head", "lowerHead"),
        ("pOsT", "mixedPost"),
    ] {
        let mut operation = op(name);
        if method == "pOsT" {
            operation["requestBody"] = json!({"required":true,"content":{"application/json":{"schema":{"$ref":"#/components/schemas/Query"}}}});
        }
        custom.insert(method.into(), operation);
    }
    let mixed = json!({"schema":{"type":"array","prefixItems":[{"$ref":"#/components/schemas/Query"},{"maxLength":16},{"type":"string","minLength":1}],"items":{"type":"integer","minimum":0,"maximum":9},"minItems":2,"maxItems":5},
        "prefixEncoding":[{"contentType":"application/json; profile=meta","headers":{"X-Slot":{"$ref":"#/components/headers/Slot"}}},{"contentType":"application/octet-stream"},{"contentType":"text/plain"}],
        "itemEncoding":{"contentType":"text/plain","headers":{"X-Slot":{"$ref":"#/components/headers/Slot"}}}});
    let form = json!({"schema":{"type":"array","prefixItems":[{"type":"string","minLength":1},{"type":"array","items":{"type":"integer"},"minItems":1}],"items":false,"minItems":2,"maxItems":2},
        "prefixEncoding":[{"contentType":"text/plain","headers":{"Content-Disposition":{"$ref":"#/components/headers/Disposition"}}},{"style":"form","explode":false,"contentType":"application/ignored","headers":{"Content-Disposition":{"$ref":"#/components/headers/Disposition"}}}]});
    let empty = json!({"schema":{"type":"array","prefixItems":[],"items":false,"maxItems":0},"prefixEncoding":[]});
    let barrier = json!({"schema":{"type":"array","prefixItems":[{"type":"string"},false,{"type":"object","patternProperties":{"x":{"type":"string"}}}],"items":{"type":"integer"},"maxItems":10},"prefixEncoding":[{"contentType":"text/plain"},{"contentType":"application/octet-stream"},{"contentType":"application/json"}],"itemEncoding":{"contentType":"text/plain"}});
    let extended = json!({"schema":{"type":"array","prefixItems":[{"type":"string"}],"items":{"type":"integer","minimum":0},"minItems":1,"maxItems":5},"prefixEncoding":[{"contentType":"text/plain"},{"contentType":"application/json"},{"contentType":"text/plain"}],"itemEncoding":{"contentType":"text/plain"}});
    let positional_op = |name: &str, media: &str, content: Value| json!({"operationId":name,"requestBody":{"required":true,"content":{media:content}},"responses":{"200":{"description":"ordered parts","content":{media:content}}}});
    let mut value = json!({"openapi":"3.2.0","info":{"title":"Swift remaining standard HTTP capabilities","version":"1"},"servers":[{"url":"https://example.test/base"}],
    "paths":{
        "/case":{"additionalOperations":custom},
        "/query/json/{id}":{"parameters":[{"$ref":"#/components/parameters/Whole"}],"query":{"operationId":"wholeJSON","parameters":[{"name":"id","in":"path","required":true,"schema":{"type":"string"}},{"name":"criteria","in":"querystring","required":true,"content":{"application/json":{"schema":{"$ref":"#/components/schemas/Query"}}}}],"responses":reply}},
        "/query/text":{"get":{"operationId":"wholeText","parameters":[{"name":"text","in":"querystring","content":{"text/plain; charset=utf-8":{"schema":{"type":"string"}}}}],"responses":reply}},
        "/query/form/{id}":{"get":{"operationId":"wholeForm","parameters":[{"name":"id","in":"path","required":true,"schema":{"type":"string"}},{"name":"fields","in":"querystring","required":true,"content":{"application/x-www-form-urlencoded":{"schema":{"$ref":"#/components/schemas/FormFields"},"encoding":{"tags":{"style":"form","explode":true}}}}}],"responses":reply}},
        "/query/large":{"get":{"operationId":"largeForm","parameters":[{"name":"fields","in":"querystring","required":true,"content":{"application/x-www-form-urlencoded":{"schema":{"type":"object","properties":{},"additionalProperties":false}}}}],"responses":reply}},
        "/ordered":{"post":positional_op("sendOrdered","multipart/mixed",mixed)},
        "/ordered-form":{"post":positional_op("sendOrderedForm","multipart/form-data",form)},
        "/empty":{"post":positional_op("emptyParts","multipart/mixed",empty)},
        "/barrier":{"post":positional_op("barrier","multipart/mixed",barrier)},
        "/extended":{"post":positional_op("extendedPrefix","multipart/mixed",extended)},
        "/reset":{"additionalOperations":{"get":{"operationId":"resetContent","responses":{"205":{"description":"no content","headers":{"X-Reset":{"required":true,"schema":{"type":"integer"}}},"content":{"application/json":{"schema":false}}}}}}},
        "/exact/{mode}":{"additionalOperations":{"get":{"operationId":"exactTransport","parameters":[{"name":"mode","in":"path","required":true,"schema":{"type":"string"}}],"responses":reply}}},
        "/exact-events/{mode}":{"additionalOperations":{"get":{"operationId":"exactEvents","parameters":[{"name":"mode","in":"path","required":true,"schema":{"type":"string"}}],"responses":{"200":{"description":"standard event envelopes","content":{"text/event-stream":{"itemSchema":{"type":"object","properties":{"data":{"type":"string"}},"required":["data"],"additionalProperties":false}}}}}}}}
    },
    "components":{
        "schemas":{
            "Reply":{"type":"object","properties":{"value":{"type":"string"}},"required":["value"],"additionalProperties":false},
            "Query":{"type":"object","properties":{"q":{"type":"string","minLength":1},"exact":{"type":"number"}},"required":["q"],"additionalProperties":false},
            "FormFields":{"type":"object","properties":{"bar":{"type":"boolean"},"foo":{"type":"string"},"tags":{"type":"array","items":{"type":"string"},"minItems":1,"maxItems":3},"meta":{"$ref":"#/components/schemas/Query"}},"required":["bar","foo"],"additionalProperties":{"type":"integer"},"minProperties":2,"maxProperties":6}
        },
        "headers":{"Slot":{"required":true,"schema":{"type":"integer","minimum":1}},"Disposition":{"required":true,"schema":{"type":"string","minLength":1}}},
        "parameters":{"Whole":{"name":"criteria","in":"querystring","content":{"text/plain":{"schema":{"type":"string"}}}}}
    }});
    value["paths"]["/query/large"]["get"]["parameters"][0]["content"]["application/x-www-form-urlencoded"]
        ["schema"]["properties"] =
        json!({"_".repeat(4096):{"type":"array","items":{"type":"string"}}});
    value
}
