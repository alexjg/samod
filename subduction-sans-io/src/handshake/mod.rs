//! Handshake protocol: mutual authentication via signed challenge/response.
//!
//! # Wire Format
//!
//! All handshake messages share a common envelope:
//!
//! ```text
//! ┌────────────┬─────┬───────────────────┐
//! │ Schema (4B)│ Tag │    Payload         │
//! │  SUH\0     │ 1B  │   (variable)       │
//! └────────────┴─────┴───────────────────┘
//! ```
//!
//! Tags:
//! - `0x00` = `SignedChallenge` (payload is `Signed<Challenge>` wire bytes)
//! - `0x01` = `SignedResponse` (payload is `Signed<Response>` wire bytes)
//! - `0x02` = `Rejection` (payload is reason(1B) + timestamp(8B))

pub mod challenge;
pub mod rejection;
pub mod response;

pub use challenge::{Challenge, ChallengeValidationError};
pub use rejection::{Rejection, RejectionDecodeError, RejectionReason};
pub use response::{Response, ResponseValidationError};

use std::time::Duration;

use sedimentree_core::codec::{
    encode::EncodeFields,
    error::{DecodeError, InvalidEnumTag, InvalidSchema},
    schema::{self, Schema},
};
use subduction_crypto::{nonce::Nonce, signed::Signed};
use thiserror::Error;

use crate::types::{Audience, PeerId, TimestampSeconds};

// ---------------------------------------------------------------------------
// HandshakeMessage envelope
// ---------------------------------------------------------------------------

const TAG_CHALLENGE: u8 = 0x00;
const TAG_RESPONSE: u8 = 0x01;
const TAG_REJECTION: u8 = 0x02;

/// A handshake message on the wire.
#[derive(Clone, Debug)]
pub enum HandshakeMessage {
    SignedChallenge(Signed<Challenge>),
    SignedResponse(Signed<Response>),
    Rejection(Rejection),
}

impl Schema for HandshakeMessage {
    const PREFIX: [u8; 2] = schema::SUBDUCTION_PREFIX;
    const TYPE_BYTE: u8 = b'H';
    const VERSION: u8 = 0;
}

impl HandshakeMessage {
    #[must_use]
    pub fn encode(&self) -> Vec<u8> {
        let (tag, payload) = match self {
            Self::SignedChallenge(signed) => (TAG_CHALLENGE, signed.as_bytes().to_vec()),
            Self::SignedResponse(signed) => (TAG_RESPONSE, signed.as_bytes().to_vec()),
            Self::Rejection(rejection) => (TAG_REJECTION, rejection.encode_payload()),
        };
        let mut buf = Vec::with_capacity(4 + 1 + payload.len());
        buf.extend_from_slice(&Self::SCHEMA);
        buf.push(tag);
        buf.extend_from_slice(&payload);
        buf
    }

    pub fn try_decode(buf: Vec<u8>) -> Result<Self, DecodeError> {
        if buf.len() < 5 {
            return Err(DecodeError::MessageTooShort {
                type_name: "HandshakeMessage",
                need: 5,
                have: buf.len(),
            });
        }

        let schema: [u8; 4] = buf[..4].try_into().expect("checked length");
        if schema != Self::SCHEMA {
            return Err(InvalidSchema {
                expected: Self::SCHEMA,
                got: schema,
            }
            .into());
        }

        let tag = buf[4];
        let payload = buf[5..].to_vec();

        match tag {
            TAG_CHALLENGE => {
                let signed = Signed::<Challenge>::try_decode(payload)?;
                Ok(Self::SignedChallenge(signed))
            }
            TAG_RESPONSE => {
                let signed = Signed::<Response>::try_decode(payload)?;
                Ok(Self::SignedResponse(signed))
            }
            TAG_REJECTION => {
                let rejection = Rejection::try_decode_payload(&payload)?;
                Ok(Self::Rejection(rejection))
            }
            _ => Err(InvalidEnumTag {
                tag,
                type_name: "HandshakeMessage",
            }
            .into()),
        }
    }
}

// ---------------------------------------------------------------------------
// Sans-IO handshake state machine
// ---------------------------------------------------------------------------

