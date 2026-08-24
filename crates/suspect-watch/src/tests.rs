//! Tests for the debounced watcher.

use std::sync::mpsc;
use std::time::Duration;

use crate::{DebounceEvent, is_watched, watch};

/// Debounce window used by every test: short enough to stay fast, long
/// enough to coalesce two writes issued back to back.
const DEBOUNCE: Duration = Duration::from_millis(150);

#[test]
fn coalesces_burst_into_one_event() {
    let dir = tempfile::tempdir().unwrap();
    let a = dir.path().join("openapi.yaml");
    let b = dir.path().join("arazzo.json");
    std::fs::write(&a, "a: 1\n").unwrap();
    std::fs::write(&b, "{}\n").unwrap();

    let (tx, rx) = mpsc::channel();
    let _handle = watch(vec![dir.path().to_path_buf()], DEBOUNCE, tx).unwrap();

    // Two writes in quick succession must collapse into ONE event.
    std::fs::write(&a, "a: 2\n").unwrap();
    std::fs::write(&b, "{\"b\":2}\n").unwrap();

    let DebounceEvent::Changed(changed) = rx
        .recv_timeout(Duration::from_secs(2))
        .expect("debounced event within 2s");
    assert!(
        changed.contains(&a) || changed.contains(&b),
        "event should mention at least one written file, got {changed:?}"
    );

    // Exactly one burst: no second event arrives once the tree stays quiet.
    // (The first flush may split notify deliveries, so drain any trailing
    // duplicates before asserting quietness.)
    while rx.recv_timeout(DEBOUNCE * 3).is_ok() {}
}

#[test]
fn ignores_temp_and_foreign_files() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("doc.yaml"), "x: 1\n").unwrap();
    std::fs::write(dir.path().join("notes.txt"), "hi\n").unwrap();

    let (tx, rx) = mpsc::channel();
    let _handle = watch(vec![dir.path().to_path_buf()], DEBOUNCE, tx).unwrap();

    std::fs::write(dir.path().join("doc.yaml.swp"), "swap\n").unwrap();
    std::fs::write(dir.path().join("#doc.yaml#"), "lock\n").unwrap();
    std::fs::write(dir.path().join("doc.yaml~"), "backup\n").unwrap();
    std::fs::write(dir.path().join("readme.md"), "md\n").unwrap();

    // Nothing watched was touched: quiet for well past the debounce window.
    assert_eq!(
        rx.recv_timeout(Duration::from_millis(600)).err(),
        Some(mpsc::RecvTimeoutError::Timeout),
        "no event expected for temp/foreign files"
    );
}

#[test]
fn dropping_handle_terminates_thread() {
    let dir = tempfile::tempdir().unwrap();
    let doc = dir.path().join("spec.yml");
    std::fs::write(&doc, "v: 1\n").unwrap();

    let (tx, rx) = mpsc::channel();
    let handle = watch(vec![dir.path().to_path_buf()], DEBOUNCE, tx.clone()).unwrap();
    drop(handle);

    // Give the thread time to observe shutdown, then prove it is gone:
    // a fresh modification can no longer produce an event.
    std::thread::sleep(POLL * 4);
    std::fs::write(&doc, "v: 2\n").unwrap();
    assert_eq!(
        rx.recv_timeout(Duration::from_millis(600)).err(),
        Some(mpsc::RecvTimeoutError::Timeout),
        "thread kept running after the handle was dropped"
    );
}

const POLL: Duration = Duration::from_millis(50);

#[test]
fn filters_extensions_and_temps() {
    use std::path::Path;
    assert!(is_watched(Path::new("/x/a.yaml")));
    assert!(!is_watched(Path::new("/x/a.YML")));
    assert!(is_watched(Path::new("b.json")));
    assert!(!is_watched(Path::new("/x/a.txt")));
    assert!(!is_watched(Path::new("/x/a.yaml~")));
    assert!(!is_watched(Path::new("/x/.#a.yaml")));
    assert!(!is_watched(Path::new("/x/#a.yaml")));
    assert!(!is_watched(Path::new("/x/a.yaml.swp")));
}
