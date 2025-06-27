use crate::{DocumentId, PeerId, StorageId, actors::messages::SyncMessage};
use std::{collections::HashMap, str::FromStr};

/// Metadata sent in join or peer messages
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PeerMetadata {
    /// The storage ID of this peer
    pub storage_id: Option<StorageId>,
    /// Whether the sender expects to connect again with this storage ID
    pub is_ephemeral: bool,
}

/// Information about document heads for a storage ID
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HeadsInfo {
    /// The heads of the document for the given storage ID as
    /// a list of base64 encoded SHA2 hashes
    pub heads: Vec<String>,
    /// The local time on the node which initially sent the remote-heads-changed
    /// message as milliseconds since the unix epoch
    pub timestamp: u64,
}

/// The format of messages sent over the wire
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WireMessage {
    /// Sent by the initiating peer in the handshake phase
    Join {
        sender_id: PeerId,
        supported_protocol_versions: Vec<String>,
        metadata: Option<PeerMetadata>,
    },
    /// Sent by the receiving peer in response to the join message in the handshake phase
    Peer {
        sender_id: PeerId,
        selected_protocol_version: String,
        target_id: PeerId,
        metadata: Option<PeerMetadata>,
    },
    /// An advisory message sent by a peer when they are planning to disconnect
    Leave { sender_id: PeerId },
    /// Sent when the senderId is asking to begin sync for the given documentId
    Request {
        document_id: DocumentId,
        sender_id: PeerId,
        target_id: PeerId,
        data: Vec<u8>,
    },
    /// Sent any time either peer wants to send a sync message about a given document
    Sync {
        document_id: DocumentId,
        sender_id: PeerId,
        target_id: PeerId,
        data: Vec<u8>,
    },
    /// Sent when a peer wants to indicate it doesn't have a given document
    DocUnavailable {
        sender_id: PeerId,
        target_id: PeerId,
        document_id: DocumentId,
    },
    /// Sent when a peer wants to send an ephemeral message to another peer
    Ephemeral {
        sender_id: PeerId,
        target_id: PeerId,
        count: u64,
        session_id: String,
        document_id: DocumentId,
        data: Vec<u8>,
    },
    /// Sent to inform the other end that there has been a protocol error
    Error { message: String },
    /// Sent when the sender wishes to change the set of storage IDs they wish to be notified of
    RemoteSubscriptionChange {
        sender_id: PeerId,
        target_id: PeerId,
        add: Option<Vec<StorageId>>,
        remove: Vec<StorageId>,
    },
    /// Sent when the sender wishes to inform the receiver that a peer has changed heads
    RemoteHeadsChanged {
        sender_id: PeerId,
        target_id: PeerId,
        document_id: DocumentId,
        new_heads: HashMap<StorageId, HeadsInfo>,
    },
}

