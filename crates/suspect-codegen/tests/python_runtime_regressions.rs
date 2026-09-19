//! Native regressions from the independent M3 Python runtime review.
//!
//! Seams: emitted JSON functions, source-bound codecs configured through
//! `CodecConfig`, and installed HTTP packages configured through `HttpConfig`.
//! Consumers never modify generated metadata or monkey-patch generated code.
//! Select Python 3.11 or 3.14 with `SUSPECT_PYTHON_BIN`. Each invocation retains
//! a fresh candidate directory and command logs for diagnosis.

use std::{
    ffi::OsString,
    fs,
    path::{Path, PathBuf},
    process::{Command, Output},
    sync::Arc,
};

use serde_json::{Value, json};
use suspect_codegen::{
    OutFile,
    python_codecs::{CodecConfig, JsonLimits, plan_codecs},
    python_http::{HttpConfig, PackageConfig, emit_http, plan_http},
};
use suspect_ir::contract::{Contract, SchemaId};
use suspect_ref::WorkspaceBuilder;
use suspect_source::Uri;

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/python-runtime-regressions")
        .join(name)
}

fn python() -> OsString {
    std::env::var_os("SUSPECT_PYTHON_BIN").unwrap_or_else(|| "python3".into())
}

struct Native {
    root: PathBuf,
}

impl Native {
    fn new(label: &str, files: &[OutFile]) -> Self {
        let parent = std::env::var_os("SUSPECT_PYTHON_REGRESSION_ARTIFACTS")
            .map(PathBuf::from)
            .unwrap_or_else(|| {
                Path::new(env!("CARGO_MANIFEST_DIR"))
                    .join("../../target/sdk-python-runtime-regressions/candidates")
            });
        fs::create_dir_all(&parent).unwrap();
        let parent = fs::canonicalize(parent).unwrap();
        let root = tempfile::Builder::new()
            .prefix(&format!("{label}-"))
            .tempdir_in(&parent)
            .unwrap()
            .keep();
        suspect_codegen::write_files(files, &root).unwrap();
        Self { root }
    }

    fn checked(&self, command: &mut Command, label: &str) -> Output {
        command.env("PYTHONDONTWRITEBYTECODE", "1");
        let output = command.output().unwrap_or_else(|error| {
            panic!(
                "{label}: {command:?}: {error}; artifacts {}",
                self.root.display()
            )
        });
        fs::write(
            self.root.join(format!("{label}.stdout.log")),
            &output.stdout,
        )
        .unwrap();
        fs::write(
            self.root.join(format!("{label}.stderr.log")),
            &output.stderr,
        )
        .unwrap();
        assert!(
            output.status.success(),
            "{label}: {command:?}\nartifacts: {}\n{}{}",
            self.root.display(),
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr),
        );
        output
    }

    fn consumer(&self, script: &str, data: Value) -> PathBuf {
        let consumer = self.root.join("consumer.py");
        fs::copy(fixture(script), &consumer).unwrap();
        // Test inputs/expected source identities, separate from emitted metadata.
        fs::write(
            self.root.join("consumer-config.json"),
            serde_json::to_vec_pretty(&data).unwrap(),
        )
        .unwrap();
        consumer
    }

    fn flat(&self, script: &str, selector: &str, data: Value) {
        let consumer = self.consumer(script, data);
        self.checked(
            Command::new(python())
                .arg("-B")
                .arg(consumer)
                .arg(selector)
                .env("PYTHONPATH", self.root.join("python"))
                .current_dir(&self.root),
            "native",
        );
    }

    fn install_wheel(&self) -> PathBuf {
        let tools = std::env::var_os("SUSPECT_PYTHON_TOOLS")
            .map(PathBuf::from)
            .unwrap_or_else(|| {
                Path::new(env!("CARGO_MANIFEST_DIR"))
                    .join("../../target/sdk-native-python-tools/bin/python")
            });
        self.checked(
            Command::new(tools)
                .args(["-m", "build", "--wheel", "--no-isolation"])
                .env_remove("PYTHONPATH")
                .current_dir(self.root.join("python")),
            "wheel-build",
        );
        let wheel = fs::read_dir(self.root.join("python/dist"))
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .find(|path| path.extension().is_some_and(|extension| extension == "whl"))
            .expect("the emitted HTTP package must build a wheel");
        self.checked(
            Command::new("uv")
                .args(["venv", "--offline", "--python"])
                .arg(python())
                .arg(self.root.join("venv"))
                .current_dir(&self.root),
            "venv",
        );
        let executable = self.root.join("venv/bin/python");
        self.checked(
            Command::new("uv")
                .args(["pip", "install", "--offline", "--python"])
                .arg(&executable)
                .arg(wheel)
                .current_dir(&self.root),
            "wheel-install",
        );
        executable
    }

    fn installed(&self, script: &str, selectors: &[&str], data: Value) {
        let executable = self.install_wheel();
        let consumer = self.consumer(script, data);
        self.checked(
            Command::new(executable)
                .arg("-B")
                .arg(consumer)
                .args(selectors)
                .env_remove("PYTHONPATH")
                .current_dir(&self.root),
            "native",
        );
    }
}

