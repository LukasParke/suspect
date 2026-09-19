//! Execute the actual emitted README first-request fence against a controlled
//! transport. The operation name is deliberately unrelated to a business task.
#![cfg(feature = "ruby-sdk")]

use serde_json::{Value, json};
use std::{
    path::{Path, PathBuf},
    process::Command,
    sync::Arc,
};
use suspect_codegen::ruby_sdk::{self, PackageConfig, RubyConfig, SdkPlan};
use suspect_ir::contract::Contract;
use suspect_ref::{DocumentProvider, ProvidedDocument, WorkspaceBuilder};
use suspect_source::Uri;

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .unwrap()
}
fn source(json_response: bool) -> Value {
    let responses = if json_response {
        json!({"200":{"description":"Ready","content":{"application/json":{"schema":{"type":"object","required":["ready"],"properties":{"ready":{"type":"boolean"}}},"example":{"ready":true}}}}})
    } else {
        json!({"204":{"description":"Ready"}})
    };
    json!({"openapi":"3.1.2","info":{"title":"README source call","version":"1"},
    "servers":[{"url":"https://readme.ruby.test/v1"}],"security":[{"apiKey":[]}],
    "components":{"securitySchemes":{"apiKey":{"type":"http","scheme":"bearer"}}},
    "paths":{
        "/ready":{"get":{"operationId":"inspect","responses":responses}},
        "/z-unpreferred":{"get":{"operationId":"getCurrentKey","responses":responses}}
    }})
}
fn plan(value: Value) -> SdkPlan {
    let uri = Uri::parse("https://physical.readme.ruby.test/openapi.json").unwrap();
    let provider = Arc::new(
        DocumentProvider::new([ProvidedDocument::new(
            uri.clone(),
            uri.clone(),
            serde_json::to_vec(&value).unwrap(),
        )
        .unwrap()])
        .unwrap(),
    );
    let workspace = Arc::new(
        WorkspaceBuilder::new()
            .allowed_documents(provider.logical_uris())
            .document_provider(provider)
            .build()
            .unwrap(),
    );
    let contract = Arc::new(Contract::from_workspace(&workspace, &uri).unwrap());
    let selected = contract
        .operations()
        .map(|operation| operation.source().clone())
        .collect::<Vec<_>>();
    ruby_sdk::plan_sdk(contract, &selected, RubyConfig::default()).unwrap()
}
fn identity() -> PackageConfig {
    PackageConfig {
        name: "readme-probe".into(),
        version: "0.1.0".into(),
        require_name: "readme_probe".into(),
        namespace: "ReadmeProbe".into(),
    }
}
fn ruby_home() -> PathBuf {
    std::env::var_os("SUSPECT_RUBY_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            PathBuf::from(std::env::var_os("HOME").unwrap())
                .join(".local/share/mise/installs/ruby/3.3.12")
        })
}
fn first_request(readme: &str) -> &str {
    readme
        .split_once("## First request\n")
        .unwrap()
        .1
        .split_once("```ruby\n")
        .unwrap()
        .1
        .split_once("\n```")
        .unwrap()
        .0
}

#[test]
#[ignore = "requires Ruby 3.3.12/4.0.6; executes the emitted README fence, with no real network calls"]
fn emitted_no_input_readme_executes_the_allocated_source_method() {
    for json_response in [false, true] {
        let plan = plan(source(json_response));
        let method = &plan
            .operations()
            .iter()
            .find(|op| op.operation_id == "inspect")
            .unwrap()
            .method_name;
        assert_eq!(method, "inspect_value");
        let files = ruby_sdk::emit_sdk(&plan, &identity()).unwrap();
        let readme = &files
            .iter()
            .find(|file| file.path == "ruby/README.md")
            .unwrap()
            .content;
        let snippet = first_request(readme).to_owned();
        let base = root().join("target/sdk-ruby-readme-f3-native");
        std::fs::create_dir_all(&base).unwrap();
        let directory = tempfile::Builder::new()
            .prefix(if json_response { "json-" } else { "empty-" })
            .tempdir_in(base)
            .unwrap()
            .keep();
        for file in files {
            let path = directory.join(file.path);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, file.content).unwrap();
        }
        std::fs::write(directory.join("readme-fragment.rb"), &snippet).unwrap();
        let status = if json_response { 200 } else { 204 };
        let body = if json_response {
            r#"{"ready":true}"#
        } else {
            ""
        };
        let script = format!(
            r#"require 'readme_probe'
require 'timeout'
REQUESTS = []
class ReadmeProbe::NetHTTPTransport
  def exchange(request:, context:)
    context.check!
    raise 'wrong allocated operation or source URL' unless request.operation_id == 'inspect' && request.method == 'GET' && request.url == 'https://readme.ruby.test/v1/ready'
    raise 'missing fixture credentials' unless request.headers['Authorization'] == 'Bearer controlled-readme-token'
    REQUESTS << request
    yield ReadmeProbe::WireResponse.new(status: {status}, headers: {{'Content-Type' => 'application/json'}}, body: {body:?})
  end
end
credentials = {{'apiKey' => 'controlled-readme-token'}}
Timeout.timeout(5) {{ eval(File.binread('readme-fragment.rb'), binding, 'ruby/README.md first request') }}
raise 'README did not perform exactly one request' unless REQUESTS.length == 1
puts 'Actual README first request executed its allocated no-input source method'
"#
        );
        std::fs::write(directory.join("consumer.rb"), script).unwrap();
        let output = Command::new(ruby_home().join("bin/ruby"))
            .arg("-I")
            .arg(directory.join("ruby/lib"))
            .arg("consumer.rb")
            .current_dir(&directory)
            .env_remove("RUBYOPT")
            .env_remove("RUBYLIB")
            .env_remove("GEM_HOME")
            .env_remove("GEM_PATH")
            .output()
            .unwrap();
        let log = format!(
            "Ruby {}\n{}{}",
            ruby_home().display(),
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        std::fs::write(directory.join("readme.log"), &log).unwrap();
        assert!(output.status.success(), "{}\n{log}", directory.display());
        assert!(snippet.contains(&format!("client.{method}")));
        eprintln!("Actual README execution: {}", directory.display());
    }
}
