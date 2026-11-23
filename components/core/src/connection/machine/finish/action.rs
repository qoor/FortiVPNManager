use crate::connection::Effect;
use crate::connection::machine::ConnectionManager;

use super::decision::Finish;

impl ConnectionManager {
    /// Carries out what the input's end decided and returns every effect of the input, in order.
    pub(in crate::connection::machine) fn finish(&mut self, finish: Finish) -> Vec<Effect> {
        self.change_timers(finish.timers);
        if let Some(hold) = finish.hold {
            self.activity_held = hold;
            self.effects.push(Effect::HoldActivity(hold));
        }
        if finish.save_ledger {
            self.saved_ledger = self.ledger.clone();
            self.effects.push(Effect::SaveLedger(self.ledger.clone()));
        }
        if finish.approve_quit {
            self.effects.push(Effect::ApproveQuit);
        }
        self.shown = finish.shown;
        let mut effects = finish.announcements;
        effects.append(&mut self.effects);
        effects
    }
}
