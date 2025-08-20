use std::sync::{Arc, Mutex};

use automerge::Automerge;
use futures::Stream;
use samod_core::{AutomergeUrl, DocumentChanged, DocumentId};

use crate::doc_actor_inner::DocActorInner;

/// The state of a single [`automerge`] document the [`Repo`](crate::Repo) is managing
///
/// [`DocHandle`]s are obtained using [`Repo::create`](crate::Repo::create) or
/// [`Repo::find`](crate::Repo::find)
///
/// Each `DocHandle` wraps an underlying `automerge::Automerge` instance in order to
/// capture local changes made to the document and publish them to any connected peers;
/// and to listen for remote changes made to the document and notify the local process.
///
/// To make local changes to a document you use [`DocHandle::with_document`] whilst
/// remote changes can be listened for using [`DocHandle::changes`].
///
/// You can also broadcast ephemeral messages to other peers using
/// [`DocHandle::broadcast`] and listen for ephemeral messages sent by other
/// peers using [`DocHandle::ephemera`].
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

    /// The ID of this document
    pub fn document_id(&self) -> &DocumentId {
        &self.document_id
    }

    /// The URL of this document in a format compatible with the JS `automerge-repo` library
    pub fn url(&self) -> AutomergeUrl {
        AutomergeUrl::from(self.document_id())
    }

    /// Make a change to the underlying `automerge::Automerge` document
    ///
    /// Note that this method blocks the current thread until the document is
    /// available. There are two major reasons that the document might be
    /// unavailable:
    ///
    /// * Another caller is currently calling `with_document` and doing something
    ///   which takes a long time
    /// * We are receiving a sync message which is taking a long time to process
    ///
    /// This means it's probably best to run calls to this method inside
    /// `spawn_blocking` or similar constructions
    pub fn with_document<F, R>(&self, f: F) -> R
    where
        F: FnOnce(&mut Automerge) -> R,
    {
        self.inner.lock().unwrap().with_document(f)
    }

    /// Listen to ephemeral messages sent by other peers to this document
    pub fn ephemera(&self) -> impl Stream<Item = Vec<u8>> {
        self.inner.lock().unwrap().create_ephemera_listener()
    }

    /// Listen for changes to the document
    pub fn changes(&self) -> impl Stream<Item = DocumentChanged> {
        self.inner.lock().unwrap().create_change_listener()
    }

    /// Send an ephemeral message which will be broadcast to all other peers who have this document open
    ///
    /// Note that whilst you can send any binary payload, the JS implementation
    /// will only process payloads which are valid CBOR
    pub fn broadcast(&self, message: Vec<u8>) {
        self.inner
            .lock()
            .unwrap()
            .broadcast_ephemeral_message(message);
    }
}
