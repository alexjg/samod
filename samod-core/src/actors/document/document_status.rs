#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum DocumentStatus {
    Spawned,
    Loading,
    Requesting,
    Ready,
    NotFound,
}
