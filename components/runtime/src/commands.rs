//! The UI's commands and the settings they change directly: the connection
//! options and the schedule, the OTP method, the login item and the Helper's registration.

use fvm_contract::app_internal::TimeOfDay;
use fvm_contract::app_internal::command::{Command, ConnectionOption};
use fvm_contract::app_internal::port::{
    LogCategory, LogLevel, PortAction, PortRequest, PortResponse, RegistrationStatus,
};
use fvm_contract::app_internal::problem::ConfigFileError;
use fvm_contract::app_internal::section::{OpenAtLogin, ScheduleHoursError};
use fvm_core::config::{Settings, normalized_days};
use fvm_core::connection;

use crate::orchestration::Runtime;

/// The settings the General and Connection panes change outside the Account draft, and the login item.
pub(crate) struct Options {
    pub(crate) options_error: Option<ConfigFileError>,
    pub(crate) hours_error: Option<ScheduleHoursError>,
    pub(crate) open_at_login: OpenAtLogin,
}

impl Options {
    pub(crate) fn new(installed_copy: bool) -> Self {
        Self {
            options_error: None,
            hours_error: None,
            open_at_login: OpenAtLogin {
                available: installed_copy,
                enabled: false,
                requires_approval: false,
                error: None,
            },
        }
    }
}

impl Runtime {
    pub(crate) fn command(&mut self, command: Command) {
        match command {
            Command::Connect {} => self.feed(connection::Input::Connect),
            Command::Disconnect {} => self.feed(connection::Input::Disconnect),
            Command::ReconnectNow {} => self.feed(connection::Input::ReconnectNow),
            Command::SettingsOpened {} => {
                self.poll_config();
                self.find_default_import_file();
                self.refresh_login_item();
                self.forms.looked_up = None;
                self.look_up_stored_secrets();
                self.refresh_google_state();
                self.feed(connection::Input::SettingsOpened);
            }
            Command::SettingsClosed {} => self.forget_typed_passwords(),
            Command::EditAccount { revision, form } => self.edit_account(revision, form),
            Command::SaveAccount {} => self.save_account(),
            Command::RevertAccount {} => self.revert_account(),
            Command::ImportClientSettings { file } => self.import_client_settings(file),
            Command::DismissImportReport {} => self.dismiss_import_report(),
            Command::SetOtpMethod { method } => self.set_otp_method(method),
            Command::TestGmail {} => self.test_gmail(),
            Command::ConnectGoogleAccount {} => self.connect_google_account(),
            Command::CancelGoogleSignIn {} => self.cancel_google_sign_in(),
            Command::DisconnectGoogleAccount {} => self.disconnect_google_account(),
            Command::ImportOAuthClient { file } => self.import_oauth_client(&file),
            Command::UseBuiltInOAuthClient {} => self.use_built_in_oauth_client(),
            Command::SetConnectionOption { option } => self.set_connection_option(option),
            Command::SetScheduleHours { start, end } => self.set_schedule_hours(start, end),
            Command::SetOpenAtLogin { enabled } => self.set_open_at_login(enabled),
            Command::RegisterHelper {} => {
                self.request(PortRequest::RegisterHelper {}, |runtime, response| {
                    let result = match response {
                        PortResponse::Registered {} => Ok(()),
                        PortResponse::RegistrationFailed { error } => Err(error),
                        other => panic!("a registration answered with {other:?}"),
                    };
                    runtime.feed(connection::Input::HelperRegistered(result));
                })
            }
            Command::OpenHelperApprovalSettings {} => {
                self.action(PortAction::OpenHelperApprovalSettings {});
                self.feed(connection::Input::HelperApprovalSettingsOpened);
            }
            Command::SetBypassRows { list, rows } => self.set_bypass_rows(list, rows),
            Command::AddCurrentNetwork {} => self.add_current_network(),
            Command::DiscardBypassChanges {} => self.discard_bypass_changes(),
            Command::ApplyBypassRules {} => self.apply_bypass_rules(),
        }
    }

    /// Writes one setting the General or Connection pane changed and hands the result to the core.
    fn change_settings(&mut self, change: impl Fn(&mut Settings)) -> Result<(), ConfigFileError> {
        if self.config.update(change)? {
            self.config_reloaded();
        }
        Ok(())
    }

