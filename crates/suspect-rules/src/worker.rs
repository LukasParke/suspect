//! Worker process management: spawn, handshake, evaluate with watchdog,
//! hot reload, kill-and-restart.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;

use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStdin, Command};
use tokio::sync::mpsc;
use tokio::time::{timeout, timeout_at};

use crate::protocol::{DoneBody, HostFrame, ReadyPayload, RuleMeta, WorkerFrame};
use crate::{Error, PROTOCOL_VERSION, Result};

const SDK_VERSION: &str = "0.1.0";

/// A live worker: protocol lockstep with deadline enforcement.
pub struct TsWorker {
    child: Child,
    stdin: ChildStdin,
    rx: mpsc::Receiver<std::io::Result<String>>,
    /// Handshake payload (bun version, sdk version, loaded rules).
    pub ready: ReadyPayload,
    next_run: u64,
}

impl TsWorker {
    /// Spawns `bun <worker_entry>` with `NODE_PATH` pointed at the cache
    /// dir (so user rules resolve `@suspect/rules-sdk`), sends `hello`,
    /// and awaits `ready`.
    ///
    /// # Errors
    /// [`Error::BunUnavailable`] when the binary cannot spawn,
    /// [`Error::Protocol`] on handshake violations, [`Error::RuleLoad`]
    /// when a rule file fails to import, [`Error::WorkerDied`] on exit.
    pub async fn spawn(
        bun: &Path,
        worker_entry: &Path,
        cache_dir: &Path,
        workspace_root: &Path,
        rule_files: &[PathBuf],
    ) -> Result<Self> {
        let mut child = Command::new(bun)
            .arg(worker_entry)
            .current_dir(workspace_root)
            .env("NODE_PATH", cache_dir.join("node_modules"))
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            // Inherited: rule authors' console.error and crash traces reach
            // the terminal; also prevents a full stderr pipe from blocking.
            .stderr(Stdio::inherit())
            .kill_on_drop(true)
            .spawn()
            .map_err(|e| Error::BunUnavailable(format!("spawn {bun:?}: {e}")))?;

        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| Error::Protocol("worker stdin unavailable".to_owned()))?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| Error::Protocol("worker stdout unavailable".to_owned()))?;
        let (tx, rx) = mpsc::channel(64);
        tokio::spawn(async move {
            // 1MB buffer: finding batches are single large lines; the
            // default 8KB capacity makes read_until re-copy the growing
            // buffer per 8KB (quadratic at finding volume).
            let mut lines = BufReader::with_capacity(1024 * 1024, stdout).lines();
            loop {
                match lines.next_line().await {
                    Ok(Some(line)) => {
                        if tx.send(Ok(line)).await.is_err() {
                            break;
                        }
                    }
                    Ok(None) => break,
                    Err(e) => {
                        let _ = tx.send(Err(e)).await;
                        break;
                    }
                }
            }
        });

        let mut worker = Self {
            child,
            stdin,
            rx,
            ready: ReadyPayload {
                bun: String::new(),
                sdk: String::new(),
                rules: Vec::new(),
            },
            next_run: 1,
        };

        let files: Vec<String> = rule_files
            .iter()
            .map(|p| p.to_string_lossy().into_owned())
            .collect();
        worker
            .send(&HostFrame::Hello {
                protocol: PROTOCOL_VERSION,
                sdk_version: SDK_VERSION.to_owned(),
                workspace_root: workspace_root.to_string_lossy().into_owned(),
                rule_files: files,
            })
            .await?;

        let ready = timeout(Duration::from_secs(30), worker.next_frame()).await;
        match ready {
            Ok(Ok(WorkerFrame::Ready(payload))) => {
                worker.ready = payload;
                Ok(worker)
            }
            Ok(Ok(WorkerFrame::Fatal(body))) => Err(Error::RuleLoad(body.message)),
            Ok(Ok(other)) => Err(Error::Protocol(format!(
                "expected ready, got {}",
                frame_name(&other)
            ))),
            Ok(Err(e)) => Err(Error::Io(e)),
            Err(_) => Err(Error::WorkerDied("handshake timed out".to_owned())),
        }
    }

    /// Rule metadata from the last (re)load.
    #[must_use]
    pub fn rules(&self) -> &[RuleMeta] {
        &self.ready.rules
    }

    async fn send(&mut self, frame: &HostFrame) -> Result<()> {
        let t0 = std::time::Instant::now();
        let mut line = serde_json::to_string(frame)?;
        line.push('\n');
        let t1 = t0.elapsed();
        self.stdin.write_all(line.as_bytes()).await?;
        self.stdin.flush().await?;
        let t2 = t0.elapsed();
        if std::env::var_os("SUSPECT_RULES_DEBUG").is_some() {
            eprintln!(
                "[host] send: serialize {t1:?}, write+flush {:?} ({} KB)",
                t2 - t1,
                line.len() / 1024
            );
        }
        Ok(())
    }

    async fn next_frame(&mut self) -> std::io::Result<WorkerFrame> {
        loop {
            let line = self.rx.recv().await.ok_or_else(|| {
                std::io::Error::new(std::io::ErrorKind::UnexpectedEof, "worker stdout closed")
            })??;
            let trimmed = line.trim();
            if trimmed.is_empty() {
                continue;
            }
            break serde_json::from_str(trimmed).map_err(|e| {
                std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    format!("unparseable worker frame: {e}"),
                )
            });
        }
    }

    /// Runs one evaluate round: selections (point rules) + walks (walk
    /// rules). Returns findings, per-rule errors, and the done frame.
    /// Enforces `deadline`; on breach the worker is killed and
    /// [`WorkerError::Timeout`] is returned (caller restarts).
    ///
    /// # Errors
    /// [`Error`] variants from protocol/IO failures; timeouts surface as
    /// [`WorkerError::Timeout`] inside [`Error::Worker`]-adjacent paths —
    /// see [`evaluate_round`].
    pub async fn evaluate(
        &mut self,
        doc_kind: crate::protocol::TargetKind,
        doc_uri: Option<String>,
        document: crate::protocol::RunDocument,
        selections: BTreeMap<String, Vec<String>>,
        walks: BTreeMap<String, crate::protocol::WalkEntry>,
        deadline: Duration,
    ) -> std::result::Result<EvaluateOutcome, WorkerError> {
        let run_id = self.next_run;
        self.next_run += 1;
        self.send(&HostFrame::Evaluate {
            document,
            run_id,
            timeout_ms: u64::try_from(deadline.as_millis()).unwrap_or(u64::MAX),
            doc_kind,
            doc_uri,
            selections,
            walks,
        })
        .await
        .map_err(|e| WorkerError::Fatal(e.to_string()))?;

        let mut findings = Vec::new();
        let mut rule_errors = Vec::new();
        #[allow(unused_assignments)]
        let mut done: Option<DoneBody> = None;
        // One deadline for the WHOLE run (per-frame resets would let a
        // trickle of frames run forever).
        let run_deadline = tokio::time::Instant::now() + deadline;
        let t_sent = std::time::Instant::now();
        if std::env::var_os("SUSPECT_RULES_DEBUG").is_some() {
            eprintln!("[host] evaluate sent at +{:?}", t_sent.elapsed());
        }

        loop {
            let frame = match timeout_at(run_deadline, self.next_frame()).await {
                Ok(Ok(f)) => {
                    if std::env::var_os("SUSPECT_RULES_DEBUG").is_some() {
                        eprintln!("[host] frame {} at +{:?}", frame_name(&f), t_sent.elapsed());
                    }
                    f
                }
                Ok(Err(e)) => return Err(WorkerError::Fatal(e.to_string())),
                Err(_) => return Err(WorkerError::Timeout { run_id }),
            };
            match frame {
                WorkerFrame::FindingsBatch(batch) if batch.run_id == run_id => {
                    findings.extend(batch.findings);
                }
                WorkerFrame::Finding(body) if body.run_id == run_id => {
                    findings.push(body);
                }
                WorkerFrame::RuleError(body) if body.run_id == run_id => {
                    rule_errors.push(body);
                }
                WorkerFrame::Done(body) if body.run_id == run_id => {
                    if std::env::var_os("SUSPECT_RULES_DEBUG").is_some() {
                        eprintln!("[host] done received at {:?}", std::time::Instant::now());
                    }
                    done = Some(body);
                    break;
                }
                WorkerFrame::Fatal(body) => return Err(WorkerError::Fatal(body.message)),
                // Frames from a previous (timed-out) run: discard.
                _ => {}
            }
        }

        let done = done.ok_or_else(|| WorkerError::Fatal("done missing".to_owned()))?;
        Ok(EvaluateOutcome {
            run_id,
            findings,
            rule_errors,
            done,
        })
    }

    /// Hot-reloads rule files; returns the new rule set.
    ///
    /// # Errors
    /// Protocol or IO failures; reload failures are fatal frames.
    pub async fn reload(&mut self, files: &[PathBuf]) -> Result<ReadyPayload> {
        let files: Vec<String> = files
            .iter()
            .map(|p| p.to_string_lossy().into_owned())
            .collect();
        self.send(&HostFrame::Reload { files }).await?;
        match timeout(Duration::from_secs(30), self.next_frame()).await {
            Ok(Ok(WorkerFrame::Ready(payload))) => {
                self.ready = payload.clone();
                Ok(payload)
            }
            Ok(Ok(WorkerFrame::Fatal(body))) => Err(Error::RuleLoad(body.message)),
            Ok(Ok(other)) => Err(Error::Protocol(format!(
                "expected ready after reload, got {}",
                frame_name(&other)
            ))),
            Ok(Err(e)) => Err(Error::Io(e)),
            Err(_) => Err(Error::WorkerDied("reload timed out".to_owned())),
        }
    }

    /// Kills the worker tree.
    ///
    /// # Errors
    /// IO failure killing the process (best-effort: errors after a
    /// successful signal are swallowed).
    pub async fn kill(&mut self) -> Result<()> {
        let _ = self.child.start_kill();
        let _ = self.child.wait().await;
        Ok(())
    }
}

