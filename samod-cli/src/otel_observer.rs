use std::sync::atomic::{AtomicI64, Ordering};

use opentelemetry::KeyValue;
use opentelemetry::metrics::{Counter, Gauge, Histogram, Meter};
use samod::{RepoEvent, RepoObserver, StorageOperation};

pub struct OtelObserver {
    documents_active: Gauge<f64>,
    documents_active_count: AtomicI64,
    peers_connected: Gauge<f64>,
    peers_connected_count: AtomicI64,
    connections_total: Counter<u64>,
    sync_processing_millis: Histogram<f64>,
    sync_queue_millis: Histogram<f64>,
    sync_message_bytes: Histogram<f64>,
    storage_operation_millis: Histogram<f64>,
    hub_event_processing_millis: Histogram<f64>,
    hub_connections: Gauge<f64>,
    hub_documents: Gauge<f64>,
    document_pending_sync_messages: Gauge<f64>,
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
            sync_queue_millis: meter
                .f64_histogram("samod_sync_queue_millis")
                .with_description("Time sync messages spent waiting in queue")
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
            hub_connections: meter
                .f64_gauge("samod_hub_connections")
                .with_description("Number of active hub connections")
                .build(),
            hub_documents: meter
                .f64_gauge("samod_hub_documents")
                .with_description("Number of active hub documents")
                .build(),
            document_pending_sync_messages: meter
                .f64_gauge("samod_document_pending_sync_messages")
                .with_description("Pending sync messages during Loading phase")
                .build(),
            documents_active_count: AtomicI64::new(0),
            peers_connected_count: AtomicI64::new(0),
        }
    }
}

impl RepoObserver for OtelObserver {
    fn observe(&self, event: &RepoEvent) {
        match event {
            RepoEvent::DocumentOpened { .. } => {
                let count = self.documents_active_count.fetch_add(1, Ordering::Relaxed) + 1;
                self.documents_active.record(count as f64, &[]);
            }
            RepoEvent::DocumentClosed { .. } => {
                let count = self.documents_active_count.fetch_sub(1, Ordering::Relaxed) - 1;
                self.documents_active.record(count as f64, &[]);
            }
            RepoEvent::ConnectionEstablished { connection_id: _ } => {
                self.connections_total
                    .add(1, &[KeyValue::new("event", "connected")]);
                let count = self.peers_connected_count.fetch_add(1, Ordering::Relaxed) + 1;
                self.peers_connected.record(count as f64, &[]);
            }
            RepoEvent::ConnectionLost { connection_id: _ } => {
                self.connections_total
                    .add(1, &[KeyValue::new("event", "disconnected")]);
                let count = self.peers_connected_count.fetch_sub(1, Ordering::Relaxed) - 1;
                self.peers_connected.record(count as f64, &[]);
            }
            RepoEvent::SyncMessageReceived {
                bytes,
                duration,
                queue_duration,
                ..
            } => {
                let attrs = [KeyValue::new("direction", "received")];
                self.sync_processing_millis
                    .record(duration.as_secs_f64() * 1000.0, &attrs);
                self.sync_queue_millis
                    .record(queue_duration.as_secs_f64() * 1000.0, &attrs);
                self.sync_message_bytes.record(*bytes as f64, &attrs);
            }
            RepoEvent::SyncMessageGenerated {
                bytes, duration, ..
            } => {
                let attrs = [KeyValue::new("direction", "generated")];
                self.sync_processing_millis
                    .record(duration.as_millis() as f64, &attrs);
                self.sync_message_bytes.record(*bytes as f64, &attrs);
            }
            RepoEvent::StorageOperationCompleted {
                operation,
                duration,
                ..
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
            RepoEvent::HubEventProcessed {
                duration,
                event_type,
                connections,
                documents,
            } => {
                let attrs = [KeyValue::new("event_type", *event_type)];
                self.hub_event_processing_millis
                    .record(duration.as_millis() as f64, &attrs);
                self.hub_connections.record(*connections as f64, &[]);
                self.hub_documents.record(*documents as f64, &[]);
            }
            RepoEvent::DocumentPendingSyncMessages { count, .. } => {
                self.document_pending_sync_messages
                    .record(*count as f64, &[]);
            }
            _ => {}
        }
    }
}
