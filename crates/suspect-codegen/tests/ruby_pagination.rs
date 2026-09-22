//! Generated pagination for the Ruby gem: emission shape, plan carriage, and
//! native behavioral verification of the emitted page/item enumerators over a
//! scripted transport. Static runtime files are never modified; the walk lives
//! entirely in the generated `client.rb`.
#![cfg(feature = "ruby-sdk")]

use serde_json::{Value, json};
use std::{process::Command, sync::Arc};

use suspect_codegen::{
    OutFile,
    backend::{Backend, GenerationOptions, TargetConfig, generate_with_options},
    ruby_sdk,
    sdk_defaults::SdkDefaults,
};
use suspect_ir::contract::Contract;
use suspect_ref::{DocumentProvider, ProvidedDocument, WorkspaceBuilder};
use suspect_source::Uri;

fn document() -> Value {
    json!({
        "openapi":"3.1.0","info":{"title":"Pagination","version":"1"},
        "servers":[{"url":"https://api.pagination.test/v1"}],
        "paths":{
            "/widgets":{"get":{
                "operationId":"listWidgets",
                "parameters":[
                    {"name":"limit","in":"query","schema":{"type":"integer","minimum":1}},
                    {"name":"offset","in":"query","schema":{"type":"integer","minimum":0}},
                    {"name":"filter","in":"query","schema":{"type":"string"}}
                ],
                "responses":{"200":{"description":"Page","content":{"application/json":{"schema":{
                    "type":"object","required":["data","total"],"properties":{
                        "data":{"type":"array","items":{"$ref":"#/components/schemas/Widget"}},
                        "total":{"type":"integer"},
                        "has_more":{"type":"boolean"}
                    }}}}}}}
            },
            "/things":{"get":{
                "operationId":"listThings",
                "parameters":[
                    {"name":"page","in":"query","schema":{"type":"integer","minimum":1}},
                    {"name":"limit","in":"query","schema":{"type":"integer","minimum":1}}
                ],
                "responses":{"200":{"description":"Page","content":{"application/json":{"schema":{
                    "type":"object","properties":{
                        "data":{"type":"array","items":{"type":"string"}}
                    }}}}}}}
            },
            "/events":{"get":{
                "operationId":"listEvents",
                "parameters":[
                    {"name":"cursor","in":"query","schema":{"type":"string"}},
                    {"name":"filter","in":"query","schema":{"type":"string"}}
                ],
                "responses":{"200":{"description":"Page","content":{"application/json":{"schema":{
                    "type":"object","properties":{
                        "items":{"type":"array","items":{"type":"string"}},
                        "next_page_token":{"type":"string"}
                    }}}}}}}
            }
        },
        "components":{"schemas":{"Widget":{"type":"object","required":["id"],"properties":{"id":{"type":"string"}}}}}
    })
}

