//! Message transport for Arazzo 1.1 AsyncAPI send/receive steps.
//!
//! A workflow that mixes HTTP calls with message traffic needs one place to
//! publish and subscribe. [`MessageTransport`] is that seam; the executor
//! owns HTTP, the transport owns messages, and nothing else in the plan
//! compiler knows the difference.
//!
//! [`FileBroker`] is the shipped transport: a JSONL inbox/outbox on disk, so
//! an offline suite can drive `send` and satisfy `receive` deterministically
//! from recorded messages, and online runs can tee traffic into the same
//! file. Real brokers (NATS, Kafka, MQTT) are adapter implementations of
//! this trait and are not shipped.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

/// One message on a channel.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Message {
    /// Channel address the message was published on.
    pub channel: String,
    /// Optional message type name within the channel.
    pub message_type: Option<String>,
    /// Correlation id, matched against a receive step's `correlationId`.
    pub correlation_id: Option<String>,
    /// The decoded JSON payload.
    pub payload: serde_json::Value,
}

/// A message publish/subscribe failure.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MessageError {
    /// The transport has no broker or the inbox is unreadable.
    Unavailable(String),
    /// No matching message arrived before the deadline.
    Timeout {
        /// The channel that was waited on.
        channel: String,
        /// The correlation id that was required, when one was declared.
        correlation_id: Option<String>,
    },
}

impl std::fmt::Display for MessageError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Unavailable(message) => write!(f, "message transport unavailable: {message}"),
            Self::Timeout {
                channel,
                correlation_id,
            } => match correlation_id {
                Some(id) => write!(f, "no message on `{channel}` with correlation id `{id}`"),
                None => write!(f, "no message on `{channel}`"),
            },
        }
    }
}

impl std::error::Error for MessageError {}

/// Publishes and subscribes to channels.
pub trait MessageTransport: Send + Sync {
    /// Publishes one message.
    ///
    /// # Errors
    /// Returns [`MessageError::Unavailable`] when the broker rejects it.
    fn send(&self, message: Message) -> Result<(), MessageError>;

    /// Waits for the next message on `channel`, matching `correlation_id`
    /// when the step declares one. Consumed messages are not returned
    /// again.
    ///
    /// # Errors
    /// Returns [`MessageError::Timeout`] when nothing matches in time.
    fn receive(
        &self,
        channel: &str,
        correlation_id: Option<&str>,
        timeout: Duration,
    ) -> Result<Message, MessageError>;
}

/// A file-backed broker: `inbox.jsonl` holds pre-recorded messages,
/// `outbox.jsonl` records what the workflow published. Both are plain JSONL
/// so a suite is reviewable and diffable, and replay is deterministic.
pub struct FileBroker {
    inbox: Mutex<(Vec<Message>, usize)>,
    outbox: PathBuf,
}

impl FileBroker {
    /// Opens (or creates) a broker rooted at `dir`.
    ///
    /// # Errors
    /// Filesystem failures reading the inbox.
    pub fn open(dir: &Path) -> Result<Self, MessageError> {
        std::fs::create_dir_all(dir)
            .map_err(|e| MessageError::Unavailable(format!("{}: {e}", dir.display())))?;
        let inbox_path = dir.join("inbox.jsonl");
        let text = std::fs::read_to_string(&inbox_path).unwrap_or_default();
        let mut messages = Vec::new();
        for line in text.lines().filter(|line| !line.trim().is_empty()) {
            if let Ok(message) = serde_json::from_str::<Message>(line) {
                messages.push(message);
            }
        }
        Ok(Self {
            inbox: Mutex::new((messages, 0)),
            outbox: dir.join("outbox.jsonl"),
        })
    }

    /// Builds an in-memory broker from recorded messages (tests, and
    /// `--message-inbox` equivalents).
    #[must_use]
    pub fn from_messages(messages: Vec<Message>) -> Self {
        Self {
            inbox: Mutex::new((messages, 0)),
            outbox: PathBuf::from("/dev/null"),
        }
    }
}

impl MessageTransport for FileBroker {
    fn send(&self, message: Message) -> Result<(), MessageError> {
        let line = serde_json::to_string(&message)
            .map_err(|e| MessageError::Unavailable(e.to_string()))?;
        let mut file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.outbox)
            .map_err(|e| MessageError::Unavailable(format!("{}: {e}", self.outbox.display())))?;
        use std::io::Write;
        writeln!(file, "{line}")
            .map_err(|e| MessageError::Unavailable(format!("{}: {e}", self.outbox.display())))
    }

    fn receive(
        &self,
        channel: &str,
        correlation_id: Option<&str>,
        timeout: Duration,
    ) -> Result<Message, MessageError> {
        let deadline = Instant::now() + timeout;
        loop {
            let mut inbox = self.inbox.lock().expect("broker inbox lock");
            let (messages, cursor) = &mut *inbox;
            if let Some(index) = (*cursor..messages.len()).find(|i| {
                let candidate = &messages[*i];
                candidate.channel == channel
                    && correlation_id
                        .is_none_or(|wanted| candidate.correlation_id.as_deref() == Some(wanted))
            }) {
                *cursor = index + 1;
                return Ok(messages[index].clone());
            }
            drop(inbox);
            if Instant::now() >= deadline {
                return Err(MessageError::Timeout {
                    channel: channel.to_owned(),
                    correlation_id: correlation_id.map(str::to_owned),
                });
            }
            std::thread::sleep(Duration::from_millis(20));
        }
    }
}

/// A transport that records sends and replays a fixed message set, used by
/// tests and by `--message-inbox` runs without a real broker.
#[derive(Default)]
pub struct LoopbackBroker {
    /// Everything published through this broker, in order.
    pub sent: Mutex<Vec<Message>>,
    inbox: Mutex<BTreeMap<String, Vec<Message>>>,
}

impl LoopbackBroker {
    /// Creates a broker preloaded with per-channel messages.
    #[must_use]
    pub fn with_inbox(inbox: BTreeMap<String, Vec<Message>>) -> Self {
        Self {
            sent: Mutex::new(Vec::new()),
            inbox: Mutex::new(inbox),
        }
    }

    /// Every message published so far.
    #[must_use]
    pub fn published(&self) -> Vec<Message> {
        self.sent.lock().expect("broker sent lock").clone()
    }
}

impl MessageTransport for LoopbackBroker {
    fn send(&self, message: Message) -> Result<(), MessageError> {
        self.sent
            .lock()
            .expect("broker sent lock")
            .push(message.clone());
        self.inbox
            .lock()
            .expect("broker inbox lock")
            .entry(message.channel.clone())
            .or_default()
            .push(message);
        Ok(())
    }

    fn receive(
        &self,
        channel: &str,
        correlation_id: Option<&str>,
        _timeout: Duration,
    ) -> Result<Message, MessageError> {
        let mut inbox = self.inbox.lock().expect("broker inbox lock");
        let queue = inbox
            .get_mut(channel)
            .ok_or_else(|| MessageError::Timeout {
                channel: channel.to_owned(),
                correlation_id: correlation_id.map(str::to_owned),
            })?;
        let index = (0..queue.len()).find(|i| {
            correlation_id.is_none_or(|wanted| queue[*i].correlation_id.as_deref() == Some(wanted))
        });
        match index {
            Some(index) => Ok(queue.remove(index)),
            None => Err(MessageError::Timeout {
                channel: channel.to_owned(),
                correlation_id: correlation_id.map(str::to_owned),
            }),
        }
    }
}
