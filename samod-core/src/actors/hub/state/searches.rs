use std::collections::HashMap;

use crate::{
    DialerId, DocSearchPhase, DocumentActorId,
    actors::hub::{
        dialer::{DialerState, DialerStatus},
        state::ActorInfo,
    },
};

pub(super) struct Searches {
    pub(super) phases: HashMap<DocumentActorId, DocSearchPhase>,
    pub(super) last_dialers: HashMap<DialerId, DialerStatus>,
}

impl Searches {
    pub(super) fn new() -> Self {
        Self {
            phases: HashMap::new(),
            last_dialers: HashMap::new(),
        }
    }

    pub(super) fn add_search(&mut self, actor: DocumentActorId, phase: DocSearchPhase) {
        self.phases.insert(actor, phase);
    }

    pub(super) fn pop_state_updates(
        &mut self,
        actors: &HashMap<DocumentActorId, ActorInfo>,
        dialers: &HashMap<DialerId, DialerState>,
    ) -> HashMap<DocumentActorId, DocSearchPhase> {
        let new_dialer_state = dialers
            .iter()
            .map(|(dialer_id, state)| (*dialer_id, state.status.clone()))
            .collect::<HashMap<DialerId, DialerStatus>>();
        let dialers_changed = self.last_dialers != new_dialer_state;
        self.last_dialers = new_dialer_state;

        let mut result = HashMap::new();
        for (actor_id, actor) in actors {
            let last_phase = self.phases.get(actor_id);
            if last_phase != Some(&actor.search_phase) || dialers_changed {
                result.insert(*actor_id, actor.search_phase.clone());
            }
            self.phases.insert(*actor_id, actor.search_phase.clone());
        }
        result
    }
}