fn json_consumer(label: &str, selector: &str) {
    Native::new(label, &suspect_codegen::python_json::emit()).flat(
        "json_cases.py",
        selector,
        json!({}),
    );
}

fn load(path: &Path) -> Arc<Contract> {
    let workspace = Arc::new(
        WorkspaceBuilder::new()
            .root(path.parent().unwrap())
            .build()
            .unwrap(),
    );
    Arc::new(Contract::from_workspace(&workspace, &Uri::from_path(path).unwrap()).unwrap())
}

fn schema(contract: &Contract, name: &str) -> SchemaId {
    let pointer = format!("/components/schemas/{name}");
    contract
        .schema_roots()
        .iter()
        .find(|id| id.pointer() == pointer)
        .unwrap_or_else(|| panic!("fixture has no schema {pointer}"))
        .clone()
}

fn codec_consumer(label: &str, roots: &[&str], config: CodecConfig, selector: &str) {
    let contract = load(&fixture("api.openapi.json"));
    let roots: Vec<_> = roots.iter().map(|name| schema(&contract, name)).collect();
    let data = json!({"document": contract.entry().as_str()});
    let plan = plan_codecs(contract, &roots, config).unwrap();
    Native::new(label, &plan.render()).flat("codec_cases.py", selector, data);
}

fn http_consumer(label: &str, selectors: &[&str]) {
    let contract = load(&fixture("api.openapi.json"));
    let selected = vec![
        contract
            .operations()
            .find(|operation| operation.operation_id() == Some("getValue"))
            .expect("fixture has getValue")
            .source()
            .clone(),
    ];
    let plan = plan_http(
        contract,
        &selected,
        HttpConfig {
            max_request_bytes: 4096,
            max_response_bytes: 256,
            ..Default::default()
        },
    )
    .unwrap();
    let package = PackageConfig {
        name: "python-runtime-regression-sdk".into(),
        version: "0.0.0".into(),
        import_name: "python_runtime_regression_sdk".into(),
    };
    let files = emit_http(&plan, &package).unwrap();
    Native::new(label, &files).installed(
        "http_cases.py",
        selectors,
        json!({
            "package": package.import_name,
            "method": plan.operations()[0].snake_name,
            "arguments": {},
            "valid_body": "{\"name\":\"ok\"}",
            "value_attribute": "name",
            "value_expected": "ok",
        }),
    );
}

#[test]
#[ignore = "requires native Python; select Python 3.11/3.14 with SUSPECT_PYTHON_BIN"]
fn json_padded_exponents_preserve_mathematical_values() {
    json_consumer("r2-exponents", "ExactNumbers.test_zero_padded_exponents");
}

#[test]
#[ignore = "requires native Python; select Python 3.11/3.14 with SUSPECT_PYTHON_BIN"]
fn json_integer_output_limits_admit_exact_fitting_values() {
    json_consumer(
        "r12-integer-bytes",
        "ExactNumbers.test_exact_integer_output_bytes",
    );
}

#[test]
#[ignore = "requires native Python; deterministic input metering, observational timings"]
fn json_string_scanning_has_bounded_work() {
    json_consumer("r1-string-scanning", "StringScanning");
}