/// Maximum clock drift tolerated during simultaneous open handshakes.
const SIMULTANEOUS_OPEN_MAX_DRIFT: Duration = Duration::from_secs(600);

/// Inputs to the handshake state machine.
#[derive(Clone, Debug)]
pub enum HandshakeInput {
    /// Start a handshake as the initiator.
    Initiate {
        audience: Audience,
        timestamp: TimestampSeconds,
        nonce: Nonce,
    },

    /// Bytes received from the remote peer.
    ReceivedBytes {
        bytes: Vec<u8>,
        /// The current time, used for challenge validation and response timestamps.
        now: TimestampSeconds,
    },

    /// A previously-requested signing operation has completed.
    SigningComplete {
        signature: ed25519_dalek::Signature,
    },
}

/// Outputs from the handshake state machine.
#[derive(Clone, Debug)]
pub enum HandshakeOutput {
    /// Send these bytes to the remote peer.
    SendBytes(Vec<u8>),

    /// Request that the host sign the given payload bytes.
    ///
    /// The state machine only ever has one outstanding signing request at a
    /// time; the caller is responsible for correlating signing tasks if it
    /// manages multiple handshakes.
    SigningRequest {
        payload_bytes: Vec<u8>,
    },

    /// The handshake completed successfully.
    Complete {
        their_peer_id: PeerId,
    },

    /// The handshake failed.
    Failed(HandshakeError),
}