impl WireMessage {
    pub fn decode(data: &[u8]) -> Result<Self, DecodeError> {
        let mut decoder = minicbor::Decoder::new(data);

        // Read the map length
        let len = decoder.map()?.ok_or(DecodeError::MissingLen)?;

        // Collect all fields from the map
        let mut fields = HashMap::new();
        for _ in 0..len {
            let key = decoder.str()?.to_string();
            match key.as_str() {
                "type" => {
                    fields.insert(key, FieldValue::String(decoder.str()?.to_string()));
                }
                "senderId" | "targetId" | "selectedProtocolVersion" | "message" | "sessionId" => {
                    fields.insert(key, FieldValue::String(decoder.str()?.to_string()));
                }
                "documentId" => {
                    if decoder.probe().str().is_ok() {
                        fields.insert(key, FieldValue::String(decoder.str()?.to_string()));
                    } else {
                        fields.insert(key, FieldValue::Bytes(decoder.bytes()?.to_vec()));
                    }
                }
                "supportedProtocolVersions" | "add" | "remove" => {
                    let array_len = decoder.array()?.ok_or(DecodeError::InvalidFormat)?;
                    let mut strings = Vec::new();
                    for _ in 0..array_len {
                        strings.push(decoder.str()?.to_string());
                    }
                    fields.insert(key, FieldValue::StringArray(strings));
                }
                "data" => {
                    fields.insert(key, FieldValue::Bytes(decoder.bytes()?.to_vec()));
                }
                "count" | "timestamp" => {
                    fields.insert(key, FieldValue::Uint(decoder.u64()?));
                }
                "metadata" => {
                    let metadata = decode_metadata(&mut decoder)?;
                    fields.insert(key, FieldValue::Metadata(metadata));
                }
                "newHeads" => {
                    let new_heads = decode_new_heads(&mut decoder)?;
                    fields.insert(key, FieldValue::NewHeads(new_heads));
                }
                _ => {
                    decoder.skip()?;
                }
            }
        }

        let message_type = fields
            .get("type")
            .and_then(|v| v.as_string())
            .ok_or(DecodeError::MissingType)?;

        match message_type.as_str() {
            "join" => {
                let sender_id = get_peer_id(&fields, "senderId")?;
                let supported_versions = fields
                    .get("supportedProtocolVersions")
                    .and_then(|v| v.as_string_array())
                    .ok_or(DecodeError::MissingField(
                        "supportedProtocolVersions".to_string(),
                    ))?
                    .clone();
                let metadata = fields
                    .get("metadata")
                    .and_then(|v| v.as_metadata())
                    .cloned();

                Ok(Self::Join {
                    sender_id,
                    supported_protocol_versions: supported_versions,
                    metadata,
                })
            }
            "peer" => {
                let sender_id = get_peer_id(&fields, "senderId")?;
                let target_id = get_peer_id(&fields, "targetId")?;
                let selected_version = fields
                    .get("selectedProtocolVersion")
                    .and_then(|v| v.as_string())
                    .ok_or(DecodeError::MissingField(
                        "selectedProtocolVersion".to_string(),
                    ))?
                    .clone();
                let metadata = fields
                    .get("metadata")
                    .and_then(|v| v.as_metadata())
                    .cloned();

                Ok(Self::Peer {
                    sender_id,
                    selected_protocol_version: selected_version,
                    target_id,
                    metadata,
                })
            }
            "leave" => {
                let sender_id = get_peer_id(&fields, "senderId")?;
                Ok(Self::Leave { sender_id })
            }
            "request" => {
                let document_id = get_document_id(&fields, "documentId")?;
                let sender_id = get_peer_id(&fields, "senderId")?;
                let target_id = get_peer_id(&fields, "targetId")?;
                let data = fields
                    .get("data")
                    .and_then(|v| v.as_bytes())
                    .ok_or(DecodeError::MissingField("data".to_string()))?
                    .clone();

                Ok(Self::Request {
                    document_id,
                    sender_id,
                    target_id,
                    data,
                })
            }
            "sync" => {
                let document_id = get_document_id(&fields, "documentId")?;
                let sender_id = get_peer_id(&fields, "senderId")?;
                let target_id = get_peer_id(&fields, "targetId")?;
                let data = fields
                    .get("data")
                    .and_then(|v| v.as_bytes())
                    .ok_or(DecodeError::MissingField("data".to_string()))?
                    .clone();

                Ok(Self::Sync {
                    document_id,
                    sender_id,
                    target_id,
                    data,
                })
            }
            "doc-unavailable" => {
                let sender_id = get_peer_id(&fields, "senderId")?;
                let target_id = get_peer_id(&fields, "targetId")?;
                let document_id = get_document_id(&fields, "documentId")?;

                Ok(Self::DocUnavailable {
                    sender_id,
                    target_id,
                    document_id,
                })
            }
            "ephemeral" => {
                let sender_id = get_peer_id(&fields, "senderId")?;
                let target_id = get_peer_id(&fields, "targetId")?;
                let count = fields
                    .get("count")
                    .and_then(|v| v.as_uint())
                    .ok_or(DecodeError::MissingField("count".to_string()))?;
                let session_id = fields
                    .get("sessionId")
                    .and_then(|v| v.as_string())
                    .ok_or(DecodeError::MissingField("sessionId".to_string()))?
                    .clone();
                let document_id = get_document_id(&fields, "documentId")?;
                let data = fields
                    .get("data")
                    .and_then(|v| v.as_bytes())
                    .ok_or(DecodeError::MissingField("data".to_string()))?
                    .clone();

                Ok(Self::Ephemeral {
                    sender_id,
                    target_id,
                    count,
                    session_id,
                    document_id,
                    data,
                })
            }
            "error" => {
                let message = fields
                    .get("message")
                    .and_then(|v| v.as_string())
                    .ok_or(DecodeError::MissingField("message".to_string()))?
                    .clone();

                Ok(Self::Error { message })
            }
            "remote-subscription-change" => {
                let sender_id = get_peer_id(&fields, "senderId")?;
                let target_id = get_peer_id(&fields, "targetId")?;
                let add = fields
                    .get("add")
                    .and_then(|v| v.as_string_array())
                    .map(|strings| {
                        strings
                            .iter()
                            .map(|s| StorageId::from(s.as_str()))
                            .collect::<Vec<_>>()
                    });
                let remove = fields
                    .get("remove")
                    .and_then(|v| v.as_string_array())
                    .ok_or(DecodeError::MissingField("remove".to_string()))?
                    .iter()
                    .map(|s| StorageId::from(s.as_str()))
                    .collect::<Vec<_>>();

                Ok(Self::RemoteSubscriptionChange {
                    sender_id,
                    target_id,
                    add,
                    remove,
                })
            }
            "remote-heads-changed" => {
                let sender_id = get_peer_id(&fields, "senderId")?;
                let target_id = get_peer_id(&fields, "targetId")?;
                let document_id = get_document_id(&fields, "documentId")?;
                let new_heads = fields
                    .get("newHeads")
                    .and_then(|v| v.as_new_heads())
                    .ok_or(DecodeError::MissingField("newHeads".to_string()))?
                    .clone();

                Ok(Self::RemoteHeadsChanged {
                    sender_id,
                    target_id,
                    document_id,
                    new_heads,
                })
            }
            other => Err(DecodeError::UnknownType(other.to_string())),
        }
    }

