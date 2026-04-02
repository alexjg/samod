//! Signing abstraction for subduction.
//!
//! The [`Signer`] trait allows the runtime to delegate Ed25519 signing to
//! different backends (in-memory key, SSH agent, WebCrypto, hardware
//! tokens, etc.). A [`MemorySigner`] implementation is provided for
//! simple cases where the signing key is held in memory.
//!
//! [`LocalSigner`] is the non-`Send` counterpart, mirroring the
//! [`Storage`](crate::storage::Storage) / [`LocalStorage`](crate::storage::LocalStorage) pattern.

use std::future::Future;

/// An async Ed25519 signer.
///
/// Implementations must be `Send + Clone + 'static` so they can be moved
/// into async tasks on multi-threaded runtimes.
pub trait Signer: Send + Clone + 'static {
    /// The public verifying key corresponding to this signer.
    fn verifying_key(&self) -> ed25519_dalek::VerifyingKey;

    /// Sign the given payload bytes and return the signature.
    fn sign(&self, payload: Vec<u8>) -> impl Future<Output = ed25519_dalek::Signature> + Send;
}

/// A version of [`Signer`] that can be used with runtimes that don't require
/// `Send` or `'static` bounds. See the [module level documentation on
/// runtimes](../index.html#runtimes) for more details.
pub trait LocalSigner: Clone + 'static {
    /// The public verifying key corresponding to this signer.
    fn verifying_key(&self) -> ed25519_dalek::VerifyingKey;

    /// Sign the given payload bytes and return the signature.
    fn sign(&self, payload: Vec<u8>) -> impl Future<Output = ed25519_dalek::Signature>;
}

impl<S: Signer> LocalSigner for S {
    fn verifying_key(&self) -> ed25519_dalek::VerifyingKey {
        Signer::verifying_key(self)
    }

    fn sign(&self, payload: Vec<u8>) -> impl Future<Output = ed25519_dalek::Signature> {
        Signer::sign(self, payload)
    }
}

/// An in-memory [`Signer`] that holds the Ed25519 signing key directly.
#[derive(Debug, Clone)]
pub struct MemorySigner {
    signing_key: ed25519_dalek::SigningKey,
}

impl MemorySigner {
    /// Create a new `MemorySigner` from an Ed25519 signing key.
    pub fn new(signing_key: ed25519_dalek::SigningKey) -> Self {
        Self { signing_key }
    }

    /// Access the underlying signing key for synchronous signing.
    pub fn signing_key(&self) -> &ed25519_dalek::SigningKey {
        &self.signing_key
    }
}

impl Signer for MemorySigner {
    fn verifying_key(&self) -> ed25519_dalek::VerifyingKey {
        self.signing_key.verifying_key()
    }

    fn sign(&self, payload: Vec<u8>) -> impl Future<Output = ed25519_dalek::Signature> + Send {
        use ed25519_dalek::Signer as _;
        std::future::ready(self.signing_key.sign(&payload))
    }
}
