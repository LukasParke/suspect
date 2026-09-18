"""Read allocated native names from CLI-emitted JSON descriptors, never source text."""

from __future__ import annotations

from pathlib import Path
from typing import Any

from evidence import read_json, require, sha


def one(values: list[Any], label: str) -> Any:
    require(len(values) == 1, f"expected exactly one {label}, found {len(values)}")
    return values[0]


def identity(value: Any) -> tuple[str, str]:
    if isinstance(value, str):
        document, separator, pointer = value.partition("#")
        require(separator and pointer.startswith("/"), "invalid source identity")
        return document, pointer
    require(isinstance(value, dict), "missing source identity")
    if "source" in value:
        return identity(value["source"])
    require(isinstance(value.get("document"), str) and isinstance(value.get("pointer"), str), "invalid source address")
    return value["document"], value["pointer"]


def identifier(value: Any, *, separators: tuple[str, ...] = ()) -> str:
    require(isinstance(value, str) and value, f"missing native identifier: {value!r}")
    parts = [value]
    for separator in separators:
        parts = [p for part in parts for p in part.split(separator)]
    require(all(p and (p[0].isalpha() or p[0] == "_") and all(c.isascii() and (c.isalnum() or c == "_") for c in p)
                for p in parts), f"unsupported native identifier: {value!r}")
    return value


def status_200(response: dict[str, Any]) -> bool:
    value = response.get("status", response.get("pattern"))
    return value == 200 or value == "200" or value == {"kind": "exact", "value": 200} or value == {"Exact": 200}


def load_binding(package: Path, target: dict[str, Any]) -> dict[str, Any]:
    language = target["language"]
    import_name = target.get("import_name")
    paths = {
        "typescript": "http-manifest.json",
        "rust": "http-manifest.json",
        "python": f"src/{import_name}/http-manifest.json",
        "go": "http-manifest.json",
        "swift": "sdk-manifest.json",
        "java": f"src/main/resources/{str(import_name).replace('.', '/')}/sdk-manifest.json",
        "csharp": "http-manifest.json",
        "kotlin": "docs/symbols.json",
        "ruby": "source-map.json",
        "php": "docs/coverage.json",
        "dart": "sdk-manifest.json",
        "cpp": "sdk-manifest.json",
    }
    used = [package / paths[language]]
    metadata = read_json(used[0])
    operation = one([op for op in metadata["operations"]
                     if op.get("operationId", op.get("operation_id")) == "getCredits"], "getCredits binding")
    document, pointer = identity(operation)
    # This fixed wire fixture is tied to this actual operation and response, not
    # to arbitrary source names that happen to include the word "credits".
    require(pointer == "/paths/~1credits/get", "getCredits binding is not the independent fixture operation")
    response_source = (document, pointer + "/responses/200/content/application~1json/schema")
    result: dict[str, Any] = {"language": language, "operationSource": {"document": document, "pointer": pointer},
                              "responseSource": {"document": response_source[0], "pointer": response_source[1]},
                              "packageName": target["package_name"], "importName": import_name}
    method_key = {"typescript": "export", "rust": "function", "go": "nativeMethod", "csharp": "methodName"}.get(language, "method")
    result["method"] = identifier(operation[method_key])
    if "path" in operation:
        require(operation["path"] == "/credits", "getCredits route differs from independent fixture")

    if language in ("typescript", "python", "go", "rust"):
        response = one([r for r in operation["responses"] if status_200(r)
                        and r.get("mediaType", "application/json") == "application/json"], "200 JSON response")
        result["model"] = identifier(response["model"])
        result["codec"] = identifier(response.get("codec", result["model"] + "Codec"))
        if language == "go":
            result["codec"] = result["model"]
            result["inputConstructor"] = identifier(operation["inputConstructor"])
            result["responseType"] = identifier(response["nativeType"])
            result["credential"] = identifier(operation["security"]["constructor"])
        if language == "rust":
            result["module"] = identifier(operation["module"])
            result["input"] = identifier(operation["inputType"])
            result["success"] = identifier(operation.get("successType", result["input"] + "Success"))
            result["variant"] = identifier(response.get("variant", "Status200"))
            constructors = operation.get("credential", {}).get("constructors", [])
            result["credential"] = identifier(one([c["name"] for c in constructors if c["kind"] == "bearer"], "bearer constructor")) if constructors else "api_key"
    elif language == "csharp":
        used.append(package / "docs/reference.json")
        reference = read_json(used[-1])
        result["model"] = identifier(one([s["name"] for s in reference["symbols"]
                                           if s["kind"] == "type" and identity(s) == response_source], "C# response model"))
        codec = one([s["name"] for s in reference["symbols"]
                     if s["kind"] == "codec" and identity(s) == response_source], "C# response codec")
        require(codec.startswith("Codecs.Decode"), "unmaintained C# codec interface")
        result["decode"] = identifier(codec.removeprefix("Codecs."))
        result["encode"] = "Encode" + result["decode"].removeprefix("Decode")
        result["credential"] = identifier(one([c["property"] for c in metadata["credentials"] if c["key"] == "apiKey"], "C# bearer property"))
    else:
        models = metadata.get("models", metadata.get("schemas"))
        if language == "cpp":
            used.append(package / "docs/coverage.json")
            coverage = read_json(used[-1])
            models = coverage["models"]
            native_op = one([op for op in coverage["operations"] if op["id"] == "getCredits"], "C++ input binding")
            result["input"] = identifier(native_op["input"])
        model = one([m for m in models if identity(m) == response_source], f"{language} response model")
        result["model"] = identifier(model.get("type", model.get("name")), separators=("::", "."))
        if language == "java":
            result["client"] = identifier(metadata["client"])
            result["codecHolder"] = identifier(model["codec"]["holder"])
            result["codecField"] = identifier(model["codec"]["field"])
        elif language == "php":
            result["decode"] = identifier(model["codecs"]["decode"])
            result["encode"] = identifier(model["codecs"]["encode"])
        elif language == "ruby":
            result["model"] = identifier(model["name"])
            result["codec"] = result["model"]
            result["require"] = metadata["package"]["require"]
        else:
            result["codec"] = identifier(model["codec"])
        if language == "swift":
            result["input"] = identifier(operation["input"])
        if language == "kotlin":
            result["result"] = identifier(operation["result"])
            response = one([r for r in operation["responses"] if status_200(r)], "Kotlin response variant")
            result["variant"] = identifier(response.get("constructor", "Status200"), separators=(".",))
            result["directData"] = bool(operation.get("resultDataType"))
    if import_name:
        identifier(import_name, separators=(".", "\\", "::"))
    result["metadata"] = [{"path": p.relative_to(package).as_posix(), "sha256": sha(p)} for p in used]
    return result
