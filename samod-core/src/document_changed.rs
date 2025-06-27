use automerge::ChangeHash;

#[derive(Clone, Debug)]
pub struct DocumentChanged {
    pub new_heads: Vec<ChangeHash>,
}