    fn set_connection_option(&mut self, option: ConnectionOption) {
        let result = self.change_settings(|settings| {
            let connection = &mut settings.connection;
            match option {
                ConnectionOption::ScheduleEnabled { enabled } => {
                    connection.schedule.enabled = enabled
                }
                ConnectionOption::ScheduleDay { day, selected } => {
                    let days = &mut connection.schedule.days;
                    days.retain(|other| *other != day);
                    if selected {
                        days.push(day);
                        *days = normalized_days(days);
                    }
                }
                ConnectionOption::AwayMinutes { minutes } => {
                    connection.schedule.away_minutes = minutes
                }
                ConnectionOption::AutoConnectOnLaunch { enabled } => {
                    connection.auto_connect_on_launch = enabled
                }
                ConnectionOption::SessionMaxMinutes { minutes } => {
                    connection.session_max_minutes = minutes
                }
                ConnectionOption::ProactiveReconnect { enabled } => {
                    connection.proactive_reconnect.enabled = enabled
                }
                ConnectionOption::ProactiveMarginMinutes { minutes } => {
                    connection.proactive_reconnect.margin_minutes = minutes
                }
                ConnectionOption::ReconnectAfterSleep { enabled } => {
                    connection.reconnect_after_sleep.enabled = enabled
                }
                ConnectionOption::MinSleepMinutes { minutes } => {
                    connection.reconnect_after_sleep.min_sleep_minutes = minutes
                }
            }
        });
        self.options.options_error = result.err();
    }

    /// Saved only when the end is later than the start; until then the pickers show the problem.
    fn set_schedule_hours(&mut self, start: TimeOfDay, end: TimeOfDay) {
        let minutes = |time: TimeOfDay| u32::from(time.hour) * 60 + u32::from(time.minute);
        if minutes(end) <= minutes(start) {
            self.options.hours_error = Some(ScheduleHoursError::EndNotAfterStart {});
            return;
        }
        let result = self.change_settings(|settings| {
            settings.connection.schedule.start = start;
            settings.connection.schedule.end = end;
        });
        self.options.hours_error = result
            .err()
            .map(|error| ScheduleHoursError::NotSaved { error });
    }

    fn set_otp_method(&mut self, method: fvm_contract::app_internal::OtpMethod) {
        if method == self.config.settings().otp.method {
            return;
        }
        let result = self.change_settings(|settings| settings.otp.method = method);
        self.gmail.method_error = result.err();
        self.google.error = None;
        self.reset_gmail_test();
    }

    // MARK: Login item

    pub(crate) fn refresh_login_item(&mut self) {
        self.options.open_at_login.available = self.facts.installed_copy;
        self.request(
            PortRequest::LoginItemRegistration {},
            |runtime, response| {
                let PortResponse::Registration { status } = response else {
                    panic!("a registration answer")
                };
                let open_at_login = &mut runtime.options.open_at_login;
                open_at_login.enabled = matches!(
                    status,
                    RegistrationStatus::Enabled | RegistrationStatus::RequiresApproval
                );
                open_at_login.requires_approval = status == RegistrationStatus::RequiresApproval;
            },
        );
    }

    fn set_open_at_login(&mut self, enabled: bool) {
        self.request(
            PortRequest::SetLoginItem { enabled },
            |runtime, response| {
                runtime.options.open_at_login.error = match response {
                    PortResponse::Registered {} => None,
                    PortResponse::RegistrationFailed { error } => Some(error),
                    other => panic!("a login item change answered with {other:?}"),
                };
                runtime.refresh_login_item();
            },
        );
    }

    /// The first launch of the copy in /Applications opens the app at login once; turning it off
    /// later is the user's choice.
    pub(crate) fn register_login_item_once(&mut self) {
        if !self.facts.installed_copy || self.state.state().login_item_registered {
            self.refresh_login_item();
            return;
        }
        self.request(
            PortRequest::SetLoginItem { enabled: true },
            |runtime, response| {
                match response {
                    PortResponse::Registered {} => {
                        if let Err(problem) = runtime
                            .state
                            .update(|state| state.login_item_registered = true)
                        {
                            runtime.system_log(LogLevel::Error, LogCategory::State, problem);
                        }
                    }
                    PortResponse::RegistrationFailed { error } => {
                        runtime.system_log(
                            LogLevel::Error,
                            LogCategory::LoginItem,
                            format!("open at login failed: {error:?}"),
                        );
                    }
                    other => panic!("a login item change answered with {other:?}"),
                }
                runtime.refresh_login_item();
            },
        );
    }
}
