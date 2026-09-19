//! Auditable compile-time asset closure for each default native profile.
//!
//! Paths in reports are relative to suspect-codegen/src. Each list includes
//! model/JSON/validation/codec/HTTP planning and emission, embedded code/docs,
//! and native package/toolchain templates. Shared validation policy defaults
//! are included; other canonical/schema compiler internals and external
//! toolchains are outside this fingerprint. Native conformance gates remain
//! necessary alongside the recorded generator version.

use std::collections::BTreeMap;

use sha2::{Digest, Sha256};

use super::native::RuntimeProvenance;

type Assets = &'static [(&'static str, &'static [u8])];

macro_rules! assets {
    ($($path:literal),* $(,)?) => {
        &[$(($path, include_bytes!(concat!("../", $path)) as &'static [u8])),*]
    };
}

const COMMON: Assets = assets!(
    "backend.rs",
    "backend/options.rs",
    "credential_env.rs",
    "http_contract.rs",
    "examples.rs",
    "examples/aggregate.rs",
    "examples/protocol.rs",
    "http_examples.rs",
    "model_naming.rs",
    "schema_view.rs",
    "../../suspect-schema/src/config.rs",
);

const TYPESCRIPT: Assets = assets!(
    "typescript.rs",
    "typescript/additional_properties.rs",
    "typescript/intersections.rs",
    "typescript/directional.rs",
    "typescript/json.rs",
    "typescript/json.ts",
    "typescript/validation.rs",
    "typescript/validation.ts",
    "typescript/pattern.ts",
    "typescript/codecs.rs",
    "typescript/codecs.ts",
    "typescript/http.rs",
    "typescript/package.rs",
    "../tools/typescript-docs/package-lock.json",
    "../../suspect-gen/src/filters.rs",
);

const RUST: Assets = assets!(
    "rust_models.rs",
    "rust_models/applicators.rs",
    "rust_models/runtime.rs",
    "rust_codecs.rs",
    "rust_codecs/emit.rs",
    "rust_codecs/json_runtime.rs",
    "rust_codecs/model_runtime.rs",
    "rust_validation.rs",
    "rust_validation/runtime.rs",
    "rust_validation/runtime_v2.rs",
    "rust_validation/number.rs",
    "rust_validation/pattern.rs",
    "rust_http.rs",
    "rust_http/emit.rs",
    "rust_http/runtime.rs",
    "rust_http/reqwest.rs",
);

const PYTHON: Assets = assets!(
    // Python's allocated model/operation names use these shared naming helpers.
    "rust_models.rs",
    "python_models.rs",
    "python_json.rs",
    "python_json/runtime.py",
    "python_validation.rs",
    "python_validation/runtime.py",
    "python_validation/runtime_v2.py",
    "python_validation/guard.py",
    "python_validation/number.py",
    "python_codecs.rs",
    "python_codecs/emit.rs",
    "python_codecs/runtime.py",
    "python_http.rs",
    "python_http/docs.rs",
    "python_http/native_examples.rs",
    "python_http/emit.rs",
    "python_http/runtime.py",
);

const GO: Assets = assets!(
    "rust_models.rs",
    // Go reuses Rust's CodecConfig and JsonLimits defaults.
    "rust_codecs.rs",
    "go_models.rs",
    "go_json.rs",
    "go_json/runtime.go",
    "go_validation.rs",
    "go_validation/runtime.go",
    "go_validation/number.go",
    "go_validation/pattern.go",
    "go_validation/scoped.go",
    "go_validation/scoped_pattern.go",
    "go_validation/resources.go",
    "go_codecs.rs",
    "go_codecs/emit.rs",
    "go_codecs/runtime.go",
    "go_http.rs",
    "go_http/planning.rs",
    "go_http/descriptors.rs",
    "go_http/docs.rs",
    "go_http/native_examples.rs",
    "go_http/emit.rs",
    "go_http/credential_env.rs",
    "go_http/runtime.go",
    "go_http/parameters.go",
    "go_http/media.go",
    "go_http/security.go",
    "go_http/parts.go",
    "go_http/stream.go",
);

const SWIFT: Assets = assets!(
    "swift_sdk.rs",
    "swift_sdk/models.rs",
    "swift_sdk/validation.rs",
    "swift_sdk/emit.rs",
    "swift_sdk/json.swift",
    "swift_sdk/number.swift",
    "swift_sdk/presence.swift",
    "swift_sdk/validation.swift",
    "swift_sdk/codecs.swift",
    "swift_sdk/transport.swift",
    "swift_sdk/protocol.rs",
    "swift_sdk/protocol_emit.rs",
    "swift_sdk/protocol_examples.rs",
    "swift_sdk/protocol_metadata.rs",
    "swift_sdk/protocol_positional.rs",
    "swift_sdk/protocol_query.rs",
    "swift_sdk/protocol_runtime.swift",
    "swift_sdk/protocol_parameters.swift",
    "swift_sdk/protocol_parts.swift",
    "swift_sdk/protocol_stream.swift",
    "swift_sdk/protocol_exact.swift",
);

#[cfg(feature = "ruby-sdk")]
const RUBY: Assets = assets!(
    "ruby_sdk.rs",
    "ruby_sdk/models.rs",
    "ruby_sdk/samples.rs",
    "ruby_sdk/emit.rs",
    "ruby_sdk/json.rb",
    "ruby_sdk/validation.rb",
    "ruby_sdk/codecs.rb",
    "ruby_sdk/http.rb",
    "ruby_sdk/runtime.rbs",
    "rust_models.rs",
);

#[cfg(feature = "csharp-sdk")]
const CSHARP: Assets = assets!(
    "csharp_sdk.rs",
    "csharp_sdk/models.rs",
    "csharp_sdk/validation.rs",
    "csharp_sdk/http.rs",
    "csharp_sdk/samples.rs",
    "csharp_sdk/docs.rs",
    "csharp_sdk/emit.rs",
    "csharp_sdk/JsonRuntime.cs",
    "csharp_sdk/ValidationRuntime.cs",
    "csharp_sdk/HttpRuntime.cs",
    "csharp_sdk/ServerRuntime.cs",
    "rust_models.rs",
    "csharp_sdk/protocol.rs",
    "csharp_sdk/credential_env.rs",
    "csharp_sdk/CredentialEnvironment.cs",
    "csharp_sdk/ProtocolRuntime.cs",
    "csharp_sdk/WireEncoding.cs",
    "csharp_sdk/PartsRuntime.cs",
    "csharp_sdk/StreamRuntime.cs",
    "csharp_sdk/positional.rs",
    "csharp_sdk/PositionalRuntime.cs",
    "csharp_sdk/ScopedValidationRuntime.cs",
    "csharp_sdk/ValidationProgramGuard.cs",
    "csharp_sdk/resources.rs",
    "csharp_sdk/ResourceScope.cs",
    "csharp_sdk/ResourceProgramGuard.cs",
    "csharp_sdk/scoped_examples.rs",
);

#[cfg(feature = "java-sdk")]
const JAVA: Assets = assets!(
    "java_sdk.rs",
    "java_sdk/models.rs",
    "java_sdk/model_emit.rs",
    "java_sdk/http.rs",
    "java_sdk/validation.rs",
    "java_sdk/json_runtime.rs",
    "java_sdk/emit.rs",
    "java_sdk/docs.rs",
    "java_sdk/readme-runtime.md",
    "java_sdk/readme-credential-env.md",
    "java_sdk/JsonRuntime.java",
    "java_sdk/ModelCodec.java",
    "java_sdk/Validation.java",
    "java_sdk/ValidationResources.java",
    "java_sdk/HttpRuntime.java",
    "java_sdk/http_emit.rs",
    "java_sdk/protocol.rs",
    "java_sdk/wire_emit.rs",
    "java_sdk/Bytes.java",
    "java_sdk/NoContent.java",
    "java_sdk/ResponseBody.java",
    "java_sdk/Protocol.java",
    "java_sdk/HttpWire.java",
    "java_sdk/WireValue.java",
    "java_sdk/WireCodec.java",
    "java_sdk/EventStream.java",
    "java_sdk/RequestOptions.java",
    "java_sdk/ExactHttp.java",
    "java_sdk/CodecException.java",
    "java_sdk/Presence.java",
    "java_sdk/Never.java",
    "java_sdk/SdkException.java",
    "rust_models.rs",
);

#[cfg(feature = "dart-sdk")]
const DART: Assets = assets!(
    "dart_sdk.rs",
    "dart_sdk/models.rs",
    "dart_sdk/validation.rs",
    "dart_sdk/emit.rs",
    "dart_sdk/README.md",
    "dart_sdk/SCOPED-VALIDATION.md",
    "dart_sdk/RESOURCE-VALIDATION.md",
    "dart_sdk/CREDENTIAL-ENV.md",
    "dart_sdk/environment.rs",
    "dart_sdk/environment.dart",
    "dart_sdk/environment_io.dart",
    "dart_sdk/environment_stub.dart",
    "dart_sdk/json.dart",
    "dart_sdk/validation.dart",
    "dart_sdk/validation_v2.dart",
    "dart_sdk/validation_v3.dart",
    "dart_sdk/codec.dart",
    "dart_sdk/transport.dart",
    "dart_sdk/url.dart",
    "dart_sdk/io_transport.dart",
    "dart_sdk/protocol.rs",
    "dart_sdk/http_emit.rs",
    "dart_sdk/wire.dart",
    "dart_sdk/auth.dart",
    "dart_sdk/forms.dart",
    "dart_sdk/framing.dart",
    "dart_sdk/io_exact.dart",
);

#[cfg(feature = "cpp-sdk")]
const CPP: Assets = assets!(
    "cpp_sdk.rs",
    "cpp_sdk/models.rs",
    "cpp_sdk/emit.rs",
    "cpp_sdk/emit/http.rs",
    "cpp_sdk/emit/credential_env.rs",
    "cpp_sdk/emit/native_example.rs",
    "cpp_sdk/runtime.hpp",
    "cpp_sdk/runtime.cpp",
    "cpp_sdk/number.hpp",
    "cpp_sdk/validation.cpp",
    "cpp_sdk/validation_v2.cpp",
    "cpp_sdk/validation_v3.cpp",
    "cpp_sdk/scoped_examples.rs",
    "cpp_sdk/http.hpp",
    "cpp_sdk/http.cpp",
    "cpp_sdk/curl_transport.cpp",
    "cpp_sdk/CMakeLists.txt",
    "cpp_sdk/config.cmake.in",
    "cpp_sdk/Doxyfile",
    "cpp_sdk/guide.md",
    "cpp_sdk/protocol.rs",
    "cpp_sdk/emit/aggregates.rs",
    "cpp_sdk/emit/wire.rs",
    "cpp_sdk/protocol.hpp",
    "cpp_sdk/wire.cpp",
    "cpp_sdk/payload.cpp",
    "cpp_sdk/stream.hpp",
    "cpp_sdk/stream.cpp",
);

#[cfg(feature = "kotlin-sdk")]
const KOTLIN: Assets = assets!(
    "kotlin_sdk.rs",
    "kotlin_sdk/models.rs",
    "kotlin_sdk/validation.rs",
    "kotlin_sdk/validation_v3.rs",
    "kotlin_sdk/samples.rs",
    "kotlin_sdk/emit.rs",
    "kotlin_sdk/protocol.rs",
    "kotlin_sdk/environment.rs",
    "kotlin_sdk/CredentialEnvironment.kt",
    "kotlin_sdk/guide-env.md",
    "kotlin_sdk/Json.kt",
    "kotlin_sdk/Validation.kt",
    "kotlin_sdk/ValidationV2.kt",
    "kotlin_sdk/ValidationResources.kt",
    "kotlin_sdk/DocumentServers.kt",
    "kotlin_sdk/Http.kt",
    "kotlin_sdk/guide.md",
    "kotlin_sdk/guide-v2.md",
    "kotlin_sdk/guide-v3.md",
    "kotlin_sdk/pom.xml",
    "kotlin_sdk/rich_emit.rs",
    "kotlin_sdk/Protocol.kt",
    "kotlin_sdk/Payload.kt",
    "kotlin_sdk/Streaming.kt",
);

#[cfg(feature = "php-sdk")]
const PHP: Assets = assets!(
    "php_sdk.rs",
    "php_sdk/models.rs",
    "php_sdk/samples.rs",
    "php_sdk/docs.rs",
    "php_sdk/emit.rs",
    "php_sdk/Number.php",
    "php_sdk/Json.php",
    "php_sdk/Validation.php",
    "php_sdk/ValidationV2.php",
    "php_sdk/ValidationResources.php",
    "php_sdk/Http.php",
    "php_sdk/protocol.rs",
    "php_sdk/protocol_emit.rs",
    "php_sdk/protocol_docs.rs",
    "php_sdk/Protocol.php",
    "php_sdk/Parts.php",
    "php_sdk/Stream.php",
    "php_sdk/Transport.php",
);

#[cfg(feature = "http-protocol")]
const PROTOCOL: Assets = assets!(
    "http_protocol.rs",
    "http_protocol/model.rs",
    "http_protocol/capabilities.rs",
    "http_protocol/planner.rs",
    "http_protocol/shapes.rs",
    "http_protocol/servers.rs",
    "http_protocol/security.rs",
    "http_protocol/parameters.rs",
    "http_protocol/bodies.rs",
    "http_protocol/responses.rs",
    "http_protocol/resource.rs",
    "http_protocol/media.rs",
    "http_protocol/wire.rs",
    "http_protocol/examples.rs",
);

pub(super) fn capture(backend: &str) -> RuntimeProvenance {
    let profile = match backend {
        "typescript-http" => TYPESCRIPT,
        "rust-http" => RUST,
        "python-http" => PYTHON,
        "go-http" => GO,
        "swift-http" | "swift-sdk" => SWIFT,
        #[cfg(feature = "ruby-sdk")]
        "ruby-http" => RUBY,
        #[cfg(feature = "csharp-sdk")]
        "csharp-http" => CSHARP,
        #[cfg(feature = "java-sdk")]
        "java-http" => JAVA,
        #[cfg(feature = "dart-sdk")]
        "dart-http" => DART,
        #[cfg(feature = "cpp-sdk")]
        "cpp-http" => CPP,
        #[cfg(feature = "kotlin-sdk")]
        "kotlin-http" => KOTLIN,
        #[cfg(feature = "php-sdk")]
        "php-http" => PHP,
        _ => &[],
    };
    // Language-owned closures are inputs to the same sorted, length-delimited
    // hash as common assets, including an empty-operation snapshot. Native
    // capture does not need to extend or reframe this identity itself.
    let adapter_assets: Assets = match backend {
        "typescript-http" => crate::typescript::http::source_assets(),
        "rust-http" => crate::rust_http::source_assets(),
        "python-http" => crate::python_http::source_assets(),
        #[cfg(feature = "ruby-sdk")]
        "ruby-http" => crate::ruby_sdk::source_assets(),
        _ => &[],
    };
    #[cfg(feature = "http-protocol")]
    let protocol = PROTOCOL;
    #[cfg(not(feature = "http-protocol"))]
    let protocol: Assets = &[];
    let assets: BTreeMap<_, _> = COMMON
        .iter()
        .chain(profile)
        .chain(adapter_assets)
        .chain(protocol)
        .copied()
        .collect();
    let mut hash = Sha256::new();
    for (name, bytes) in &assets {
        hash.update((name.len() as u64).to_be_bytes());
        hash.update(name.as_bytes());
        hash.update((bytes.len() as u64).to_be_bytes());
        hash.update(bytes);
    }
    RuntimeProvenance {
        generator_version: env!("CARGO_PKG_VERSION").into(),
        runtime_version: None,
        profile: format!("{backend}:default-policy"),
        plan_and_runtime_sha256: format!("{:x}", hash.finalize()),
        fingerprinted_assets: assets.into_keys().map(str::to_owned).collect(),
    }
}