/// Errors that can occur during the handshake.
#[derive(Clone, Debug, Error)]
pub enum HandshakeError {
    #[error("decode error: {0}")]
    Decode(#[from] DecodeError),

    #[error("signature verification failed")]
    InvalidSignature,

    #[error("response validation failed: {0}")]
    ResponseValidation(#[from] ResponseValidationError),

    #[error("challenge validation failed: {0}")]
    ChallengeValidation(#[from] ChallengeValidationError),

    #[error("unexpected message: expected {expected}, got {got}")]
    UnexpectedMessage {
        expected: &'static str,
        got: &'static str,
    },

    #[error("invalid state for input")]
    InvalidState,

    #[error("reflected challenge detected (possible reflection attack)")]
    ReflectedChallenge,

    #[error("challenge signed by our own key (reflection attack)")]
    ReflectionAttack,

    #[error("peer ID mismatch in simultaneous open")]
    SimultaneousOpenPeerMismatch,

    /// The remote peer rejected our handshake.
    #[error("rejected by peer: {reason:?}")]
    Rejected {
        reason: RejectionReason,
        server_timestamp: TimestampSeconds,
    },
}

/// Configuration for the responder side of the handshake.
#[derive(Clone, Debug)]
pub struct ResponderConfig {
    pub expected_audience: Audience,
    pub max_clock_drift: Duration,
}

/// Configuration for the handshake state machine.
#[derive(Clone, Debug)]
pub struct HandshakeConfig {
    pub our_verifying_key: ed25519_dalek::VerifyingKey,
    pub responder_config: Option<ResponderConfig>,
}

/// What kind of thing we're waiting for a signature on.
#[derive(Clone, Debug)]
enum PendingSign {
    Challenge(Challenge),
    Response {
        their_peer_id: PeerId,
        response: Response,
    },
    SimultaneousResponse {
        our_challenge: Challenge,
        their_peer_id: PeerId,
        response: Response,
    },
}

#[derive(Clone, Debug)]
enum State {
    Idle,
    AwaitingSignature(PendingSign),
    AwaitingResponse {
        our_challenge: Challenge,
        our_signed_challenge_bytes: Vec<u8>,
    },
    SimultaneousAwaitingResponse {
        our_challenge: Challenge,
        expected_peer_id: PeerId,
    },
    Complete { their_peer_id: PeerId },
    Failed,
}

/// A sans-IO handshake state machine.
///
/// Handles initiator, responder, and simultaneous-open roles.
#[derive(Clone, Debug)]
pub struct HandshakeMachine {
    config: HandshakeConfig,
    state: State,
}

impl HandshakeMachine {
    #[must_use]
    pub fn new(config: HandshakeConfig) -> Self {
        Self {
            config,
            state: State::Idle,
        }
    }

    pub fn step(&mut self, input: HandshakeInput) -> Vec<HandshakeOutput> {
        let mut outputs = Vec::new();
        match self.step_inner(input, &mut outputs) {
            Ok(()) => {}
            Err(err) => {
                self.state = State::Failed;
                outputs.push(HandshakeOutput::Failed(err));
            }
        }
        outputs
    }

    #[must_use]
    pub fn is_finished(&self) -> bool {
        matches!(self.state, State::Complete { .. } | State::Failed)
    }

    #[must_use]
    pub fn their_peer_id(&self) -> Option<PeerId> {
        match &self.state {
            State::Complete { their_peer_id } => Some(*their_peer_id),
            _ => None,
        }
    }

    fn step_inner(
        &mut self,
        input: HandshakeInput,
        outputs: &mut Vec<HandshakeOutput>,
    ) -> Result<(), HandshakeError> {
        let state = std::mem::replace(&mut self.state, State::Failed);

        match (state, input) {
            // ----- Initiator: start -----
            (State::Idle, HandshakeInput::Initiate { audience, timestamp, nonce }) => {
                let challenge = Challenge::new(audience, timestamp, nonce);
                self.request_signature(PendingSign::Challenge(challenge), outputs);
                Ok(())
            }

            // ----- Signature ready -----
            (State::AwaitingSignature(pending), HandshakeInput::SigningComplete { signature }) => {
                self.complete_signature(pending, signature, outputs)
            }

            // ----- Initiator: received message while awaiting response -----
            (
                State::AwaitingResponse {
                    our_challenge,
                    our_signed_challenge_bytes,
                },
                HandshakeInput::ReceivedBytes { bytes, now },
            ) => {
                let msg = HandshakeMessage::try_decode(bytes)?;
                match msg {
                    HandshakeMessage::SignedResponse(signed) => {
                        self.verify_response(signed, &our_challenge, outputs)?;
                        Ok(())
                    }
                    HandshakeMessage::SignedChallenge(their_signed) => {
                        self.handle_simultaneous_open(
                            our_challenge,
                            &our_signed_challenge_bytes,
                            their_signed,
                            now,
                            outputs,
                        )
                    }
                    HandshakeMessage::Rejection(rejection) => {
                        Err(HandshakeError::Rejected {
                            reason: rejection.reason,
                            server_timestamp: rejection.server_timestamp,
                        })
                    }
                }
            }

            // ----- Responder: received challenge while idle -----
            (State::Idle, HandshakeInput::ReceivedBytes { bytes, now }) => {
                let msg = HandshakeMessage::try_decode(bytes)?;
                match msg {
                    HandshakeMessage::SignedChallenge(signed) => {
                        self.handle_received_challenge(signed, now, outputs)
                    }
                    HandshakeMessage::SignedResponse(_) => Err(HandshakeError::UnexpectedMessage {
                        expected: "SignedChallenge",
                        got: "SignedResponse",
                    }),
                    HandshakeMessage::Rejection(_) => Err(HandshakeError::UnexpectedMessage {
                        expected: "SignedChallenge",
                        got: "Rejection",
                    }),
                }
            }

            // ----- Simultaneous open: received response to our challenge -----
            (
                State::SimultaneousAwaitingResponse {
                    our_challenge,
                    expected_peer_id,
                },
                HandshakeInput::ReceivedBytes { bytes, .. },
            ) => {
                let msg = HandshakeMessage::try_decode(bytes)?;
                match msg {
                    HandshakeMessage::SignedResponse(signed) => {
                        let their_peer_id =
                            self.verify_response(signed, &our_challenge, outputs)?;
                        if their_peer_id != expected_peer_id {
                            self.state = State::Failed;
                            return Err(HandshakeError::SimultaneousOpenPeerMismatch);
                        }
                        Ok(())
                    }
                    _ => Err(HandshakeError::UnexpectedMessage {
                        expected: "SignedResponse",
                        got: "other",
                    }),
                }
            }

            // ----- Terminal / invalid -----
            (State::Complete { .. } | State::Failed, _) => Err(HandshakeError::InvalidState),
            (_, _) => Err(HandshakeError::InvalidState),
        }
    }

    fn request_signature(
        &mut self,
        pending: PendingSign,
        outputs: &mut Vec<HandshakeOutput>,
    ) {
        let payload_bytes = match &pending {
            PendingSign::Challenge(challenge) => signable_bytes(&self.config.our_verifying_key, challenge),
            PendingSign::Response { response, .. }
            | PendingSign::SimultaneousResponse { response, .. } => {
                signable_bytes(&self.config.our_verifying_key, response)
            }
        };
        outputs.push(HandshakeOutput::SigningRequest { payload_bytes });
        self.state = State::AwaitingSignature(pending);
    }

    fn complete_signature(
        &mut self,
        pending: PendingSign,
        signature: ed25519_dalek::Signature,
        outputs: &mut Vec<HandshakeOutput>,
    ) -> Result<(), HandshakeError> {
        let vk = self.config.our_verifying_key;

        match pending {
            PendingSign::Challenge(challenge) => {
                let signed = Signed::from_parts(vk, signature, &challenge);
                let signed_bytes = signed.as_bytes().to_vec();
                let msg = HandshakeMessage::SignedChallenge(signed);
                outputs.push(HandshakeOutput::SendBytes(msg.encode()));

                self.state = State::AwaitingResponse {
                    our_challenge: challenge,
                    our_signed_challenge_bytes: signed_bytes,
                };
                Ok(())
            }
            PendingSign::Response { their_peer_id, response } => {
                let signed = Signed::from_parts(vk, signature, &response);
                let msg = HandshakeMessage::SignedResponse(signed);
                outputs.push(HandshakeOutput::SendBytes(msg.encode()));

                self.state = State::Complete { their_peer_id };
                outputs.push(HandshakeOutput::Complete { their_peer_id });
                Ok(())
            }
            PendingSign::SimultaneousResponse {
                our_challenge,
                their_peer_id,
                response,
            } => {
                let signed = Signed::from_parts(vk, signature, &response);
                let msg = HandshakeMessage::SignedResponse(signed);
                outputs.push(HandshakeOutput::SendBytes(msg.encode()));

                self.state = State::SimultaneousAwaitingResponse {
                    our_challenge,
                    expected_peer_id: their_peer_id,
                };
                Ok(())
            }
        }
    }

    fn verify_response(
        &mut self,
        signed: Signed<Response>,
        our_challenge: &Challenge,
        outputs: &mut Vec<HandshakeOutput>,
    ) -> Result<PeerId, HandshakeError> {
        let verified = signed
            .try_verify()
            .map_err(|_| HandshakeError::InvalidSignature)?;
        verified.payload().validate(our_challenge)?;
        let their_peer_id = PeerId::from_verifying_key(&verified.issuer());

        self.state = State::Complete { their_peer_id };
        outputs.push(HandshakeOutput::Complete { their_peer_id });
        Ok(their_peer_id)
    }

    /// Verify an incoming challenge and prepare a response (responder role).
    ///
    /// On validation failure, emits a `Rejection` message before returning the error.
    fn handle_received_challenge(
        &mut self,
        signed: Signed<Challenge>,
        now: TimestampSeconds,
        outputs: &mut Vec<HandshakeOutput>,
    ) -> Result<(), HandshakeError> {
        let verified = match signed.try_verify() {
            Ok(v) => v,
            Err(_) => {
                self.send_rejection(RejectionReason::InvalidSignature, now, outputs);
                return Err(HandshakeError::InvalidSignature);
            }
        };

        let their_peer_id = PeerId::from_verifying_key(&verified.issuer());
        let challenge = *verified.payload();

        if let Some(ref rc) = self.config.responder_config {
            if let Err(e) = challenge.validate(&rc.expected_audience, now, rc.max_clock_drift) {
                self.send_rejection(e.to_rejection_reason(), now, outputs);
                return Err(e.into());
            }
        }

        let response = Response::for_challenge(&challenge, now);
        self.request_signature(
            PendingSign::Response { their_peer_id, response },
            outputs,
        );
        Ok(())
    }

    fn handle_simultaneous_open(
        &mut self,
        our_challenge: Challenge,
        our_signed_challenge_bytes: &[u8],
        their_signed: Signed<Challenge>,
        now: TimestampSeconds,
        outputs: &mut Vec<HandshakeOutput>,
    ) -> Result<(), HandshakeError> {
        if our_signed_challenge_bytes == their_signed.as_bytes() {
            return Err(HandshakeError::ReflectedChallenge);
        }

        let verified = their_signed
            .try_verify()
            .map_err(|_| HandshakeError::InvalidSignature)?;

        let their_peer_id = PeerId::from_verifying_key(&verified.issuer());
        let our_peer_id = PeerId::from_verifying_key(&self.config.our_verifying_key);

        if their_peer_id == our_peer_id {
            return Err(HandshakeError::ReflectionAttack);
        }

        let their_challenge = *verified.payload();

        if let Some(ref rc) = self.config.responder_config {
            their_challenge.validate(
                &rc.expected_audience,
                now,
                SIMULTANEOUS_OPEN_MAX_DRIFT,
            )?;
        }

        let response = Response::for_challenge(&their_challenge, now);
        self.request_signature(
            PendingSign::SimultaneousResponse {
                our_challenge,
                their_peer_id,
                response,
            },
            outputs,
        );
        Ok(())
    }

    /// Emit a rejection message to send to the peer.
    fn send_rejection(
        &self,
        reason: RejectionReason,
        now: TimestampSeconds,
        outputs: &mut Vec<HandshakeOutput>,
    ) {
        let rejection = Rejection::new(reason, now);
        let msg = HandshakeMessage::Rejection(rejection);
        outputs.push(HandshakeOutput::SendBytes(msg.encode()));
    }
}

/// Build the bytes-to-sign for a payload: schema(4) + verifying_key(32) + fields(N).
fn signable_bytes<T: Schema + EncodeFields>(
    vk: &ed25519_dalek::VerifyingKey,
    payload: &T,
) -> Vec<u8> {
    let mut buf = Vec::with_capacity(4 + 32 + payload.fields_size());
    buf.extend_from_slice(&T::SCHEMA);
    buf.extend_from_slice(vk.as_bytes());
    payload.encode_fields(&mut buf);
    buf
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::DiscoveryId;
    use ed25519_dalek::{SigningKey, Signer as _};

    fn make_keypair(seed: u8) -> (SigningKey, ed25519_dalek::VerifyingKey) {
        let signing = SigningKey::from_bytes(&[seed; 32]);
        let verifying = signing.verifying_key();
        (signing, verifying)
    }

    fn make_nonce(val: u8) -> Nonce {
        Nonce::from_bytes([val; 16])
    }

    const NOW: TimestampSeconds = TimestampSeconds::new(1000);

    fn sign_output(
        signing_key: &SigningKey,
        output: &HandshakeOutput,
    ) -> Option<HandshakeInput> {
        match output {
            HandshakeOutput::SigningRequest { payload_bytes } => {
                let signature = signing_key.sign(payload_bytes);
                Some(HandshakeInput::SigningComplete { signature })
            }
            _ => None,
        }
    }

    fn received(bytes: Vec<u8>) -> HandshakeInput {
        HandshakeInput::ReceivedBytes { bytes, now: NOW }
    }

    #[test]
    fn rejection_roundtrip() {
        let rejection = Rejection::new(
            RejectionReason::ClockDrift,
            TimestampSeconds::new(1234567890),
        );
        let msg = HandshakeMessage::Rejection(rejection);
        let encoded = msg.encode();
        let decoded = HandshakeMessage::try_decode(encoded).unwrap();
        match decoded {
            HandshakeMessage::Rejection(r) => {
                assert_eq!(r.reason, RejectionReason::ClockDrift);
                assert_eq!(r.server_timestamp, TimestampSeconds::new(1234567890));
            }
            other => panic!("expected Rejection, got {:?}", other),
        }
    }

    #[test]
    fn signed_challenge_roundtrip() {
        let (signing_key, verifying_key) = make_keypair(1);
        let challenge = Challenge::new(
            Audience::Discover(DiscoveryId::from_raw([0xAA; 32])),
            NOW,
            make_nonce(42),
        );

        let payload_bytes = signable_bytes(&verifying_key, &challenge);
        let sig = signing_key.sign(&payload_bytes);
        let signed = Signed::from_parts(verifying_key, sig, &challenge);

        let msg = HandshakeMessage::SignedChallenge(signed);
        let encoded = msg.encode();
        let decoded = HandshakeMessage::try_decode(encoded).unwrap();

        match decoded {
            HandshakeMessage::SignedChallenge(s) => {
                let verified = s.try_verify().unwrap();
                assert_eq!(*verified.payload(), challenge);
                assert_eq!(verified.issuer(), verifying_key);
            }
            other => panic!("expected SignedChallenge, got {:?}", other),
        }
    }

    #[test]
    fn initiator_responder_happy_path() {
        let (init_signing, init_verifying) = make_keypair(1);
        let (resp_signing, resp_verifying) = make_keypair(2);
        let audience = Audience::Known(PeerId::from_verifying_key(&resp_verifying));

        let mut initiator = HandshakeMachine::new(HandshakeConfig {
            our_verifying_key: init_verifying,
            responder_config: None,
        });
        let mut responder = HandshakeMachine::new(HandshakeConfig {
            our_verifying_key: resp_verifying,
            responder_config: None,
        });

        // Initiator starts
        let o = initiator.step(HandshakeInput::Initiate {
            audience,
            timestamp: NOW,
            nonce: make_nonce(42),
        });
        assert_eq!(o.len(), 1);
        let o = initiator.step(sign_output(&init_signing, &o[0]).unwrap());
        assert_eq!(o.len(), 1);
        let challenge_bytes = match &o[0] {
            HandshakeOutput::SendBytes(b) => b.clone(),
            other => panic!("expected SendBytes, got {:?}", other),
        };

        // Responder receives challenge
        let o = responder.step(received(challenge_bytes));
        assert_eq!(o.len(), 1);
        let o = responder.step(sign_output(&resp_signing, &o[0]).unwrap());
        assert_eq!(o.len(), 2); // SendBytes + Complete
        let response_bytes = match &o[0] {
            HandshakeOutput::SendBytes(b) => b.clone(),
            other => panic!("expected SendBytes, got {:?}", other),
        };
        assert!(matches!(&o[1], HandshakeOutput::Complete { their_peer_id }
            if *their_peer_id == PeerId::from_verifying_key(&init_verifying)));

        // Initiator receives response
        let o = initiator.step(received(response_bytes));
        assert_eq!(o.len(), 1);
        assert!(matches!(&o[0], HandshakeOutput::Complete { their_peer_id }
            if *their_peer_id == PeerId::from_verifying_key(&resp_verifying)));

        assert!(initiator.is_finished());
        assert!(responder.is_finished());
    }

    #[test]
    fn simultaneous_open() {
        let (a_signing, a_verifying) = make_keypair(1);
        let (b_signing, b_verifying) = make_keypair(2);

        let mut machine_a = HandshakeMachine::new(HandshakeConfig {
            our_verifying_key: a_verifying,
            responder_config: None,
        });
        let mut machine_b = HandshakeMachine::new(HandshakeConfig {
            our_verifying_key: b_verifying,
            responder_config: None,
        });

        // Both initiate
        let a_o = machine_a.step(HandshakeInput::Initiate {
            audience: Audience::Known(PeerId::from_verifying_key(&b_verifying)),
            timestamp: NOW,
            nonce: make_nonce(1),
        });
        let b_o = machine_b.step(HandshakeInput::Initiate {
            audience: Audience::Known(PeerId::from_verifying_key(&a_verifying)),
            timestamp: NOW,
            nonce: make_nonce(2),
        });

        // Both sign and send challenges
        let a_o = machine_a.step(sign_output(&a_signing, &a_o[0]).unwrap());
        let a_challenge = match &a_o[0] { HandshakeOutput::SendBytes(b) => b.clone(), _ => panic!() };
        let b_o = machine_b.step(sign_output(&b_signing, &b_o[0]).unwrap());
        let b_challenge = match &b_o[0] { HandshakeOutput::SendBytes(b) => b.clone(), _ => panic!() };

        // Both receive the other's challenge (simultaneous open)
        let a_o = machine_a.step(received(b_challenge));
        assert_eq!(a_o.len(), 1);
        let b_o = machine_b.step(received(a_challenge));
        assert_eq!(b_o.len(), 1);

        // Both sign their responses
        let a_o = machine_a.step(sign_output(&a_signing, &a_o[0]).unwrap());
        let a_response = match &a_o[0] { HandshakeOutput::SendBytes(b) => b.clone(), _ => panic!() };
        let b_o = machine_b.step(sign_output(&b_signing, &b_o[0]).unwrap());
        let b_response = match &b_o[0] { HandshakeOutput::SendBytes(b) => b.clone(), _ => panic!() };

        // Both receive the other's response → complete
        let a_o = machine_a.step(received(b_response));
        assert!(matches!(&a_o[0], HandshakeOutput::Complete { their_peer_id }
            if *their_peer_id == PeerId::from_verifying_key(&b_verifying)));
        let b_o = machine_b.step(received(a_response));
        assert!(matches!(&b_o[0], HandshakeOutput::Complete { their_peer_id }
            if *their_peer_id == PeerId::from_verifying_key(&a_verifying)));
    }

    #[test]
    fn invalid_response_signature_fails() {
        let (init_signing, init_verifying) = make_keypair(1);
        let (_resp_signing, resp_verifying) = make_keypair(2);
        let (wrong_signing, _) = make_keypair(3);

        let mut initiator = HandshakeMachine::new(HandshakeConfig {
            our_verifying_key: init_verifying,
            responder_config: None,
        });
        let mut responder = HandshakeMachine::new(HandshakeConfig {
            our_verifying_key: resp_verifying,
            responder_config: None,
        });

        let o = initiator.step(HandshakeInput::Initiate {
            audience: Audience::Known(PeerId::from_verifying_key(&resp_verifying)),
            timestamp: NOW,
            nonce: make_nonce(42),
        });
        let o = initiator.step(sign_output(&init_signing, &o[0]).unwrap());
        let challenge = match &o[0] { HandshakeOutput::SendBytes(b) => b.clone(), _ => panic!() };

        let o = responder.step(received(challenge));
        // Sign with WRONG key
        let o = responder.step(sign_output(&wrong_signing, &o[0]).unwrap());
        let response = match &o[0] { HandshakeOutput::SendBytes(b) => b.clone(), _ => panic!() };

        let o = initiator.step(received(response));
        assert!(matches!(&o[0], HandshakeOutput::Failed(HandshakeError::InvalidSignature)));
    }

    #[test]
    fn responder_sends_rejection_on_invalid_signature() {
        let (init_signing, _init_verifying) = make_keypair(1);
        let (_resp_signing, resp_verifying) = make_keypair(2);

        let mut responder = HandshakeMachine::new(HandshakeConfig {
            our_verifying_key: resp_verifying,
            responder_config: None,
        });

        // Build a challenge signed by init_signing but with wrong issuer bytes
        // (corrupt the signed bytes so signature verification fails)
        let challenge = Challenge::new(
            Audience::Known(PeerId::from_verifying_key(&resp_verifying)),
            NOW,
            make_nonce(1),
        );
        let payload = signable_bytes(&resp_verifying, &challenge);
        // Sign with a different key than what the issuer field says
        let sig = init_signing.sign(&payload);
        let signed = Signed::from_parts(resp_verifying, sig, &challenge);
        let msg = HandshakeMessage::SignedChallenge(signed);
        let bytes = msg.encode();

        let o = responder.step(received(bytes));
        // Should get: SendBytes(rejection) + Failed
        assert_eq!(o.len(), 2);
        match &o[0] {
            HandshakeOutput::SendBytes(bytes) => {
                let decoded = HandshakeMessage::try_decode(bytes.clone()).unwrap();
                match decoded {
                    HandshakeMessage::Rejection(r) => {
                        assert_eq!(r.reason, RejectionReason::InvalidSignature);
                    }
                    other => panic!("expected Rejection, got {:?}", other),
                }
            }
            other => panic!("expected SendBytes(rejection), got {:?}", other),
        }
        assert!(matches!(&o[1], HandshakeOutput::Failed(HandshakeError::InvalidSignature)));
    }

    #[test]
    fn responder_sends_rejection_on_clock_drift() {
        let (init_signing, init_verifying) = make_keypair(1);
        let (_resp_signing, resp_verifying) = make_keypair(2);

        let mut responder = HandshakeMachine::new(HandshakeConfig {
            our_verifying_key: resp_verifying,
            responder_config: Some(ResponderConfig {
                expected_audience: Audience::Known(PeerId::from_verifying_key(&resp_verifying)),
                max_clock_drift: Duration::from_secs(10),
            }),
        });

        // Create a valid challenge but with a timestamp far in the past
        let challenge = Challenge::new(
            Audience::Known(PeerId::from_verifying_key(&resp_verifying)),
            TimestampSeconds::new(500), // way before NOW=1000
            make_nonce(1),
        );
        let payload = signable_bytes(&init_verifying, &challenge);
        let sig = init_signing.sign(&payload);
        let signed = Signed::from_parts(init_verifying, sig, &challenge);
        let msg = HandshakeMessage::SignedChallenge(signed);
        let bytes = msg.encode();

        let o = responder.step(received(bytes));
        assert_eq!(o.len(), 2);
        match &o[0] {
            HandshakeOutput::SendBytes(bytes) => {
                let decoded = HandshakeMessage::try_decode(bytes.clone()).unwrap();
                match decoded {
                    HandshakeMessage::Rejection(r) => {
                        assert_eq!(r.reason, RejectionReason::ClockDrift);
                    }
                    other => panic!("expected Rejection, got {:?}", other),
                }
            }
            other => panic!("expected SendBytes(rejection), got {:?}", other),
        }
        assert!(matches!(&o[1], HandshakeOutput::Failed(HandshakeError::ChallengeValidation(..))));
    }

    #[test]
    fn initiator_handles_rejection_from_responder() {
        let (init_signing, init_verifying) = make_keypair(1);

        let mut initiator = HandshakeMachine::new(HandshakeConfig {
            our_verifying_key: init_verifying,
            responder_config: None,
        });

        let o = initiator.step(HandshakeInput::Initiate {
            audience: Audience::Known(PeerId::new([2; 32])),
            timestamp: NOW,
            nonce: make_nonce(1),
        });
        let o = initiator.step(sign_output(&init_signing, &o[0]).unwrap());
        assert!(matches!(&o[0], HandshakeOutput::SendBytes(_)));

        // Simulate receiving a rejection
        let rejection = Rejection::new(RejectionReason::ClockDrift, TimestampSeconds::new(999));
        let rejection_bytes = HandshakeMessage::Rejection(rejection).encode();

        let o = initiator.step(received(rejection_bytes));
        assert_eq!(o.len(), 1);
        match &o[0] {
            HandshakeOutput::Failed(HandshakeError::Rejected { reason, .. }) => {
                assert_eq!(*reason, RejectionReason::ClockDrift);
            }
            other => panic!("expected Failed(Rejected), got {:?}", other),
        }
    }

    #[test]
    fn input_after_completion_fails() {
        let (init_signing, init_verifying) = make_keypair(1);
        let (resp_signing, resp_verifying) = make_keypair(2);

        let mut initiator = HandshakeMachine::new(HandshakeConfig {
            our_verifying_key: init_verifying,
            responder_config: None,
        });
        let mut responder = HandshakeMachine::new(HandshakeConfig {
            our_verifying_key: resp_verifying,
            responder_config: None,
        });

        let o = initiator.step(HandshakeInput::Initiate {
            audience: Audience::Known(PeerId::from_verifying_key(&resp_verifying)),
            timestamp: NOW,
            nonce: make_nonce(42),
        });
        let o = initiator.step(sign_output(&init_signing, &o[0]).unwrap());
        let challenge = match &o[0] { HandshakeOutput::SendBytes(b) => b.clone(), _ => panic!() };
        let o = responder.step(received(challenge));
        let o = responder.step(sign_output(&resp_signing, &o[0]).unwrap());
        let response = match &o[0] { HandshakeOutput::SendBytes(b) => b.clone(), _ => panic!() };
        initiator.step(received(response));
        assert!(initiator.is_finished());

        let o = initiator.step(received(vec![0; 10]));
        assert!(matches!(&o[0], HandshakeOutput::Failed(HandshakeError::InvalidState)));
    }
}
