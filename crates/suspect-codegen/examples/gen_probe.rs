//! Timing probe for the generation pipeline: splits the cold and
//! incremental costs of a session tick into its stages. Run with:
//!
//! ```sh
//! cargo run --release -p suspect-codegen --example gen_probe -- \
//!     <spec.yaml> [operationId ...]
//! ```

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;

use suspect_codegen::backend::GenerationOptions;
use suspect_ir::contract::Contract;
use suspect_ref::WorkspaceBuilder;

fn ms(start: Instant) -> f64 {
    start.elapsed().as_secs_f64() * 1000.0
}

fn main() {
    let mut args = std::env::args().skip(1);
    let spec = PathBuf::from(args.next().expect("spec path"));
    let operations: Vec<String> = args.collect();

    let root = spec.parent().map(PathBuf::from).unwrap_or_default();
    let name = spec
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "spec.yaml".to_owned());

    let t = Instant::now();
    let ws = Arc::new(
        WorkspaceBuilder::new()
            .root(&root)
            .build()
            .expect("workspace"),
    );
    ws.load_all(&name).expect("load");
    let load = ms(t);

    let entry = ws.uris().first().expect("entry uri").clone();

    let t = Instant::now();
    let contract = Arc::new(Contract::from_workspace(&ws, &entry).expect("contract"));
    let compile = ms(t);
    println!("operations: {}", contract.operations().count());

    let t = Instant::now();
    let report = suspect_codegen::admission::review(&contract);
    let admission = ms(t);
    println!(
        "admission: {} findings ({} refusals)",
        report.findings.len(),
        report
            .findings
            .iter()
            .filter(|f| f.kind == suspect_codegen::admission::FindingKind::Refusal)
            .count(),
    );

    let selected: Vec<_> = if operations.is_empty() {
        contract
            .operations()
            .map(|op| op.source().clone())
            .collect()
    } else {
        operations
            .iter()
            .map(|id| {
                contract
                    .operations()
                    .find(|op| op.operation_id() == Some(id.as_str()))
                    .expect("operation exists")
                    .source()
                    .clone()
            })
            .collect()
    };

    let target = suspect_codegen::backend::TargetConfig {
        backend: suspect_codegen::backend::Backend::TypescriptHttp,
        package_name: "bench".to_owned(),
        package_version: "0.1.0".to_owned(),
        import_name: None,
    };
    let t = Instant::now();
    let files = suspect_codegen::backend::generate_with_options(
        contract.clone(),
        &selected,
        &target,
        &GenerationOptions::default(),
    )
    .expect("render");
    let render = ms(t);
    println!(
        "render: {} files, {:.0} KiB",
        files.len(),
        files.iter().map(|f| f.content.len()).sum::<usize>() as f64 / 1024.0
    );

    let t = Instant::now();
    let selection = suspect_ir::contract::OperationSelection::new([
        "getFeatures",
        "getHomeUsers",
        "getIdentity",
        "getSections",
        "getServerInfo",
        "listDVRs",
    ]);
    let contract3 = Arc::new(
        suspect_ir::contract::Contract::from_workspace_scoped(&ws, &entry, &selection)
            .expect("scoped contract"),
    );
    let scoped_compile = ms(t);
    let scoped_ops = contract3.operations().count();

    let t = Instant::now();
    let contract2 = Arc::new(Contract::from_workspace(&ws, &entry).expect("contract 2"));
    let recompile = ms(t);
    let same_ptr = Arc::ptr_eq(&contract, &contract2);

    println!();
    println!("load (parse 63k lines):        {load:8.1} ms");
    println!("contract compile (all ops):   {compile:8.1} ms");
    println!("admission review (all ops):    {admission:8.1} ms");
    println!("render typescript (6/404 ops): {render:8.1} ms");
    println!("scoped compile (6 ops):       {scoped_compile:8.1} ms  ({scoped_ops} ops in graph)");
    println!("recompile from cached ws:     {recompile:8.1} ms  (Arc identical: {same_ptr})");
}