    pub fn encode(&self) -> Vec<u8> {
        self.encode_inner().unwrap()
    }

    fn encode_inner(&self) -> Result<Vec<u8>, EncodeError> {
        let mut encoder = minicbor::Encoder::new(Vec::<u8>::new());

        match self {
            Self::Join {
                sender_id,
                supported_protocol_versions,
                metadata,
            } => {
                let field_count = if metadata.is_some() { 4 } else { 3 };
                encoder.map(field_count)?;
                encoder.str("type")?.str("join")?;
                encoder.str("senderId")?.str(&sender_id.to_string())?;
                encoder.str("supportedProtocolVersions")?;
                encoder.array(supported_protocol_versions.len() as u64)?;
                for version in supported_protocol_versions {
                    encoder.str(version)?;
                }
                if let Some(metadata) = metadata {
                    encoder.str("metadata")?;
                    encode_metadata(&mut encoder, metadata)
                        .map_err(|e| EncodeError::Minicbor(format!("{e:?}")))?;
                }
            }
            Self::Peer {
                sender_id,
                selected_protocol_version,
                target_id,
                metadata,
            } => {
                let field_count = if metadata.is_some() { 5 } else { 4 };
                encoder.map(field_count)?;
                encoder.str("type")?.str("peer")?;
                encoder.str("senderId")?.str(&sender_id.to_string())?;
                encoder
                    .str("selectedProtocolVersion")?
                    .str(selected_protocol_version)?;
                encoder.str("targetId")?.str(&target_id.to_string())?;
                if let Some(metadata) = metadata {
                    encoder.str("metadata")?;
                    encode_metadata(&mut encoder, metadata)
                        .map_err(|e| EncodeError::Minicbor(format!("{e:?}")))?;
                }
            }
            Self::Leave { sender_id } => {
                encoder.map(2)?;
                encoder.str("type")?.str("leave")?;
                encoder.str("senderId")?.str(&sender_id.to_string())?;
            }
            Self::Request {
                document_id,
                sender_id,
                target_id,
                data,
            } => {
                encoder.map(5)?;
                encoder.str("type")?.str("request")?;
                encoder.str("documentId")?.str(&document_id.to_string())?;
                encoder.str("senderId")?.str(&sender_id.to_string())?;
                encoder.str("targetId")?.str(&target_id.to_string())?;
                encoder.str("data")?.bytes(data)?;
            }
            Self::Sync {
                document_id,
                sender_id,
                target_id,
                data,
            } => {
                encoder.map(5)?;
                encoder.str("type")?.str("sync")?;
                encoder.str("documentId")?.str(&document_id.to_string())?;
                encoder.str("senderId")?.str(&sender_id.to_string())?;
                encoder.str("targetId")?.str(&target_id.to_string())?;
                encoder.str("data")?.bytes(data)?;
            }
            Self::DocUnavailable {
                sender_id,
                target_id,
                document_id,
            } => {
                encoder.map(4)?;
                encoder.str("type")?.str("doc-unavailable")?;
                encoder.str("senderId")?.str(&sender_id.to_string())?;
                encoder.str("targetId")?.str(&target_id.to_string())?;
                encoder.str("documentId")?.str(&document_id.to_string())?;
            }
            Self::Ephemeral {
                sender_id,
                target_id,
                count,
                session_id,
                document_id,
                data,
            } => {
                encoder.map(7)?;
                encoder.str("type")?.str("ephemeral")?;
                encoder.str("senderId")?.str(&sender_id.to_string())?;
                encoder.str("targetId")?.str(&target_id.to_string())?;
                encoder.str("count")?.u64(*count)?;
                encoder.str("sessionId")?.str(session_id)?;
                encoder.str("documentId")?.str(&document_id.to_string())?;
                encoder.str("data")?.bytes(data)?;
            }
            Self::Error { message } => {
                encoder.map(2)?;
                encoder.str("type")?.str("error")?;
                encoder.str("message")?.str(message)?;
            }
            Self::RemoteSubscriptionChange {
                sender_id,
                target_id,
                add,
                remove,
            } => {
                let field_count = if add.is_some() { 5 } else { 4 };
                encoder.map(field_count)?;
                encoder.str("type")?.str("remote-subscription-change")?;
                encoder.str("senderId")?.str(&sender_id.to_string())?;
                encoder.str("targetId")?.str(&target_id.to_string())?;
                if let Some(add_list) = add {
                    encoder.str("add")?;
                    encoder.array(add_list.len() as u64)?;
                    for storage_id in add_list {
                        encoder.str(&storage_id.to_string())?;
                    }
                }
                encoder.str("remove")?;
                encoder.array(remove.len() as u64)?;
                for storage_id in remove {
                    encoder.str(&storage_id.to_string())?;
                }
            }
            Self::RemoteHeadsChanged {
                sender_id,
                target_id,
                document_id,
                new_heads,
            } => {
                encoder.map(5)?;
                encoder.str("type")?.str("remote-heads-changed")?;
                encoder.str("senderId")?.str(&sender_id.to_string())?;
                encoder.str("targetId")?.str(&target_id.to_string())?;
                encoder.str("documentId")?.str(&document_id.to_string())?;
                encoder.str("newHeads")?;
                encode_new_heads(&mut encoder, new_heads)
                    .map_err(|e| EncodeError::Minicbor(format!("{e:?}")))?;
            }
        }

        Ok(encoder.into_writer())
    }
}