fn contract() -> Arc<Contract> {
    let entry = Uri::parse("https://source.pagination.test/openapi.json").unwrap();
    let provider = Arc::new(
        DocumentProvider::new([ProvidedDocument::new(
            entry.clone(),
            entry.clone(),
            serde_json::to_vec(&document()).unwrap(),
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
    Arc::new(Contract::from_workspace(&workspace, &entry).unwrap())
}

fn generate(options: &GenerationOptions) -> Vec<OutFile> {
    let contract = contract();
    let selected = contract
        .operations()
        .map(|operation| operation.source().clone())
        .collect::<Vec<_>>();
    generate_with_options(
        contract,
        &selected,
        &TargetConfig {
            backend: Backend::RubyHttp,
            package_name: "pagination-sdk".into(),
            package_version: "0.1.0".into(),
            import_name: None,
        },
        options,
    )
    .unwrap()
}

fn configured_options() -> GenerationOptions {
    GenerationOptions {
        sdk_defaults: Some(SdkDefaults::v1()),
        ..Default::default()
    }
}

fn content<'a>(files: &'a [OutFile], suffix: &str) -> &'a str {
    files
        .iter()
        .find(|file| file.path.ends_with(suffix))
        .unwrap_or_else(|| panic!("no generated file ending in {suffix}"))
        .content
        .as_str()
}

#[ignore = "requires the Ruby interpreter on the test host"]
#[test]
fn configured_emission_extends_only_the_generated_client_and_signatures() {
    let configured = generate(&configured_options());
    let control = generate(&GenerationOptions::default());
    assert_eq!(configured.len(), control.len(), "no files may be added");
    let mut changed = Vec::new();
    for file in &control {
        let emitted = configured
            .iter()
            .find(|candidate| candidate.path == file.path)
            .unwrap_or_else(|| panic!("{} disappeared under the policy", file.path));
        if emitted.content != file.content {
            changed.push(file.path.clone());
        }
    }
    changed.sort();
    assert_eq!(
        changed,
        vec![
            "ruby/lib/pagination_sdk/client.rb".to_owned(),
            "ruby/sig/pagination_sdk.rbs".to_owned(),
        ],
        "only the generated client file and its signatures may change"
    );
    let client = content(&configured, "ruby/lib/pagination_sdk/client.rb");
    for expected in [
        "# Lazily yields every success page of listWidgets.",
        "def list_widgets_pages(**kwargs)",
        "Enumerator.new do |yielder|",
        "def list_widgets_items(**kwargs)",
        "def list_widgets_next_page(**kwargs)",
        "def list_events_pages(**kwargs)",
        "def list_events_items(**kwargs)",
        "def list_events_next_page(**kwargs)",
        // Limit/offset walk: exact advance with the documented zero-item stop
        // and the caller's other keyword arguments preserved.
        "count = pagination_count(pagination_field(page.data, \"data\"))",
        "break if count.zero?",
        "position += count",
        "request[:offset] = position",
        "page = list_widgets(**request)",
        // Cursor walk: pointer-driven continuation with the repeat guard, and
        // the empty-token stop rule.
        "value = pagination_field(page.data, \"next_page_token\")",
        "break if value.nil? || value.equal?(UNSET) || value == ''",
        "pagination-stalled",
        "request[:cursor] = value",
        // Page-number walk: one-based increment with the documented stops.
        "def list_things_pages(**kwargs)",
        "position += 1",
        "request[:page] = position",
        // The helper readers are private and tolerate absent values.
        "private :pagination_field, :pagination_count",
    ] {
        assert!(
            client.contains(expected),
            "client.rb is missing:\n{expected}\n--- emitted: ---\n{client}"
        );
    }
    // The first request of a walk carries the configured initial offset, and
    // the caller-supplied offset wins until the first continuation.
    assert!(
        client.contains("position = 0 if position.nil? || position.equal?(UNSET)"),
        "{client}"
    );
    assert!(client.contains("request[:offset] = position"), "{client}");
    assert!(client.contains("loop do"), "{client}");
    // RBS signatures cover every emitted method.
    let signatures = content(&configured, "ruby/sig/pagination_sdk.rbs");
    for expected in [
        "def list_widgets_pages: (**untyped) -> Enumerator[ListWidgetsStatus200]",
        "def list_widgets_items: (**untyped) -> Enumerator[Types::list_widgets_response200_data_items]",
        "def list_things_pages: (**untyped) -> Enumerator[ListThingsStatus200]",
        "def list_things_next_page: (**untyped) -> Hash[Symbol, untyped]?",
        "def list_widgets_next_page: (**untyped) -> Hash[Symbol, untyped]?",
        "def list_events_pages: (**untyped) -> Enumerator[ListEventsStatus200]",
        "def list_events_items: (**untyped) -> Enumerator[Types::list_events_response200_items_items]",
        "def list_events_next_page: (**untyped) -> Hash[Symbol, untyped]?",
    ] {
        assert!(
            signatures.contains(expected),
            "signatures are missing:\n{expected}\n--- emitted: ---\n{signatures}"
        );
    }
}

/// A manual mapping may declare a has-more indicator; the emitted walk must
/// then stop on `false` even when the page still returned items.
#[ignore = "requires the Ruby interpreter on the test host"]
#[test]
fn manual_has_more_mapping_emits_and_honors_the_false_stop_rule() {
    let defaults: SdkDefaults = serde_json::from_value(json!({
        "version": "v1",
        "pagination": {
            "operations": {
                "listWidgets": {
                    "pattern": "limit-offset",
                    "request": {"limit": "limit", "offset": "offset"},
                    "response": {"items": "/data", "has-more": "/has_more"},
                    "initial_offset": 0,
                    "advance": "items-returned"
                }
            }
        }
    }))
    .unwrap();
    let files = generate(&GenerationOptions {
        sdk_defaults: Some(defaults),
        ..Default::default()
    });
    let client = content(&files, "ruby/lib/pagination_sdk/client.rb");
    assert_eq!(
        client
            .matches("break if pagination_field(page.data, \"has_more\").equal?(false)")
            .count(),
        1,
        "the page walk must stop on a false has-more indicator; the item walk\n     inherits the stop through the pages walk"
    );
    // An explicit mapping does not disable detection for the other operations.
    assert!(
        client.contains("def list_events_pages(**kwargs)"),
        "{client}"
    );
}

#[ignore = "requires the Ruby interpreter on the test host"]
#[test]
fn policy_without_paginated_operations_emits_nothing_new() {
    let off = GenerationOptions {
        sdk_defaults: Some(
            serde_json::from_value(json!({
                "version":"v1","pagination":"off"
            }))
            .unwrap(),
        ),
        ..Default::default()
    };
    let disabled = generate(&off);
    let control = generate(&GenerationOptions::default());
    assert_eq!(disabled.len(), control.len());
    for (disabled, control) in disabled.iter().zip(control.iter()) {
        assert_eq!(disabled.path, control.path);
        assert_eq!(disabled.content, control.content);
    }
}

#[ignore = "requires the Ruby interpreter on the test host"]
#[test]
fn plan_carries_the_pagination_outcome_only_when_configured() {
    let contract = contract();
    let selected = contract
        .operations()
        .map(|operation| operation.source().clone())
        .collect::<Vec<_>>();
    let configured = ruby_sdk::plan_sdk(
        contract.clone(),
        &selected,
        ruby_sdk::RubyConfig {
            sdk_defaults: Some(serde_json::from_value(json!({"version":"v1"})).unwrap()),
            ..Default::default()
        },
    )
    .unwrap_or_else(|errors| panic!("{errors:#?}"));
    let pagination = configured
        .pagination()
        .expect("configured policy is carried");
    assert_eq!(pagination.operations.len(), 3);
    let widgets = pagination
        .operations
        .iter()
        .find(|operation| {
            operation.selection.pattern
                == suspect_codegen::sdk_defaults::PaginationPattern::LimitOffset
        })
        .expect("limit/offset operation");
    assert_eq!(widgets.selection.initial_offset, Some(0));
    assert_eq!(widgets.pages_name, "list_widgets_pages");
    assert_eq!(widgets.items_name, "list_widgets_items");
    assert_eq!(widgets.next_page_name, "list_widgets_next_page");
    let things = pagination
        .operations
        .iter()
        .find(|operation| {
            operation.selection.pattern
                == suspect_codegen::sdk_defaults::PaginationPattern::PageNumber
        })
        .expect("page-number operation");
    assert_eq!(things.pages_name, "list_things_pages");
    assert_eq!(things.next_page_name, "list_things_next_page");
    let events = pagination
        .operations
        .iter()
        .find(|operation| {
            operation.selection.pattern == suspect_codegen::sdk_defaults::PaginationPattern::Cursor
        })
        .expect("cursor operation");
    assert_eq!(events.selection.initial_offset, None);
    assert_eq!(events.pages_name, "list_events_pages");
    let control = ruby_sdk::plan_sdk(contract, &selected, ruby_sdk::RubyConfig::default())
        .unwrap_or_else(|errors| panic!("{errors:#?}"));
    assert!(control.pagination().is_none());
}

const BEHAVIOR: &str = r#"# frozen_string_literal: true
require 'json'
$LOAD_PATH.unshift(File.join(__dir__, 'ruby', 'lib'))
require 'pagination_sdk'

class ScriptedTransport
  attr_reader :requests

  def initialize(pages)
    @pages = pages
    @requests = []
    @lock = Mutex.new
  end

  def exchange(request:, context:)
    context.check!
    @lock.synchronize do
      index = @requests.length
      @requests << request.url
      raise "unexpected request #{index}" if index >= @pages.length
      yield PaginationSdk::WireResponse.new(
        status: 200,
        headers: { 'Content-Type' => 'application/json' },
        body: @pages[index]
      )
    end
  end
end

def client(transport)
  PaginationSdk::Client.new(transport: transport, server_url: 'http://127.0.0.1')
end

def query(request)
  URI(request).query.to_s
end

# Limit/offset walk: offset 0 then 2, filter preserved, zero-item page stops.
transport = ScriptedTransport.new([
  { data: [{ id: 'a' }, { id: 'b' }], total: 9 }.to_json,
  { data: [], total: 9 }.to_json,
])
client(transport).list_widgets_pages(limit: 2, filter: 'x').each_with_index do |page, index|
  raise 'unexpected status' unless page.status == 200
  expected = index.zero? ? %w[a b] : []
  raise 'walk shape changed' unless page.data.data.map(&:id) == expected
end
raise 'request count changed' unless transport.requests.length == 2
raise 'filter lost' unless transport.requests.all? { |r| query(r).include?('limit=2') && query(r).include?('filter=x') }
raise 'offset controls did not advance' unless query(transport.requests[0]).include?('offset=0') && query(transport.requests[1]).include?('offset=2')

# Caller offset is honored for page 1 and replaced afterwards; items flatten.
transport = ScriptedTransport.new([
  { data: [{ id: 'a' }, { id: 'b' }], total: 9 }.to_json,
  { data: [{ id: 'c' }], total: 9 }.to_json,
  { data: [], total: 9 }.to_json,
])
items = client(transport).list_widgets_items(limit: 2, offset: 10, filter: 'x').map(&:id)
raise 'items changed' unless items == %w[a b c]
raise 'caller offset was not respected' unless query(transport.requests[0]).include?('offset=10') && query(transport.requests[1]).include?('offset=12') && query(transport.requests[2]).include?('offset=13')

# Early break never issues another request.
transport = ScriptedTransport.new([
  { data: [{ id: 'a' }], total: 9 }.to_json,
  { data: [{ id: 'b' }], total: 9 }.to_json,
])
client(transport).list_widgets_pages(limit: 2).each { break }
raise 'early break issued a request' unless transport.requests.length == 1
items_transport = ScriptedTransport.new([
  { data: [{ id: 'a' }], total: 9 }.to_json,
  { data: [{ id: 'b' }], total: 9 }.to_json,
])
items = []
client(items_transport).list_widgets_items(limit: 2).each do |item|
  items << item.id
  break
end
raise 'item walk changed' unless items == ['a']
raise 'item break issued a request' unless items_transport.requests.length == 1

# Cursor walk: no invented cursor, second request carries the token, missing
# token stops.
transport = ScriptedTransport.new([
  { items: %w[i1], next_page_token: 'c2' }.to_json,
  { items: %w[i2] }.to_json,
])
items = client(transport).list_events_items(filter: 'f').to_a
raise 'cursor items changed' unless items == %w[i1 i2]
raise 'first request invented a cursor' if query(transport.requests[0]).include?('cursor=')
raise 'second cursor missing' unless query(transport.requests[1]).include?('cursor=c2')

# Caller cursor wins for page 1, then the computed continuation replaces it.
transport = ScriptedTransport.new([
  { items: %w[i1], next_page_token: 'c9' }.to_json,
  { items: %w[i2] }.to_json,
])
client(transport).list_events_pages(cursor: 'c1').to_a
raise 'caller cursor lost' unless query(transport.requests[0]).include?('cursor=c1')
raise 'continuation not applied' unless query(transport.requests[1]).include?('cursor=c9')

# An empty cursor token stops the walk.
transport = ScriptedTransport.new([{ items: %w[i1], next_page_token: '' }.to_json])
count = client(transport).list_events_pages.to_a.length
raise 'empty token walk changed' unless count == 1 && transport.requests.length == 1

# A repeated identical continuation raises the typed pagination-stalled error
# after delivering the offending page exactly once, issuing no third request.
transport = ScriptedTransport.new([
  { items: %w[i1], next_page_token: 'c1' }.to_json,
  { items: %w[i2], next_page_token: 'c1' }.to_json,
])
delivered = 0
begin
  client(transport).list_events_pages.each do
    delivered += 1
  end
  raise 'a repeated continuation must fail the walk'
rescue PaginationSdk::RequestError => error
  raise 'wrong failure kind' unless error.kind == :'pagination-stalled'
end
raise 'offending page was not delivered' unless delivered == 2
raise 'non-progress looped' unless transport.requests.length == 2

# Page-number walk: no invented page, increments 2 then 3, zero-item page stops.
transport = ScriptedTransport.new([
  { data: %w[t1 t2] }.to_json,
  { data: %w[t3] }.to_json,
  { data: [] }.to_json,
])
items = client(transport).list_things_items(limit: 2).to_a
raise 'page-number items changed' unless items == %w[t1 t2 t3]
raise 'first request invented a page' if query(transport.requests[0]).include?('page=')
raise 'page did not increment' unless query(transport.requests[1]).include?('page=2') && query(transport.requests[2]).include?('page=3')

# next_page builds the following keyword hash, or nil when the walk stops.
transport = ScriptedTransport.new([{ data: [{ id: 'a' }, { id: 'b' }], total: 9 }.to_json])
next_request = client(transport).list_widgets_next_page(limit: 2, filter: 'x')
raise 'builder issued extra requests' unless transport.requests.length == 1
raise 'next input lost the advanced offset' unless next_request[:offset] == 2
raise 'next input dropped other members' unless next_request[:limit] == 2 && next_request[:filter] == 'x'
transport = ScriptedTransport.new([{ data: [], total: 9 }.to_json])
raise 'zero-item page must end the walk' unless client(transport).list_widgets_next_page(limit: 2).nil?
transport = ScriptedTransport.new([{ items: %w[i1] }.to_json])
raise 'absent token must end the walk' unless client(transport).list_events_next_page(cursor: 'c2').nil?

puts 'pagination behavior verified'
"#;

fn ruby_home() -> std::path::PathBuf {
    std::env::var_os("SUSPECT_RUBY_HOME")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| {
            std::path::PathBuf::from(std::env::var_os("HOME").unwrap())
                .join(".local/share/mise/installs/ruby/3.3.12")
        })
}

/// The gem requires Ruby >= 3.3; older interpreters cannot even parse the
/// emitted runtime syntax, so discovery refuses them.
fn ruby() -> Option<std::path::PathBuf> {
    if let Some(path) = std::env::var_os("SUSPECT_RUBY_BIN") {
        return Some(std::path::PathBuf::from(path));
    }
    let ruby = ruby_home().join("bin/ruby");
    if !ruby.is_file() {
        return None;
    }
    let output = Command::new(&ruby).arg("--version").output().ok()?;
    let text = String::from_utf8_lossy(&output.stdout).trim().to_owned();
    let minor = text
        .strip_prefix("ruby ")
        .and_then(|rest| rest.split('.').nth(1))
        .and_then(|minor| minor.parse::<u32>().ok())?;
    (minor >= 3).then_some(ruby)
}

fn checked(command: &mut Command, root: &std::path::Path, label: &str) {
    let output = command.output().unwrap();
    std::fs::write(root.join(format!("{label}.stdout.log")), &output.stdout).unwrap();
    std::fs::write(root.join(format!("{label}.stderr.log")), &output.stderr).unwrap();
    assert!(
        output.status.success(),
        "{command:?}\n{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

#[ignore = "requires the Ruby interpreter on the test host"]
#[test]
fn native_walks_count_requests_and_preserve_input() {
    let Some(ruby) = ruby() else {
        eprintln!("ruby_pagination: no Ruby >= 3.3 toolchain; degrading to static assertions");
        return;
    };
    let root = tempfile::tempdir().unwrap();
    let files = generate(&configured_options());
    suspect_codegen::write_files(&files, root.path()).unwrap();

    // Every emitted Ruby file must at least be syntactically valid.
    for file in &files {
        if file.path.ends_with(".rb") {
            checked(
                Command::new(&ruby)
                    .arg("-c")
                    .arg(root.path().join(&file.path)),
                root.path(),
                "syntax",
            );
        }
    }

    std::fs::write(root.path().join("behavior.rb"), BEHAVIOR).unwrap();
    checked(
        Command::new(&ruby)
            .arg(root.path().join("behavior.rb"))
            .current_dir(root.path()),
        root.path(),
        "behavior",
    );
    eprintln!("ruby_pagination: native Ruby behavioral gate passed");
}
