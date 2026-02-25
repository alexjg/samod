use opentelemetry::KeyValue;
use opentelemetry::metrics::{Counter, Gauge, Histogram, Meter};
use samod::{RepoEvent, RepoObserver, StorageOperation};

pub struct OtelObserver {
    documents_active: Gauge<f64>,
    peers_connected: Gauge<f64>,
    connections_total: Counter<u64>,
    sync_processing_millis: Histogram<f64>,
    sync_message_bytes: Histogram<f64>,
    storage_operation_millis: Histogram<f64>,
    hub_event_processing_millis: Histogram<f64>,
}

impl OtelObserver {
    pub fn new(meter: &Meter) -> Self {
        Self {
            documents_active: meter
                .f64_gauge("samod_documents_active")
                .with_description("Number of active document actors")
                .build(),
            peers_connected: meter
                .f64_gauge("samod_peers_connected")
                .with_description("Number of connected peers")
                .build(),
            connections_total: meter
                .u64_counter("samod_connections_total")
                .with_description("Total connection lifecycle events")
                .build(),
            sync_processing_millis: meter
                .f64_histogram("samod_sync_processing_millis")
                .with_description("Time spent processing sync messages")
                .with_unit("ms")
                .build(),
            sync_message_bytes: meter
                .f64_histogram("samod_sync_message_bytes")
                .with_description("Size of sync messages")
                .with_unit("By")
                .build(),
            storage_operation_millis: meter
                .f64_histogram("samod_storage_operation_millis")
                .with_description("Time spent on storage operations")
                .with_unit("ms")
                .build(),
            hub_event_processing_millis: meter
                .f64_histogram("samod_hub_event_processing_millis")
                .with_description("Time spent processing hub events")
                .with_unit("ms")
                .build(),
        }
    }
}

impl RepoObserver for OtelObserver {
    fn observe(&self, event: &RepoEvent) {
        match event {
            RepoEvent::DocumentOpened { .. } => {
                self.documents_active.record(1.0, &[]);
            }
            RepoEvent::DocumentClosed { .. } => {
                self.documents_active.record(-1.0, &[]);
            }
            RepoEvent::ConnectionEstablished { connection_id: _ } => {
                self.connections_total
                    .add(1, &[KeyValue::new("event", "connected")]);
                self.peers_connected.record(1.0, &[]);
            }
            RepoEvent::ConnectionLost { connection_id: _ } => {
                self.connections_total
                    .add(1, &[KeyValue::new("event", "disconnected")]);
                self.peers_connected.record(-1.0, &[]);
            }
            RepoEvent::SyncMessageReceived {
                document_id,
                bytes,
                duration,
                ..
            } => {
                let attrs = [
                    KeyValue::new("direction", "received"),
                    KeyValue::new("document_id", document_id.to_string()),
                ];
                self.sync_processing_millis
                    .record(duration.as_millis() as f64, &attrs);
                self.sync_message_bytes.record(*bytes as f64, &attrs[..1]);
            }
            RepoEvent::SyncMessageGenerated {
                document_id,
                bytes,
                duration,
                ..
            } => {
                let attrs = [
                    KeyValue::new("direction", "generated"),
                    KeyValue::new("document_id", document_id.to_string()),
                ];
                self.sync_processing_millis
                    .record(duration.as_millis() as f64, &attrs);
                self.sync_message_bytes.record(*bytes as f64, &attrs[..1]);
            }
            RepoEvent::StorageOperationCompleted {
                operation,
                duration,
            } => {
                let op = match operation {
                    StorageOperation::Load => "load",
                    StorageOperation::LoadRange => "load_range",
                    StorageOperation::Put => "put",
                    StorageOperation::Delete => "delete",
                };
                self.storage_operation_millis.record(
                    duration.as_millis() as f64,
                    &[KeyValue::new("operation", op)],
                );
            }
            RepoEvent::HubEventProcessed { duration } => {
                self.hub_event_processing_millis
                    .record(duration.as_millis() as f64, &[]);
            }
            _ => {}
        }
    }
}
