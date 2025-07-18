use std::sync::{Arc, Mutex};

use automerge::Automerge;
use futures::Stream;
use samod_core::{AutomergeUrl, DocumentChanged, DocumentId};

use crate::doc_actor_inner::DocActorInner;

// A `DocHandle` wraps an automerge document and does two things:
//
// * Captures modifications made using `DocHandle::with_document` and publishes
//   those changes to any connected peers
// * Provides a way to listen for changes made by other peers (or local processes)
//
// To obtain a `DocHandle` you call either [`Samod::create`]
#[derive(Clone)]
pub struct DocHandle {
    inner: Arc<Mutex<DocActorInner>>,
    document_id: DocumentId,
}

impl DocHandle {
    pub(crate) fn new(doc_id: DocumentId, inner: Arc<Mutex<DocActorInner>>) -> Self {
        Self {
            document_id: doc_id,
            inner,
        }
    }

    pub fn document_id(&self) -> &DocumentId {
        &self.document_id
    }

    pub fn url(&self) -> AutomergeUrl {
        AutomergeUrl::from(self.document_id())
    }

    // Note that this method blocks the current thread until the document is
    // available. There are two major reasons that the document might be
    // unavailable:
    //
    // * Another caller is currently calling `with_document` and doing something
    //   which takes a long time
    // * We are receiving a sync message which is taking a long time to process
    //
    // This means it's probably best to run calls to this method inside
    // `spawn_blocking` or similar constructions
    pub fn with_document<F, R>(&self, f: F) -> R
    where
        F: FnOnce(&mut Automerge) -> R,
    {
        self.inner.lock().unwrap().with_document(f)
    }

    // Listen to ephemeral messages sent by other peers to this document
    pub fn ephemera(&self) -> impl Stream<Item = Vec<u8>> {
        self.inner.lock().unwrap().create_ephemera_listener()
    }

    // Listen for changes to the document
    pub fn changes(&self) -> impl Stream<Item = DocumentChanged> {
        self.inner.lock().unwrap().create_change_listener()
    }

    // Send an ephemeral message which will be broadcast to all other peers who have this document open
    //
    // Note that whilst you can send any binary payload, the JS implementation
    // will only process payloads which are valid CBOR
    pub fn broadcast(&self, message: Vec<u8>) {
        self.inner
            .lock()
            .unwrap()
            .broadcast_ephemeral_message(message);
    }
}
