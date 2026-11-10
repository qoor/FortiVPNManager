//! Gathering the readiness facts the core judges: the settings file, the account,
//! the stored secrets and the Helper. A Helper of another build is asked to exit when idle, so the
//! system starts this build's Helper, and is checked once more.

use std::time::Duration;

use fvm_backend_adapter::openfortivpn::OpenFortiVpn;
use fvm_contract::app_helper::HelperStatus;
use fvm_contract::app_internal::OtpMethod;
use fvm_contract::app_internal::port::{
    HelperCall, LogCategory, LogLevel, PortRequest, PortResponse, RegistrationStatus, SecretItem,
};
use fvm_core::connection::{self, HelperFacts, ReadinessFacts};

use crate::orchestration::Runtime;

/// Gives the system time to start the replaced Helper.
const HELPER_RESTART_WAIT: Duration = Duration::from_secs(1);

impl Runtime {
    pub(crate) fn check_readiness(&mut self) {
        self.request(PortRequest::HelperRegistration {}, |runtime, response| {
            let PortResponse::Registration { status } = response else {
                panic!("a registration answer")
            };
            match status {
                RegistrationStatus::NotRegistered | RegistrationStatus::NotFound => {
                    runtime.gather_readiness(HelperFacts::NotRegistered);
                }
                RegistrationStatus::RequiresApproval => {
                    runtime.gather_readiness(HelperFacts::RequiresApproval)
                }
                RegistrationStatus::Enabled => runtime.ask_helper_build(true),
            }
        });
    }

    fn ask_helper_build(&mut self, may_restart: bool) {
        self.request(
            PortRequest::HelperBuild {},
            move |runtime, response| match response {
                PortResponse::HelperBuild { build } if build != runtime.facts.app_build => {
                    if may_restart && runtime.cm.should_restart_helper(&build) {
                        runtime.system_log(
                            LogLevel::Notice,
                            LogCategory::Readiness,
                            format!(
                                "helper build {build} != app {}; restarting it",
                                runtime.facts.app_build
                            ),
                        );
                        runtime.call_helper(HelperCall::ExitWhenIdle {}, |runtime, _| {
                            let queue = runtime.queue();
                            runtime.tasks.spawn(async move {
                                tokio::time::sleep(HELPER_RESTART_WAIT).await;
                                queue.run(|runtime| runtime.ask_helper_build(false));
                            });
                        });
                    } else {
                        runtime.gather_readiness(HelperFacts::Answered {
                            build,
                            status: None,
                        });
                    }
                }
                PortResponse::HelperBuild { build } => {
                    runtime.call_helper(HelperCall::Status {}, move |runtime, response| {
                        let status = match response {
                            PortResponse::HelperReply { json: Some(json) } => {
                                serde_json::from_str::<HelperStatus>(&json).ok()
                            }
                            _ => None,
                        };
                        runtime.gather_readiness(HelperFacts::Answered { build, status });
                    });
                }
                PortResponse::HelperUnreachable { failure } => {
                    runtime.system_log(
                        LogLevel::Error,
                        LogCategory::Readiness,
                        format!("helper check failed: {failure:?}"),
                    );
                    runtime.gather_readiness(HelperFacts::Unreachable(failure));
                }
                other => panic!("a Helper build answered with {other:?}"),
            },
        );
    }

    /// The secrets come after the Helper's round trip, so the facts reflect the latest file.
    fn gather_readiness(&mut self, helper: HelperFacts) {
        let answered = matches!(&helper, HelperFacts::Answered { build, .. } if *build == self.facts.app_build);
        if answered && !self.bypass.helper_ready {
            // A Helper registered, approved or updated since: the routes can be synced now.
            self.sync_bypass_routes();
        }
        self.bypass.helper_ready = answered;
        let settings = self.config.settings().clone();
        let vpn = &settings.vpn;
        let address = settings.otp.gmail_address.clone();
        let account_values_usable = !vpn.host.is_empty()
            && !vpn.username.is_empty()
            && OpenFortiVpn
                .check_account(&vpn.host, &vpn.username, &vpn.realm)
                .is_ok();
        let mut facts = ReadinessFacts {
            config_valid: self.config.is_valid(),
            account_complete: account_values_usable && !address.is_empty(),
            vpn_password_stored: false,
            gmail_credential_stored: false,
            helper,
        };
        let vpn_item =
            (!vpn.host.is_empty() && !vpn.username.is_empty()).then(|| SecretItem::VpnPassword {
                username: vpn.username.clone(),
                host: vpn.host.clone(),
            });
        let method = settings.otp.method;
        self.has_secret(vpn_item, move |runtime, vpn_password_stored| {
            facts.vpn_password_stored = vpn_password_stored;
            let app_password = (!address.is_empty()).then(|| SecretItem::GmailAppPassword {
                address: address.clone(),
            });
            runtime.has_secret(app_password, move |runtime, app_password_stored| {
                if app_password_stored || method != OtpMethod::GmailApi || address.is_empty() {
                    facts.gmail_credential_stored = app_password_stored;
                    runtime.feed(connection::Input::ReadinessChecked(facts));
                    return;
                }
                runtime.gmail_api_authorized(&address, move |runtime, authorized| {
                    facts.gmail_credential_stored = authorized;
                    runtime.feed(connection::Input::ReadinessChecked(facts));
                });
            });
        });
    }

    /// Whether the secret storage holds `item`; no item, or a failed lookup, is no.
    pub(crate) fn has_secret(
        &mut self,
        item: Option<SecretItem>,
        then: impl FnOnce(&mut Runtime, bool) + 'static,
    ) {
        let Some(item) = item else {
            then(self, false);
            return;
        };
        self.request(
            PortRequest::HasSecret { item },
            move |runtime, response| match response {
                PortResponse::HasSecret { exists } => then(runtime, exists),
                PortResponse::SecretStoreFailed { failure } => {
                    runtime.system_log(
                        LogLevel::Error,
                        LogCategory::Readiness,
                        format!("keychain lookup failed: {}", failure.description),
                    );
                    then(runtime, false);
                }
                other => panic!("a secret lookup answered with {other:?}"),
            },
        );
    }
}
