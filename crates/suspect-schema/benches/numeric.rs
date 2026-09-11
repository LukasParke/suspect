//! Reproducible exact-arithmetic timings through the public API. Parsing the
//! input document is outside the measurement; compilation and validation
//! have separate measurements. Run `cargo bench -p suspect-schema --bench numeric`.

use std::hint::black_box;
use std::time::Duration;

use criterion::{BenchmarkId, Criterion, criterion_group, criterion_main};
use suspect_low::LowDoc;
use suspect_schema::{Compiler, Config};
use suspect_source::{Source, Uri};

fn doc(text: &str) -> LowDoc {
    LowDoc::parse(
        Uri::parse("memory://numeric.json").unwrap(),
        Source::from_vec(text.as_bytes().to_vec()),
    )
}

fn numeric(c: &mut Criterion) {
    let power = format!("1{}", "0".repeat(1000));
    let previous = "9".repeat(1000);
    let cases = [
        (
            "ordinary_decimal",
            r#"{"type":"number","minimum":0,"maximum":1000,"multipleOf":0.01}"#.to_owned(),
            "4.21".to_owned(),
        ),
        (
            "wide_integer",
            r#"{"type":"integer","minimum":9007199254740993,"maximum":18446744073709551616,"multipleOf":3}"#.to_owned(),
            "9007199254740993".to_owned(),
        ),
        (
            "exponent_400",
            r#"{"type":"integer","minimum":1e400,"maximum":1e400,"multipleOf":0.125}"#.to_owned(),
            "10e399".to_owned(),
        ),
        (
            "exponent_1001_digits",
            format!(r#"{{"type":"integer","minimum":1e{power},"maximum":1e{power},"multipleOf":0.125}}"#),
            format!("10e{previous}"),
        ),
    ];
    let mut group = c.benchmark_group("exact_numeric");
    group.sample_size(20);
    group.warm_up_time(Duration::from_secs(1));
    group.measurement_time(Duration::from_secs(2));
    for (name, schema, instance) in &cases {
        let schema_doc = doc(schema);
        let instance_doc = doc(instance);
        let compiler = Compiler::new(Config::default());
        let compiled = compiler.compile(schema_doc.root()).unwrap();
        assert!(compiled.validate(instance_doc.root()).is_empty(), "{name}");
        group.bench_with_input(BenchmarkId::new("compile", name), &schema_doc, |b, d| {
            b.iter(|| black_box(compiler.compile(d.root()).unwrap()));
        });
        group.bench_function(BenchmarkId::new("validate", name), |b| {
            b.iter(|| black_box(compiled.validate(instance_doc.root())));
        });
    }
    group.finish();
}

criterion_group!(benches, numeric);
criterion_main!(benches);
