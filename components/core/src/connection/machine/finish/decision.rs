use std::time::SystemTime;

use fvm_contract::app_helper::AttemptId;

use crate::connection::machine::timers::decision::TimerChange;
use crate::connection::machine::{AttemptReason, ConnectionManager, Phase, Quit};
use crate::connection::view::Shown;
use crate::connection::{Effect, Note, TimerKind};

/// What the attempt announcement and the schedule notes compare before and after an input.
pub(in crate::connection::machine) struct NoteMarks {
    /// The attempt under way.
    attempt: Option<AttemptId>,
    scheduled_attempt: Option<AttemptId>,
    holding: bool,
    suspended: bool,
}

/// What follows from the state an input left.
pub(in crate::connection::machine) struct Finish {
    pub(super) timers: Vec<(TimerKind, TimerChange)>,
    /// The activity hold, when it changes.
    pub(super) hold: Option<bool>,
    pub(super) save_ledger: bool,
    pub(super) approve_quit: bool,
    /// The announcement of an attempt begun by this input, then the schedule notes.
    pub(super) announcements: Vec<Effect>,
    pub(super) shown: Shown,
}

impl ConnectionManager {
    pub(in crate::connection::machine) fn note_marks(&self) -> NoteMarks {
        NoteMarks {
            attempt: match &self.phase {
                Phase::Connecting(attempt) => Some(attempt.id.clone()),
                _ => None,
            },
            scheduled_attempt: match &self.phase {
                Phase::Connecting(attempt) if attempt.reason == AttemptReason::Scheduled => {
                    Some(attempt.id.clone())
                }
                _ => None,
            },
            holding: self.phase.is_holding(),
            suspended: self.reconnect_limits.reconnect_suspended,
        }
    }

    pub(in crate::connection::machine) fn finish_plan(
        &self,
        marks: &NoteMarks,
        now: SystemTime,
    ) -> Finish {
        // A pending stale-code search keeps its polling and its time limit on time.
        let hold = self.quit != Quit::Terminated
            && (self.phase.holds_activity() || self.stale_otp.is_some());
        let mut announcements = match &self.phase {
            Phase::Connecting(attempt) if marks.attempt.as_ref() != Some(&attempt.id) => {
                vec![Effect::AttemptBegun {
                    attempt: attempt.id.clone(),
                    session: attempt.session,
                }]
            }
            _ => Vec::new(),
        };
        announcements.append(&mut self.notes(marks));
        Finish {
            timers: self.timer_changes(now),
            hold: (hold != self.activity_held).then_some(hold),
            save_ledger: self.ledger != self.saved_ledger,
            approve_quit: self.quit == Quit::Terminated,
            announcements,
            shown: self.judge_shown(now),
        }
    }

    fn notes(&self, before: &NoteMarks) -> Vec<Effect> {
        let Some(schedule) = &self.settings.connection.schedule else {
            return Vec::new();
        };
        let after = self.note_marks();
        let mut notes = Vec::new();
        if after.scheduled_attempt.is_some() && after.scheduled_attempt != before.scheduled_attempt
        {
            notes.push(Effect::Note {
                attempt: after.scheduled_attempt,
                note: Note::ScheduledHoursStarted,
            });
        }
        if after.holding && !before.holding {
            notes.push(Effect::Note {
                attempt: None,
                note: Note::ReconnectHeldWhileAway,
            });
        }
        if after.suspended && !before.suspended {
            notes.push(Effect::Note {
                attempt: None,
                note: Note::ReconnectsSuspended {
                    away_minutes: schedule.away_minutes,
                },
            });
        }
        notes
    }
}