#[test]
#[ignore = "requires native Python; select Python 3.11/3.14 with SUSPECT_PYTHON_BIN"]
fn json_error_formatting_is_bounded_and_control_escaped() {
    json_consumer("r16-diagnostics", "DiagnosticFormatting");
}

#[test]
#[ignore = "requires native Python; select Python 3.11/3.14 with SUSPECT_PYTHON_BIN"]
fn codecs_round_trip_native_objects_after_literal_and_json_union_arms() {
    codec_consumer(
        "r3-mixed-unions",
        &["LiteralOrObject", "JsonOrObject"],
        CodecConfig::default(),
        "MixedUnions",
    );
}

#[test]
#[ignore = "requires native Python; select Python 3.11/3.14 with SUSPECT_PYTHON_BIN"]
fn codecs_keep_json_resource_exhaustion_fatal_across_union_trials() {
    codec_consumer(
        "r3-branch-resource",
        &["LiteralOrStrings"],
        CodecConfig {
            json_limits: JsonLimits {
                max_work: 32,
                ..Default::default()
            },
            ..Default::default()
        },
        "BranchResources",
    );
}

#[test]
#[ignore = "requires native Python; select Python 3.11/3.14 with SUSPECT_PYTHON_BIN"]
fn codecs_charge_generic_json_copies_to_one_conversion_budget() {
    codec_consumer(
        "r6-shared-copy",
        &["CopyBox"],
        CodecConfig {
            max_conversion_steps: 512,
            ..Default::default()
        },
        "SharedCopyBudget",
    );
}

#[test]
#[ignore = "requires native Python; select Python 3.11/3.14 with SUSPECT_PYTHON_BIN"]
fn codecs_map_literal_equality_exhaustion_to_a_located_resource_error() {
    codec_consumer(
        "r7-literal-resource",
        &["Tagged"],
        CodecConfig {
            schema: suspect_schema::Config {
                max_equality_steps: 1,
                ..Default::default()
            },
            ..Default::default()
        },
        "LiteralEqualityBudget",
    );
}

#[test]
#[ignore = "requires native Python; select Python 3.11/3.14 with SUSPECT_PYTHON_BIN"]
fn codecs_decode_zero_padded_integer_exponents_exactly() {
    codec_consumer(
        "r2-integer-codec",
        &["IntegerBox"],
        CodecConfig::default(),
        "IntegerConversion",
    );
}

#[test]
#[ignore = "requires native Python, Python wheel tools, and cached offline uv/httpx dependencies"]
fn installed_http_cleanup_preserves_primary_failure_and_cancellation() {
    http_consumer("r8-http-cleanup", &["SyncCleanup", "AsyncCleanup"]);
}

#[test]
#[ignore = "requires native Python, Python wheel tools, and cached offline uv/httpx dependencies"]
fn installed_http_rejects_invalid_ports_at_the_sdk_boundary() {
    http_consumer("r13-http-ports", &["ServerPorts", "AsyncServerPorts"]);
}

#[test]
#[ignore = "separate opt-in: requires OPENROUTER_WEB_ROOT and native Python"]
fn original_openrouter_container_file_accepts_zero_padded_integer_exponents() {
    let source = PathBuf::from(
        std::env::var_os("OPENROUTER_WEB_ROOT")
            .expect("set OPENROUTER_WEB_ROOT for the original ContainerFile source gate"),
    )
    .join("projects/docs/openapi/openapi.yaml");
    let contract = load(&source);
    let root = schema(&contract, "ContainerFile");
    let plan = plan_codecs(
        contract,
        std::slice::from_ref(&root),
        CodecConfig::default(),
    )
    .unwrap();
    let symbol = plan
        .models()
        .symbols()
        .iter()
        .find(|symbol| symbol.source() == &root)
        .expect("ContainerFile must retain a source-bound codec");
    Native::new("r2-original-container-file", &plan.render()).flat(
        "codec_cases.py",
        "OriginalContainerFile",
        json!({"container_file_codec": format!("{}Codec", symbol.name())}),
    );
}