// Helper enum for decoding field values
#[derive(Debug, Clone)]
enum FieldValue {
    String(String),
    StringArray(Vec<String>),
    Bytes(Vec<u8>),
    Uint(u64),
    Metadata(PeerMetadata),
    NewHeads(HashMap<StorageId, HeadsInfo>),
}

impl FieldValue {
    fn as_string(&self) -> Option<&String> {
        match self {
            FieldValue::String(s) => Some(s),
            _ => None,
        }
    }

    fn as_string_array(&self) -> Option<&Vec<String>> {
        match self {
            FieldValue::StringArray(arr) => Some(arr),
            _ => None,
        }
    }

    fn as_bytes(&self) -> Option<&Vec<u8>> {
        match self {
            FieldValue::Bytes(b) => Some(b),
            _ => None,
        }
    }

    fn as_uint(&self) -> Option<u64> {
        match self {
            FieldValue::Uint(u) => Some(*u),
            _ => None,
        }
    }

    fn as_metadata(&self) -> Option<&PeerMetadata> {
        match self {
            FieldValue::Metadata(m) => Some(m),
            _ => None,
        }
    }

    fn as_new_heads(&self) -> Option<&HashMap<StorageId, HeadsInfo>> {
        match self {
            FieldValue::NewHeads(h) => Some(h),
            _ => None,
        }
    }
}

