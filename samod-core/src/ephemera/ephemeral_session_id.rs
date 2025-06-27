#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) struct EphemeralSessionId(String);

impl EphemeralSessionId {
    pub(crate) fn new_from_rng<R: rand::Rng>(rng: &mut R) -> Self {
        let bytes: [u8; 16] = rng.random();
        let uuid = uuid::Builder::from_random_bytes(bytes).into_uuid();
        Self(uuid.to_string())
    }
}

impl std::fmt::Display for EphemeralSessionId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl From<String> for EphemeralSessionId {
    fn from(value: String) -> Self {
        Self(value)
    }
}
