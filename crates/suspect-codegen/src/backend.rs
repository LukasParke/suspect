//! One canonical backend boundary for planning, packaging and incremental callers.
use crate::{OutFile, http_contract::HttpDiagnostic};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use suspect_ir::contract::{Contract, SourceId};

mod options;
pub use options::GenerationOptions;
pub(crate) use options::*;

macro_rules! backends {
    ($( $(#[$attribute:meta])* $variant:ident => ($name:literal, $directory:literal, $language:literal, $description:literal); )+) => {
        /// Independently verified profiles. Registration requires native acceptance.
        #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
        pub enum Backend {
            $( $(#[$attribute])* #[serde(rename = $name)] $variant, )+
        }
        impl Backend {
            /// The same registry drives configuration, CLI choices and inventories.
            pub const ALL: &'static [Self] = &[$( $(#[$attribute])* Self::$variant, )+];
            pub const fn name(self) -> &'static str {
                match self { $( $(#[$attribute])* Self::$variant => $name, )+ }
            }
            pub const fn artifact_directory(self) -> &'static str {
                match self { $( $(#[$attribute])* Self::$variant => $directory, )+ }
            }
            pub const fn description(self) -> &'static str {
                match self { $( $(#[$attribute])* Self::$variant => $description, )+ }
            }
            pub const fn owner(self) -> &'static str {
                match self { $( $(#[$attribute])* Self::$variant => concat!("suspect-sdk:", $name), )+ }
            }
            /// `ua/v1` language tag used in the attribution header comment.
            pub const fn language_tag(self) -> &'static str {
                match self { $( $(#[$attribute])* Self::$variant => $language, )+ }
            }
        }
    };
}

backends! {
    TypescriptHttp => ("typescript-http", "typescript", "typescript", "Source-selected HTTP operations, exact codecs and ESM packaging");
    RustHttp => ("rust-http", "rust", "rust", "Source-selected native HTTP clients, exact codecs and Cargo packaging");
    PythonHttp => ("python-http", "python", "python", "Source-selected sync/async clients, exact codecs and wheel packaging");
    GoHttp => ("go-http", "go", "go", "Source-selected context-aware clients, exact codecs and Go modules");
    SwiftHttp => ("swift-http", "swift", "swift", "Source-selected async clients, exact codecs, SwiftPM and DocC");
    #[cfg(feature = "ruby-sdk")]
    RubyHttp => ("ruby-http", "ruby", "ruby", "Source-selected keyword clients, exact codecs, gems, RBS and YARD");
    #[cfg(feature = "csharp-sdk")]
    CsharpHttp => ("csharp-http", "csharp", "csharp", "Source-selected Task clients, exact codecs, NuGet and native .NET docs");
    #[cfg(feature = "dart-sdk")]
    DartHttp => ("dart-http", "dart", "dart", "Source-selected Future clients, exact codecs, pub packages and dartdoc");
    #[cfg(feature = "cpp-sdk")]
    CppHttp => ("cpp-http", "cpp", "cpp", "Source-selected C++20 clients, exact codecs, CMake, libcurl and Doxygen");
    #[cfg(feature = "kotlin-sdk")]
    KotlinHttp => ("kotlin-http", "kotlin", "kotlin", "Source-selected coroutine clients, exact codecs, Maven and Dokka");
    #[cfg(feature = "php-sdk")]
    PhpHttp => ("php-http", "php", "php", "Source-selected typed PHP clients, exact codecs, Composer and PHPDoc");
    #[cfg(feature = "java-sdk")]
    JavaHttp => ("java-http", "java", "java", "Source-selected immutable Java clients, exact codecs, CompletableFuture, Maven and Javadoc");
}
/// Package identity is independent from every OpenAPI semantic.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TargetConfig {
    pub backend: Backend,
    pub package_name: String,
    pub package_version: String,
    /// Python import name, Swift module, JVM package, or Ruby/C#/PHP/C++ namespace.
    /// Omission uses the backend's default. Profiles without an independent
    /// native identity reject this option rather than silently ignoring it.
    pub import_name: Option<String>,
}
#[derive(Debug, Clone)]
pub struct BackendDiagnostic {
    pub source: Option<SourceId>,
    pub at: Option<std::ops::Range<usize>>,
    pub code: &'static str,
    pub message: String,
}
impl From<HttpDiagnostic> for BackendDiagnostic {
    fn from(d: HttpDiagnostic) -> Self {
        Self {
            source: Some(d.source),
            at: Some(d.at),
            code: d.code,
            message: d.message,
        }
    }
}
fn package(message: impl ToString) -> Vec<BackendDiagnostic> {
    vec![BackendDiagnostic {
        source: None,
        at: None,
        code: "sdk-package",
        message: message.to_string(),
    }]
}

/// Main registers an adapter here after its targeted native environment proof.
/// Direct native planners remain the interface used to produce that proof.
pub(crate) fn credential_env_admission_error(
    config: &TargetConfig,
    options: &GenerationOptions,
) -> Option<BackendDiagnostic> {
    const VERIFIED: &[Backend] = &[
        Backend::TypescriptHttp,
        Backend::RustHttp,
        Backend::PythonHttp,
        Backend::GoHttp,
        Backend::SwiftHttp,
        #[cfg(feature = "java-sdk")]
        Backend::JavaHttp,
        #[cfg(feature = "php-sdk")]
        Backend::PhpHttp,
        #[cfg(feature = "dart-sdk")]
        Backend::DartHttp,
        #[cfg(feature = "ruby-sdk")]
        Backend::RubyHttp,
        #[cfg(feature = "csharp-sdk")]
        Backend::CsharpHttp,
        #[cfg(feature = "cpp-sdk")]
        Backend::CppHttp,
        #[cfg(feature = "kotlin-sdk")]
        Backend::KotlinHttp,
    ];
    if options.credential_env.is_none() || VERIFIED.contains(&config.backend) {
        return None;
    }
    Some(BackendDiagnostic {
        source: None,
        at: None,
        code: "sdk-credential-env-unavailable",
        message: format!(
            "{} credential_env v1 requires its completed native client-default proof",
            config.backend.name()
        ),
    })
}

pub(crate) fn configuration_error(config: &TargetConfig) -> Option<&'static str> {
    config.import_name.as_ref()?;
    match config.backend {
        Backend::TypescriptHttp => {
            Some("TypeScript import identity comes from package_name; import_name is not supported")
        }
        Backend::RustHttp => {
            Some("Rust crate import identity comes from package_name; import_name is not supported")
        }
        Backend::GoHttp => Some(
            "the canonical Go profile uses package sdk and package_name as its module path; import_name is not supported",
        ),
        #[cfg(feature = "dart-sdk")]
        Backend::DartHttp => {
            Some("Dart library identity comes from package_name; import_name is not supported")
        }
        _ => None,
    }
}

#[cfg(feature = "ruby-sdk")]
pub(crate) fn ruby_package(config: &TargetConfig) -> crate::ruby_sdk::PackageConfig {
    crate::ruby_sdk::PackageConfig {
        name: config.package_name.clone(),
        version: config.package_version.clone(),
        require_name: config.package_name.replace(['-', '.'], "_"),
        namespace: config
            .import_name
            .clone()
            .unwrap_or_else(|| crate::rust_models::pascal(&config.package_name)),
    }
}

#[cfg(feature = "csharp-sdk")]
pub(crate) fn csharp_config(config: &TargetConfig) -> crate::csharp_sdk::SdkConfig {
    crate::csharp_sdk::SdkConfig {
        name: config.package_name.clone(),
        version: config.package_version.clone(),
        namespace: config.import_name.clone().unwrap_or_else(|| {
            config
                .package_name
                .split('.')
                .map(crate::rust_models::pascal)
                .collect::<Vec<_>>()
                .join(".")
        }),
    }
}

#[cfg(feature = "dart-sdk")]
pub(crate) fn dart_config(config: &TargetConfig) -> crate::dart_sdk::DartConfig {
    crate::dart_sdk::DartConfig {
        package: crate::dart_sdk::PackageConfig {
            name: config.package_name.clone(),
            version: config.package_version.clone(),
        },
        ..Default::default()
    }
}

#[cfg(feature = "cpp-sdk")]
pub(crate) fn cpp_config(config: &TargetConfig) -> crate::cpp_sdk::SdkConfig {
    crate::cpp_sdk::SdkConfig {
        name: config.package_name.clone(),
        version: config.package_version.clone(),
        namespace: config
            .import_name
            .clone()
            .unwrap_or_else(|| config.package_name.clone()),
        ..Default::default()
    }
}

#[cfg(feature = "kotlin-sdk")]
pub(crate) fn kotlin_config(
    config: &TargetConfig,
) -> Result<crate::kotlin_sdk::SdkConfig, Vec<BackendDiagnostic>> {
    let (group_id, artifact_id) = config
        .package_name
        .split_once(':')
        .ok_or_else(|| package("Kotlin package_name must be Maven group:artifact coordinates"))?;
    Ok(crate::kotlin_sdk::SdkConfig {
        group_id: group_id.into(),
        artifact_id: artifact_id.into(),
        version: config.package_version.clone(),
        package_name: config
            .import_name
            .clone()
            .unwrap_or_else(|| format!("{group_id}.{}", artifact_id.replace('-', "_"))),
        ..Default::default()
    })
}

#[cfg(feature = "php-sdk")]
pub(crate) fn php_config(config: &TargetConfig) -> crate::php_sdk::PhpConfig {
    crate::php_sdk::PhpConfig {
        package_name: config.package_name.clone(),
        package_version: config.package_version.clone(),
        namespace: config.import_name.clone().unwrap_or_else(|| {
            config
                .package_name
                .split('/')
                .map(crate::rust_models::pascal)
                .collect::<Vec<_>>()
                .join("\\")
        }),
        ..Default::default()
    }
}

#[cfg(feature = "java-sdk")]
pub(crate) fn java_config(
    config: &TargetConfig,
) -> Result<(crate::java_sdk::PackageConfig, crate::java_sdk::MavenConfig), Vec<BackendDiagnostic>>
{
    let (group, artifact) = config
        .package_name
        .split_once(':')
        .filter(|(group, artifact)| {
            !group.is_empty() && !artifact.is_empty() && !artifact.contains(':')
        })
        .ok_or_else(|| package("Java package_name must be Maven group:artifact coordinates"))?;
    Ok((
        crate::java_sdk::PackageConfig {
            package: config.import_name.clone().unwrap_or_else(|| group.into()),
            version: config.package_version.clone(),
            api_name: "Client".into(),
        },
        crate::java_sdk::MavenConfig {
            group_id: Some(group.into()),
            artifact_id: artifact.into(),
            ..Default::default()
        },
    ))
}

/// Plan and emit one target from a caller-owned immutable contract. No input IO,
/// toolchain execution or publishing occurs at this boundary.
pub fn generate(
    contract: Arc<Contract>,
    operations: &[SourceId],
    config: &TargetConfig,
) -> Result<Vec<OutFile>, Vec<BackendDiagnostic>> {
    generate_with_options(contract, operations, config, &GenerationOptions::default())
}

/// Plan with explicit versioned interpretation choices through the same native
/// adapter used by generation, sessions and compatibility. No source is rewritten.
pub fn generate_with_options(
    contract: Arc<Contract>,
    operations: &[SourceId],
    config: &TargetConfig,
    options: &GenerationOptions,
) -> Result<Vec<OutFile>, Vec<BackendDiagnostic>> {
    if let Some(error) = configuration_error(config) {
        return Err(package(error));
    }
    if let Some(error) = credential_env_admission_error(config, options) {
        return Err(vec![error]);
    }
    let attribution = crate::attribution::AttributionDescriptor::plan(
        env!("CARGO_PKG_VERSION"),
        &config.package_name,
        &config.package_version,
        contract.openapi_version(),
        config.backend.language_tag(),
    );
    let attribution = Some(&attribution);
    match config.backend {
        Backend::TypescriptHttp => {
            let plan = crate::typescript::http::plan_http(
                contract,
                operations,
                typescript_options(options, attribution),
            )
            .map_err(|errors| errors.into_iter().map(Into::into).collect::<Vec<_>>())?;
            crate::typescript::package::emit_http(
                &plan,
                &crate::typescript::package::PackageConfig {
                    name: config.package_name.clone(),
                    version: config.package_version.clone(),
                },
            )
            .map_err(package)
        }
        Backend::RustHttp => {
            let plan = crate::rust_http::plan_http_v3(
                contract,
                operations,
                rust_options(options, attribution),
            )
            .map_err(|errors| errors.into_iter().map(Into::into).collect::<Vec<_>>())?;
            crate::rust_http::emit_http(
                &plan,
                &crate::rust_http::PackageConfig {
                    name: config.package_name.clone(),
                    version: config.package_version.clone(),
                },
            )
            .map_err(package)
        }
        Backend::PythonHttp => {
            let plan = crate::python_http::plan_http(
                contract,
                operations,
                python_options(options, attribution),
            )
            .map_err(|errors| errors.into_iter().map(Into::into).collect::<Vec<_>>())?;
            crate::python_http::emit_http(
                &plan,
                &crate::python_http::PackageConfig {
                    name: config.package_name.clone(),
                    version: config.package_version.clone(),
                    import_name: config
                        .import_name
                        .clone()
                        .unwrap_or_else(|| config.package_name.replace('-', "_")),
                },
            )
            .map_err(package)
        }
        Backend::GoHttp => {
            let plan =
                crate::go_http::plan_http(contract, operations, go_options(options, attribution))
                    .map_err(|errors| errors.into_iter().map(Into::into).collect::<Vec<_>>())?;
            crate::go_http::emit_http(
                &plan,
                &crate::go_http::PackageConfig {
                    module_path: config.package_name.clone(),
                    package_name: "sdk".into(),
                    version: config.package_version.clone(),
                },
            )
            .map_err(|errors| package(errors.join("; ")))
        }
        Backend::SwiftHttp => {
            let plan = crate::swift_sdk::plan_sdk(
                contract,
                operations,
                swift_options(options, attribution),
            )
            .map_err(|errors| errors.into_iter().map(Into::into).collect::<Vec<_>>())?;
            crate::swift_sdk::emit_sdk(
                &plan,
                &crate::swift_sdk::PackageConfig {
                    name: config.package_name.clone(),
                    version: config.package_version.clone(),
                    module_name: config
                        .import_name
                        .clone()
                        .unwrap_or_else(|| config.package_name.clone()),
                },
            )
            .map(|files| {
                files
                    .into_iter()
                    .map(|file| OutFile {
                        path: format!("swift/{}", file.path),
                        content: file.content,
                    })
                    .collect()
            })
            .map_err(|errors| package(errors.join("; ")))
        }
        #[cfg(feature = "ruby-sdk")]
        Backend::RubyHttp => {
            let plan =
                crate::ruby_sdk::plan_sdk(contract, operations, ruby_options(options, attribution))
                    .map_err(|errors| errors.into_iter().map(Into::into).collect::<Vec<_>>())?;
            crate::ruby_sdk::emit_sdk(&plan, &ruby_package(config)).map_err(|errors| {
                package(
                    errors
                        .into_iter()
                        .map(|error| error.message)
                        .collect::<Vec<_>>()
                        .join("; "),
                )
            })
        }
        #[cfg(feature = "csharp-sdk")]
        Backend::CsharpHttp => {
            let plan = crate::csharp_sdk::plan_sdk_with_options(
                contract,
                operations,
                csharp_config(config),
                csharp_options(options, attribution),
            )
            .map_err(|errors| errors.into_iter().map(Into::into).collect::<Vec<_>>())?;
            plan.render()
                .map_err(|errors| errors.into_iter().map(Into::into).collect())
        }
        #[cfg(feature = "dart-sdk")]
        Backend::DartHttp => {
            let mut native_config = dart_config(config);
            native_config.credential_env = options.credential_env.clone();
            native_config.attribution = attribution.cloned();
            native_config.sdk_defaults = options.sdk_defaults.clone();
            let plan = crate::dart_sdk::plan_sdk_with_profiles(
                contract,
                operations,
                native_config,
                &options.compatibility_profiles,
            )
            .map_err(|errors| errors.into_iter().map(Into::into).collect::<Vec<_>>())?;
            Ok(plan.render())
        }
        #[cfg(feature = "cpp-sdk")]
        Backend::CppHttp => {
            let mut native_config = cpp_config(config);
            native_config.legacy_binary_strings = options.legacy_binary_strings();
            native_config.credential_env = options.credential_env.clone();
            native_config.sdk_defaults = options.sdk_defaults.clone();
            native_config.attribution = attribution.cloned();
            let plan = crate::cpp_sdk::plan_sdk(contract, operations, native_config)
                .map_err(|errors| errors.into_iter().map(Into::into).collect::<Vec<_>>())?;
            plan.render()
                .map_err(|errors| errors.into_iter().map(Into::into).collect())
        }
        #[cfg(feature = "kotlin-sdk")]
        Backend::KotlinHttp => {
            let mut native_config = kotlin_config(config)?;
            native_config.credential_env = options.credential_env.clone();
            native_config.sdk_defaults = options.sdk_defaults.clone();
            native_config.attribution = attribution.cloned();
            let plan = crate::kotlin_sdk::plan_sdk_with_profiles(
                contract,
                operations,
                native_config,
                &options.compatibility_profiles,
            )
            .map_err(|errors| errors.into_iter().map(Into::into).collect::<Vec<_>>())?;
            plan.render()
                .map_err(|errors| errors.into_iter().map(Into::into).collect())
        }
        #[cfg(feature = "php-sdk")]
        Backend::PhpHttp => {
            let mut native_config = php_config(config);
            native_config.credential_env = options.credential_env.clone();
            native_config.attribution = attribution.cloned();
            native_config.sdk_defaults = options.sdk_defaults.clone();
            let plan = crate::php_sdk::protocol::plan_sdk(
                contract,
                operations,
                native_config,
                options.apply_to(crate::php_sdk::protocol::capabilities()),
            )
            .map_err(|errors| errors.into_iter().map(Into::into).collect::<Vec<_>>())?;
            Ok(plan.render())
        }
        #[cfg(feature = "java-sdk")]
        Backend::JavaHttp => {
            let (package, maven) = java_config(config)?;
            let plan = crate::java_sdk::plan_sdk_with_protocol_v3(
                contract,
                operations,
                package,
                &[],
                maven,
                java_options(options, attribution),
            )
            .map_err(|errors| errors.into_iter().map(Into::into).collect::<Vec<_>>())?;
            plan.render()
                .map_err(|errors| errors.into_iter().map(Into::into).collect())
        }
    }
}