// Helper functions
fn get_peer_id(fields: &HashMap<String, FieldValue>, key: &str) -> Result<PeerId, DecodeError> {
    let peer_id_str = fields
        .get(key)
        .and_then(|v| v.as_string())
        .ok_or_else(|| DecodeError::MissingField(key.to_string()))?;
    Ok(PeerId::from(peer_id_str.as_str()))
}

fn get_document_id(
    fields: &HashMap<String, FieldValue>,
    key: &str,
) -> Result<DocumentId, DecodeError> {
    let field = fields
        .get(key)
        .ok_or_else(|| DecodeError::MissingField(key.to_string()))?;
    match field {
        FieldValue::String(s) => {
            DocumentId::from_str(s).map_err(|_| DecodeError::InvalidDocumentId)
        }
        FieldValue::Bytes(b) => {
            DocumentId::try_from(b.to_vec()).map_err(|_| DecodeError::InvalidDocumentId)
        }
        _ => Err(DecodeError::InvalidDocumentId),
    }
}

fn decode_metadata(decoder: &mut minicbor::Decoder) -> Result<PeerMetadata, DecodeError> {
    let len = decoder.map()?.ok_or(DecodeError::InvalidFormat)?;
    let mut storage_id = None;
    let mut is_ephemeral = false;

    for _ in 0..len {
        match decoder.str()? {
            "storageId" => {
                let storage_id_str = decoder.str()?;
                storage_id = Some(StorageId::from(storage_id_str));
            }
            "isEphemeral" => {
                is_ephemeral = decoder.bool()?;
            }
            _ => {
                decoder.skip()?;
            }
        }
    }

    Ok(PeerMetadata {
        storage_id,
        is_ephemeral,
    })
}

fn encode_metadata<W: minicbor::encode::Write>(
    encoder: &mut minicbor::Encoder<W>,
    metadata: &PeerMetadata,
) -> Result<(), minicbor::encode::Error<W::Error>> {
    let field_count = if metadata.storage_id.is_some() { 2 } else { 1 };
    encoder.map(field_count)?;
    if let Some(storage_id) = &metadata.storage_id {
        encoder.str("storageId")?.str(&storage_id.to_string())?;
    }
    encoder.str("isEphemeral")?.bool(metadata.is_ephemeral)?;
    Ok(())
}

fn decode_new_heads(
    decoder: &mut minicbor::Decoder,
) -> Result<HashMap<StorageId, HeadsInfo>, DecodeError> {
    let len = decoder.map()?.ok_or(DecodeError::InvalidFormat)?;
    let mut new_heads = HashMap::new();

    for _ in 0..len {
        let storage_id_str = decoder.str()?;
        let storage_id = StorageId::from(storage_id_str);

        let heads_len = decoder.map()?.ok_or(DecodeError::InvalidFormat)?;
        let mut heads = Vec::new();
        let mut timestamp = 0;

        for _ in 0..heads_len {
            match decoder.str()? {
                "heads" => {
                    let heads_array_len = decoder.array()?.ok_or(DecodeError::InvalidFormat)?;
                    for _ in 0..heads_array_len {
                        heads.push(decoder.str()?.to_string());
                    }
                }
                "timestamp" => {
                    timestamp = decoder.u64()?;
                }
                _ => {
                    decoder.skip()?;
                }
            }
        }

        new_heads.insert(storage_id, HeadsInfo { heads, timestamp });
    }

    Ok(new_heads)
}

