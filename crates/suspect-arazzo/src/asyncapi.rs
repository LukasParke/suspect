//! Minimal AsyncAPI channel/message reader for Arazzo 1.1 message steps.
//!
//! Reads the subset an Arazzo workflow needs: the channels a step's
//! `channelPath` addresses, the messages published/subscribed on them, and
//! the JSON Schema each message payload declares. Payload schemas are
//! resolved from `components/schemas` (AsyncAPI 2.x/3.x `$ref` spelling)
//! into owned JSON so the same schema compiler validates received
//! payloads.

use suspect_low::{LowDoc, NodeRef, ValueKind};

/// Which way a channel message flows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MessageDirection {
    /// The producer publishes it (`publish`/`send`).
    Outgoing,
    /// The consumer subscribes to it (`subscribe`/`receive`).
    Incoming,
}

/// One message definition on a channel.
#[derive(Debug, Clone, PartialEq)]
pub struct ChannelMessage {
    /// Message name, used to address a specific message type.
    pub name: String,
    /// The direction this message flows in.
    pub direction: MessageDirection,
    /// The resolved payload JSON Schema, when the message declares one.
    pub payload: Option<serde_json::Value>,
    /// The message's `correlationId` location expression, when declared.
    pub correlation_location: Option<String>,
}

/// One addressable channel.
#[derive(Debug, Clone, PartialEq)]
pub struct Channel {
    /// The channel address (`orders`, `orders.created`, …).
    pub address: String,
    /// Messages published (`send`) or subscribed (`receive`).
    pub messages: Vec<ChannelMessage>,
}

impl Channel {
    /// The message a step of `direction` addresses: the one named `name`
    /// when given, else the first message flowing that way.
    #[must_use]
    pub fn message(
        &self,
        direction: MessageDirection,
        name: Option<&str>,
    ) -> Option<&ChannelMessage> {
        // A channel declaring only one direction still serves both sides
        // of a round trip (a request/reply channel), so fall back to every
        // message when nothing flows the requested way.
        let flows = self.messages.iter().any(|m| m.direction == direction);
        let matches_direction = |m: &ChannelMessage| !flows || m.direction == direction;
        match name {
            Some(name) => self
                .messages
                .iter()
                .find(|m| m.name == name && matches_direction(m))
                .or_else(|| self.messages.iter().find(|m| matches_direction(m))),
            None => self.messages.iter().find(|m| matches_direction(m)),
        }
    }
}

/// A parsed AsyncAPI document.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct AsyncApiDoc {
    channels: Vec<Channel>,
}

/// A failure while reading an AsyncAPI document.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AsyncApiError(pub String);

impl std::fmt::Display for AsyncApiError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "asyncapi: {}", self.0)
    }
}

impl std::error::Error for AsyncApiError {}

impl AsyncApiDoc {
    /// Reads the channels from an AsyncAPI document.
    ///
    /// # Errors
    /// Returns an error when `channels` is absent or malformed.
    pub fn parse(doc: &LowDoc) -> Result<Self, AsyncApiError> {
        let root = doc.root();
        let Some(channels) = root.get("channels") else {
            return Err(AsyncApiError("missing `channels`".to_owned()));
        };
        if channels.kind() != ValueKind::Object {
            return Err(AsyncApiError("`channels` must be a map".to_owned()));
        }
        let mut parsed = Vec::new();
        for entry in channels.entries() {
            let Some(channel_node) = entry.value else {
                continue;
            };
            let address = channel_node
                .get("address")
                .and_then(|a| a.as_str())
                .unwrap_or(entry.key)
                .to_owned();
            let mut messages = Vec::new();
            // AsyncAPI 2.x: publish/subscribe; 3.x: send/receive.
            for (key, flow) in [
                ("publish", MessageDirection::Outgoing),
                ("send", MessageDirection::Outgoing),
                ("subscribe", MessageDirection::Incoming),
                ("receive", MessageDirection::Incoming),
            ] {
                let Some(list) = channel_node.get(key) else {
                    continue;
                };
                for item in list.items() {
                    let name = item
                        .get("name")
                        .and_then(|n| n.as_str())
                        .unwrap_or("message")
                        .to_owned();
                    let correlation_location = item.get("correlationId").and_then(|c| {
                        c.get("location")
                            .and_then(|l| l.as_str())
                            .map(str::to_owned)
                            .or_else(|| c.as_str().map(str::to_owned))
                    });
                    let payload = item
                        .get("payload")
                        .map(|payload| resolve_schema(root, payload));
                    messages.push(ChannelMessage {
                        name,
                        direction: flow,
                        payload,
                        correlation_location,
                    });
                }
            }
            parsed.push(Channel { address, messages });
        }
        Ok(Self { channels: parsed })
    }

    /// The channel at `address`, matched on the channel key or its
    /// `address` value.
    #[must_use]
    pub fn channel(&self, address: &str) -> Option<&Channel> {
        self.channels.iter().find(|c| c.address == address)
    }

    /// Every declared channel address.
    #[must_use]
    pub fn addresses(&self) -> Vec<&str> {
        self.channels.iter().map(|c| c.address.as_str()).collect()
    }
}

/// Resolves a message payload node into owned JSON, inlining a
/// `components/schemas` `$ref` when the payload is a bare reference.
fn resolve_schema(root: NodeRef<'_>, payload: NodeRef<'_>) -> serde_json::Value {
    let json: serde_json::Value =
        serde_json::from_str(&suspect_overlay::Value::from_node(payload).to_json())
            .unwrap_or(serde_json::Value::Null);
    let Some(reference) = json.get("$ref").and_then(|r| r.as_str()) else {
        return json;
    };
    let Ok(pointer) = suspect_low::Pointer::parse(reference) else {
        return json;
    };
    root.pointer(&pointer)
        .map(|target| {
            serde_json::from_str(&suspect_overlay::Value::from_node(target).to_json())
                .unwrap_or(json.clone())
        })
        .unwrap_or(json)
}

/// A workflow's channel reference: the source description plus address.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChannelRef {
    /// The `sourceDescriptions` entry holding the AsyncAPI document.
    pub source: String,
    /// The channel address within that document.
    pub address: String,
}

/// Parses a `channelPath` expression (`$sourceDescriptions.<name>.<address>`).
#[must_use]
pub fn parse_channel_path(path: &str) -> Option<ChannelRef> {
    let rest = path
        .trim_start_matches('$')
        .trim_start_matches("sourceDescriptions.");
    let (source, address) = rest.split_once('.')?;
    if source.is_empty() || address.is_empty() {
        return None;
    }
    Some(ChannelRef {
        source: source.to_owned(),
        address: address.trim_start_matches('/').to_owned(),
    })
}
