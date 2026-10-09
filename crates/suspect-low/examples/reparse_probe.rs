//! didChange reparse cost: whole-file parse vs incremental reparse after a
//! one-line edit, on a real specification. Run with:
//!
//! ```sh
//! cargo run --release -p suspect-low --example reparse_probe -- <spec.yaml>
//! ```

use suspect_low::LowDoc;
use suspect_source::Source;
use suspect_syntax::Edit;

fn ms(start: std::time::Instant) -> f64 {
    start.elapsed().as_secs_f64() * 1000.0
}

fn main() {
    let path = std::env::args().nth(1).expect("spec path");
    let bytes = std::fs::read(&path).expect("read");
    let lines = bytes.iter().filter(|&&b| b == b'\n').count();

    let t = std::time::Instant::now();
    let doc = LowDoc::parse("file:///probe.yaml".into(), Source::from_vec(bytes.clone()));
    let cold = ms(t);

    // A realistic keystroke: append two characters to a summary line,
    // inside the document's body.
    let at = bytes
        .windows(8)
        .rposition(|w| w == b"summary:")
        .map(|i| {
            bytes[i..]
                .iter()
                .position(|&b| b == b'\n')
                .map(|n| i + n)
                .unwrap_or(i)
        })
        .unwrap_or(bytes.len() / 2);

    // Full reparse of the edited text (the pre-fix didChange behavior).
    let mut edited = bytes.clone();
    edited.splice(at..at, b" x".to_vec());
    let t = std::time::Instant::now();
    let _full = LowDoc::parse(
        "file:///probe.yaml".into(),
        Source::from_vec(edited.clone()),
    );
    let full_reparse = ms(t);

    // Incremental reparse of the same edit (the new didChange behavior).
    let li = doc.inner().line_index().clone();
    let edit = Edit::from_buffer(doc.inner().bytes(), &li, at, at, 2, b" x");
    let t = std::time::Instant::now();
    let _incremental = doc.reparse(Source::from_vec(edited), std::slice::from_ref(&edit));
    let incremental = ms(t);

    // And a line-deletion edit, the case whole-document costs were worst at.
    let line_end = bytes[at..]
        .iter()
        .position(|&b| b == b'\n')
        .map(|n| at + n + 1)
        .unwrap_or(at);
    let mut deleted = bytes.clone();
    deleted.drain(at..line_end);
    let edit = Edit::from_buffer(doc.inner().bytes(), &li, at, line_end, 0, b"");
    let t = std::time::Instant::now();
    let _deleted_incremental = doc.reparse(Source::from_vec(deleted), std::slice::from_ref(&edit));
    let deletion = ms(t);

    println!("document: {lines} lines, {} bytes", bytes.len());
    println!("cold parse:                {cold:8.2} ms");
    println!("didChange, whole-file:     {full_reparse:8.2} ms  (pre-fix behavior)");
    println!("didChange, incremental:    {incremental:8.2} ms  (insertion)");
    println!("didChange, incremental:    {deletion:8.2} ms  (line deletion)");
}
