//! Process status through `main`: runtime usage errors exit 2, and the one
//! Ctrl-C listener gives 130 for interrupted work and 0 for servers.

use std::path::{Path, PathBuf};
use std::process::Command;

fn example(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../examples/load")
        .join(name)
        .canonicalize()
        .expect("example exists")
}

#[test]
fn missing_profile_exits_2_through_main() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::copy(
        example("petstore.openapi.yaml"),
        dir.path().join("api.yaml"),
    )
    .unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_suspect"))
        .current_dir(dir.path())
        .args([
            "gen",
            "sdk",
            "api.yaml",
            "--package-name",
            "fixture",
            "--output",
            "output",
        ])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(2), "{output:?}");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("--profile is required"), "{stderr}");
    assert!(!dir.path().join("output").exists());
}

#[cfg(unix)]
mod sigint {
    use std::io::{BufRead, BufReader};
    use std::net::TcpListener;
    use std::process::{Child, Command, Stdio};
    use std::sync::mpsc;
    use std::time::{Duration, Instant};

    use super::example;

    fn interrupt(child: &Child) {
        let pid = libc::pid_t::try_from(child.id()).unwrap();
        // SAFETY: plain kill(2) on a child this test spawned and still owns.
        assert_eq!(unsafe { libc::kill(pid, libc::SIGINT) }, 0);
    }

    fn wait(child: &mut Child, limit: Duration) -> Option<i32> {
        let start = Instant::now();
        loop {
            if let Some(status) = child.try_wait().unwrap() {
                return status.code();
            }
            if start.elapsed() > limit {
                let _ = child.kill();
                panic!("suspect did not exit within {limit:?}");
            }
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    /// `test load` against a listener that accepts and never answers: Ctrl-C
    /// stops arrivals, the grace period drains nothing, and no request
    /// completed, so the run reports 130.
    #[test]
    fn interrupted_load_exits_130() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let (accepted_tx, accepted_rx) = mpsc::channel();
        std::thread::spawn(move || {
            let mut held = Vec::new();
            for stream in listener.incoming() {
                held.push(stream);
                let _ = accepted_tx.send(());
            }
        });
        let mut child = Command::new(env!("CARGO_BIN_EXE_suspect"))
            .args(["test", "load"])
            .arg(example("flows.arazzo.yaml"))
            .args(["--workflow", "browse", "--rate", "2", "--duration", "60s"])
            .args(["--grace", "1s", "--timeout", "60s", "--format", "ndjson"])
            .args(["--base-url", &format!("http://127.0.0.1:{port}")])
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        // The listener is installed before dispatch, so once a virtual user
        // has connected, SIGINT reaches the graceful path.
        accepted_rx
            .recv_timeout(Duration::from_secs(60))
            .expect("load connected");
        interrupt(&child);
        let code = wait(&mut child, Duration::from_secs(30));
        let mut stderr = String::new();
        for line in BufReader::new(child.stderr.take().unwrap()).lines() {
            stderr.push_str(&line.unwrap());
            stderr.push('\n');
        }
        assert_eq!(code, Some(130), "{stderr}");
        assert!(stderr.contains("interrupt: stopping arrivals"), "{stderr}");
    }

    /// Ctrl-C is how the gateway stops, so it exits 0.
    #[test]
    fn interrupted_gateway_exits_0() {
        let mut child = Command::new(env!("CARGO_BIN_EXE_suspect"))
            .args(["traffic", "gateway"])
            .arg(example("petstore.openapi.yaml"))
            .args(["--mode", "mock", "--port", "0"])
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        let mut stderr = BufReader::new(child.stderr.take().unwrap());
        let mut line = String::new();
        while !line.contains("gateway listening on") {
            line.clear();
            assert!(stderr.read_line(&mut line).unwrap() > 0, "gateway exited");
        }
        interrupt(&child);
        assert_eq!(wait(&mut child, Duration::from_secs(30)), Some(0));
    }
}
