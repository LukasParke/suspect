use suspect_low::{LowDoc, SpecFamily};
use suspect_source::{Source, Uri};

#[test]
fn stripe_parses() {
    // corpus/ is gitignored; skip on clean checkouts and CI
    let corpus = concat!(env!("CARGO_MANIFEST_DIR"), "/../../corpus");
    if !std::path::Path::new(corpus).join("stripe.yaml").exists() {
        eprintln!("skipping: corpus absent");
        return;
    }
    for f in [
        concat!(env!("CARGO_MANIFEST_DIR"), "/../../corpus/stripe.yaml"),
        concat!(env!("CARGO_MANIFEST_DIR"), "/../../corpus/stripe-sdk.yaml"),
    ] {
        let bytes = std::fs::read(f).unwrap();
        let len = bytes.len();
        let doc = LowDoc::parse(Uri::from("mem://s.yaml"), Source::from_vec(bytes));
        println!(
            "{f}: family={:?} errors={} schemas={:?}",
            doc.sniff_family(),
            doc.syntax_errors().len(),
            doc.root()
                .get("components")
                .map(|c| c.get("schemas").map(|s| s.entries().len()))
        );
        let _ = len;
    }
    // Stripe publishes OAS 3.0 documents
    assert_eq!(doc_family(), SpecFamily::Oas30);
}
fn doc_family() -> SpecFamily {
    let bytes = std::fs::read(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../corpus/stripe.yaml"
    ))
    .unwrap();
    LowDoc::parse(Uri::from("mem://s.yaml"), Source::from_vec(bytes)).sniff_family()
}