fn encode_new_heads<W: minicbor::encode::Write>(
    encoder: &mut minicbor::Encoder<W>,
    new_heads: &HashMap<StorageId, HeadsInfo>,
) -> Result<(), minicbor::encode::Error<W::Error>> {
    encoder.map(new_heads.len() as u64)?;
    for (storage_id, heads_info) in new_heads {
        encoder.str(&storage_id.to_string())?;
        encoder.map(2)?;
        encoder.str("heads")?;
        encoder.array(heads_info.heads.len() as u64)?;
        for head in &heads_info.heads {
            encoder.str(head)?;
        }
        encoder.str("timestamp")?.u64(heads_info.timestamp)?;
    }
    Ok(())
}

#[derive(Debug, thiserror::Error)]
pub enum DecodeError {
    #[error("missing len")]
    MissingLen,
    #[error("invalid format")]
    InvalidFormat,
    #[error("{0}")]
    Minicbor(String),
    #[error("no type field")]
    MissingType,
    #[error("missing field: {0}")]
    MissingField(String),
    #[error("invalid document ID")]
    InvalidDocumentId,
    #[error("unknown type {0}")]
    UnknownType(String),
}

impl From<minicbor::decode::Error> for DecodeError {
    fn from(e: minicbor::decode::Error) -> Self {
        Self::Minicbor(e.to_string())
    }
}

#[derive(Debug, thiserror::Error)]
pub enum EncodeError {
    #[error("{0}")]
    Minicbor(String),
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
}

impl<T> From<minicbor::encode::Error<T>> for EncodeError
where
    T: std::fmt::Display,
{
    fn from(e: minicbor::encode::Error<T>) -> Self {
        Self::Minicbor(e.to_string())
    }
}

pub(crate) struct WireMessageBuilder {
    pub(crate) sender_id: PeerId,
    pub(crate) target_id: PeerId,
    pub(crate) document_id: DocumentId,
}

impl WireMessageBuilder {
    #[allow(clippy::wrong_self_convention)]
    pub(crate) fn from_sync_message(self, msg: SyncMessage) -> WireMessage {
        match msg {
            SyncMessage::Request { data } => WireMessage::Request {
                document_id: self.document_id,
                sender_id: self.sender_id,
                target_id: self.target_id,
                data,
            },
            SyncMessage::Sync { data } => WireMessage::Sync {
                document_id: self.document_id,
                sender_id: self.sender_id,
                target_id: self.target_id,
                data,
            },
            SyncMessage::DocUnavailable => WireMessage::DocUnavailable {
                sender_id: self.sender_id,
                target_id: self.target_id,
                document_id: self.document_id,
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_join_message_roundtrip() {
        let msg = WireMessage::Join {
            sender_id: PeerId::from("test-peer"),
            supported_protocol_versions: vec!["1".to_string()],
            metadata: Some(PeerMetadata {
                storage_id: Some(StorageId::new(&mut rand::rng())),
                is_ephemeral: false,
            }),
        };

        let encoded = msg.encode();
        let decoded = WireMessage::decode(&encoded).unwrap();
        assert_eq!(msg, decoded);
    }

    #[test]
    fn test_peer_message_roundtrip() {
        let msg = WireMessage::Peer {
            sender_id: PeerId::from("sender"),
            selected_protocol_version: "1".to_string(),
            target_id: PeerId::from("target"),
            metadata: None,
        };

        let encoded = msg.encode();
        let decoded = WireMessage::decode(&encoded).unwrap();
        assert_eq!(msg, decoded);
    }

    #[test]
    fn test_error_message_roundtrip() {
        let msg = WireMessage::Error {
            message: "Protocol error".to_string(),
        };

        let encoded = msg.encode();
        let decoded = WireMessage::decode(&encoded).unwrap();
        assert_eq!(msg, decoded);
    }
}