/// Outcome of one successful evaluate round.
#[derive(Debug)]
pub struct EvaluateOutcome {
    /// Run id (matches the frame).
    pub run_id: u64,
    /// Findings in worker emission order (host sorts before publish).
    pub findings: Vec<crate::protocol::FindingBody>,
    /// Rules that threw during the run.
    pub rule_errors: Vec<crate::protocol::RuleErrorBody>,
    /// Completion frame with worker-measured duration.
    pub done: DoneBody,
}

/// Evaluate failures that map to host-side policy (restart vs disable).
#[derive(Debug, thiserror::Error)]
pub enum WorkerError {
    /// Deadline breached; the worker was killed and must be restarted.
    #[error("evaluate timed out (run {run_id})")]
    Timeout {
        /// The abandoned run id.
        run_id: u64,
    },
    /// Worker-level failure (crash, closed stdout, protocol violation).
    #[error("worker failed: {0}")]
    Fatal(String),
}

fn frame_name(frame: &WorkerFrame) -> &'static str {
    match frame {
        WorkerFrame::Ready(_) => "ready",
        WorkerFrame::Finding(_) => "finding",
        WorkerFrame::FindingsBatch(_) => "findings_batch",
        WorkerFrame::Done(_) => "done",
        WorkerFrame::RuleError(_) => "rule_error",
        WorkerFrame::Fatal(_) => "fatal",
        WorkerFrame::Pong => "pong",
    }
}
