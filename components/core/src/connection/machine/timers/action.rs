use crate::connection::machine::{ConnectionManager, Step, TimerSlot};
use crate::connection::{Effect, TimerId, TimerKind};

use super::decision::TimerChange;

impl ConnectionManager {
    /// Marks the due timer fired; what it is due for follows.
    pub(in crate::connection::machine) fn fire_timer(
        &mut self,
        due: Option<TimerKind>,
    ) -> Vec<Step> {
        let Some(kind) = due else {
            return Vec::new();
        };
        self.timers
            .get_mut(&kind)
            .expect("a due timer has its slot")
            .fired = true;
        vec![Step::TimerDue(kind)]
    }

    pub(in crate::connection::machine) fn change_timers(
        &mut self,
        changes: Vec<(TimerKind, TimerChange)>,
    ) {
        for (kind, change) in changes {
            match change {
                TimerChange::Remove { cancel } => {
                    self.timers.remove(&kind);
                    if cancel {
                        self.effects.push(Effect::CancelTimer(kind));
                    }
                }
                TimerChange::Set { at } => {
                    self.next_timer_generation += 1;
                    let timer = TimerId {
                        kind,
                        generation: self.next_timer_generation,
                    };
                    self.timers.insert(
                        kind,
                        TimerSlot {
                            at,
                            generation: timer.generation,
                            fired: false,
                        },
                    );
                    self.effects.push(Effect::SetTimer { timer, at });
                }
            }
        }
        self.proactive_timer_rearm = false;
    }

    /// Asks the runtime for a fresh proactive timer even at an unchanged time: a timer may not
    /// survive the sleep reliably.
    pub(in crate::connection::machine) fn rearm_proactive_timer(&mut self) {
        self.proactive_timer_rearm = true;
    }
}
