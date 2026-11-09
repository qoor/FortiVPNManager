//! The domain sections the UI draws (SPEC F-RT-02 item 2), built from the core's view and the
//! runtime's own state. Only the ones that changed are sent.

use fvm_contract::app_internal::MinutesSetting;
use fvm_contract::app_internal::section::{
    ConfigFileState, ConfigSection, ConnectionOptions, ScheduleSection, Section,
};
use fvm_core::config;

use crate::orchestration::Runtime;

fn setting(value: i64, range: &std::ops::RangeInclusive<i64>) -> MinutesSetting {
    let narrow = |value: i64| u32::try_from(value).expect("minute settings are small and positive");
    MinutesSetting {
        value: narrow(value),
        minimum: narrow(*range.start()),
        maximum: narrow(*range.end()),
    }
}

impl Runtime {
    pub(crate) fn sections(&self) -> Vec<Section> {
        vec![
            Section::Connection {
                section: Box::new(self.cm.connection_section()),
            },
            Section::Schedule {
                section: Box::new(self.schedule_section()),
            },
            Section::Readiness {
                section: Box::new(self.cm.readiness_section()),
            },
            Section::Gmail {
                section: Box::new(self.gmail_section()),
            },
            Section::BypassRules {
                section: Box::new(self.bypass_section()),
            },
            Section::Config {
                section: Box::new(self.config_section()),
            },
        ]
    }

    fn schedule_section(&self) -> ScheduleSection {
        let schedule = &self.config.settings().connection.schedule;
        ScheduleSection {
            enabled: schedule.enabled,
            days: schedule.days.clone(),
            start: schedule.start,
            end: schedule.end,
            away_minutes: setting(schedule.away_minutes, &config::AWAY_MINUTES),
            outlook: self.cm.schedule_outlook(),
            hours_error: self.options.hours_error.clone(),
        }
    }

    fn config_section(&self) -> ConfigSection {
        let settings = self.config.settings();
        let connection = &settings.connection;
        ConfigSection {
            file: ConfigFileState {
                path: self.config.path().display().to_string(),
                exists: self.config.exists(),
                load_error: self.config.load_error().cloned(),
                warnings: self.config.warnings().to_vec(),
            },
            account: self.forms.state(settings),
            options: ConnectionOptions {
                auto_connect_on_launch: connection.auto_connect_on_launch,
                session_max_minutes: setting(
                    connection.session_max_minutes,
                    &config::SESSION_MAX_MINUTES,
                ),
                proactive_reconnect: connection.proactive_reconnect.enabled,
                margin_minutes: setting(
                    connection.proactive_reconnect.margin_minutes,
                    &config::MARGIN_MINUTES,
                ),
                reconnect_after_sleep: connection.reconnect_after_sleep.enabled,
                min_sleep_minutes: setting(
                    connection.reconnect_after_sleep.min_sleep_minutes,
                    &config::MIN_SLEEP_MINUTES,
                ),
            },
            options_error: self.options.options_error.clone(),
            open_at_login: self.options.open_at_login.clone(),
            log_file: self.log_file_path(),
        }
    }
}
