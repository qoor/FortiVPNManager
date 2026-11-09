//! The Account tab's draft: what the user typed, compared with the file it
//! started from. Saving writes only the fields the user changed, so a hand edit of the others
//! survives, then the passwords under the account actually saved. Untouched fields follow the file
//! when it changes elsewhere. An empty password field keeps the stored password.

use std::io;
use std::path::{Path, PathBuf};

use fvm_backend_adapter::openfortivpn::OpenFortiVpn;
use fvm_contract::app_internal::command::AccountForm;
use fvm_contract::app_internal::port::{
    LogCategory, LogLevel, PortRequest, PortResponse, SecretItem, SecretStoreFailure,
};
use fvm_contract::app_internal::problem::FileProblem;
use fvm_contract::app_internal::section::{
    AccountFeedback, AccountFormState, AccountSaveError, ClientSettingsImport, ImportError,
    ImportReport, ImportedPassword,
};
use fvm_contract::app_internal::{ButtonState, SecretText};
use fvm_core::config::Settings;
use fvm_core::connection;

use crate::files;
use crate::orchestration::Runtime;

/// An openfortivpn config file is small; anything larger is not one.
const MAX_IMPORT_BYTES: u64 = 64 * 1024;
const MAX_TRUSTED_CERTS: usize = 4;

/// The non-secret fields as the file holds them, in the form's spelling.
#[derive(Clone, PartialEq, Eq, Debug, Default)]
struct Fields {
    host: String,
    port: String,
    username: String,
    realm: String,
    gmail_address: String,
}

impl Fields {
    fn of(settings: &Settings) -> Self {
        Self {
            host: settings.vpn.host.clone(),
            port: settings.vpn.port.to_string(),
            username: settings.vpn.username.clone(),
            realm: settings.vpn.realm.clone(),
            gmail_address: settings.otp.gmail_address.clone(),
        }
    }
}

pub(crate) struct Forms {
    /// Grows each time the runtime replaces what was typed.
    revision: u64,
    typed: Fields,
    vpn_password: SecretText,
    app_password: SecretText,
    /// The file's fields the draft started from; edits are changes relative to these.
    base: Fields,
    feedback: Option<AccountFeedback>,
    import_report: Option<ClientSettingsImport>,
    default_import_file: Option<String>,
    /// Whether the secret storage holds a password for the typed account and address.
    vpn_password_stored: bool,
    app_password_stored: bool,
    /// The account the stored flags were looked up for.
    pub(crate) looked_up: Option<(String, String, String)>,
    pub(crate) saving: bool,
}

impl Default for Forms {
    fn default() -> Self {
        Self {
            revision: 0,
            typed: Fields::default(),
            vpn_password: SecretText(String::new()),
            app_password: SecretText(String::new()),
            base: Fields::default(),
            feedback: None,
            import_report: None,
            default_import_file: None,
            vpn_password_stored: false,
            app_password_stored: false,
            looked_up: None,
            saving: false,
        }
    }
}

fn trimmed(text: &str) -> String {
    text.trim().to_owned()
}

/// What the app password is stored as: Google shows it in groups of four with spaces.
pub(crate) fn normalized_app_password(password: &str) -> String {
    password.chars().filter(|c| !c.is_whitespace()).collect()
}

impl Forms {
    fn has_changes(&self) -> bool {
        self.typed != self.base
            || !self.vpn_password.0.is_empty()
            || !self.app_password.0.is_empty()
    }

    /// Drops every edit and starts from the file.
    pub(crate) fn reset_account(&mut self, settings: &Settings) {
        self.base = Fields::of(settings);
        self.typed = self.base.clone();
        self.vpn_password = SecretText(String::new());
        self.app_password = SecretText(String::new());
        self.revision += 1;
    }

    /// The file changed: fields the user is editing keep the draft, the others follow the file.
    pub(crate) fn merge_account(&mut self, settings: &Settings) {
        let latest = Fields::of(settings);
        if latest == self.base {
            return;
        }
        let before = self.typed.clone();
        let follow = |typed: &mut String, base: &String, latest: &String| {
            if typed == base {
                typed.clone_from(latest);
            }
        };
        follow(&mut self.typed.host, &self.base.host, &latest.host);
        follow(&mut self.typed.port, &self.base.port, &latest.port);
        follow(
            &mut self.typed.username,
            &self.base.username,
            &latest.username,
        );
        follow(&mut self.typed.realm, &self.base.realm, &latest.realm);
        follow(
            &mut self.typed.gmail_address,
            &self.base.gmail_address,
            &latest.gmail_address,
        );
        self.base = latest;
        if self.typed != before {
            self.revision += 1;
        }
    }

    /// The typed Gmail address, trimmed, as the Gmail test and Google sign-in use it.
    pub(crate) fn typed_gmail_address(&self) -> String {
        trimmed(&self.typed.gmail_address)
    }

    pub(crate) fn typed_app_password(&self) -> Option<String> {
        (!self.app_password.0.is_empty()).then(|| normalized_app_password(&self.app_password.0))
    }

    pub(crate) fn state(&self, settings: &Settings) -> AccountFormState {
        let enabled = |on: bool| {
            if on && !self.saving {
                ButtonState::Enabled
            } else {
                ButtonState::Disabled
            }
        };
        let vpn = &settings.vpn;
        AccountFormState {
            revision: self.revision,
            host: self.typed.host.clone(),
            port: self.typed.port.clone(),
            username: self.typed.username.clone(),
            realm: self.typed.realm.clone(),
            gmail_address: self.typed.gmail_address.clone(),
            vpn_password_entered: !self.vpn_password.0.is_empty(),
            app_password_entered: !self.app_password.0.is_empty(),
            vpn_password_stored: self.vpn_password_stored,
            app_password_stored: self.app_password_stored,
            saved_account_problem: OpenFortiVpn
                .check_account(&vpn.host, &vpn.username, &vpn.realm)
                .err(),
            feedback: self.feedback.clone(),
            save: enabled(self.has_changes()),
            revert: enabled(self.has_changes()),
            import_report: self.import_report.clone(),
            default_import_file: self.default_import_file.clone(),
        }
    }
}

impl Runtime {
    pub(crate) fn edit_account(&mut self, revision: u64, form: AccountForm) {
        if revision != self.forms.revision {
            // Typed against fields the runtime has since replaced.
            return;
        }
        let forms = &mut self.forms;
        forms.typed = Fields {
            host: form.host,
            port: form.port,
            username: form.username,
            realm: form.realm,
            gmail_address: form.gmail_address,
        };
        forms.vpn_password = form.vpn_password;
        forms.app_password = form.app_password;
        if forms.has_changes() {
            forms.feedback = None;
        }
        self.look_up_stored_secrets();
    }

    /// The settings window closed: typed secrets do not linger in memory, here or in the UI.
    pub(crate) fn forget_typed_passwords(&mut self) {
        if self.forms.vpn_password.0.is_empty() && self.forms.app_password.0.is_empty() {
            return;
        }
        self.forms.vpn_password = SecretText(String::new());
        self.forms.app_password = SecretText(String::new());
        self.forms.revision += 1;
    }

    pub(crate) fn revert_account(&mut self) {
        self.forms.feedback = None;
        self.forms.reset_account(&self.config.settings().clone());
        self.look_up_stored_secrets();
    }

    /// Whether the secret storage holds passwords for the typed account and address; asked again
    /// only when those change.
    pub(crate) fn look_up_stored_secrets(&mut self) {
        let typed = &self.forms.typed;
        let key = (
            trimmed(&typed.username),
            trimmed(&typed.host),
            trimmed(&typed.gmail_address),
        );
        if self.forms.looked_up.as_ref() == Some(&key) {
            return;
        }
        self.forms.looked_up = Some(key.clone());
        let (username, host, address) = key.clone();
        let vpn = (!host.is_empty() && !username.is_empty())
            .then_some(SecretItem::VpnPassword { username, host });
        let app = (!address.is_empty()).then_some(SecretItem::GmailAppPassword { address });
        self.has_secret(vpn, move |runtime, vpn_stored| {
            runtime.has_secret(app, move |runtime, app_stored| {
                // A newer lookup started meanwhile has the say.
                if runtime.forms.looked_up.as_ref() == Some(&key) {
                    runtime.forms.vpn_password_stored = vpn_stored;
                    runtime.forms.app_password_stored = app_stored;
                }
            });
        });
    }

    /// "Save": checks the draft, writes the changed fields, then the passwords under the account
    /// that was saved.
    pub(crate) fn save_account(&mut self) {
        let forms = &mut self.forms;
        forms.feedback = None;
        let fail = |forms: &mut Forms, error: AccountSaveError| {
            forms.feedback = Some(AccountFeedback::Failed { error })
        };
        let Some(port) = trimmed(&forms.typed.port)
            .parse::<i64>()
            .ok()
            .filter(|port| fvm_core::config::PORTS.contains(port))
        else {
            return fail(forms, AccountSaveError::InvalidPort {});
        };
        let host = trimmed(&forms.typed.host);
        let username = trimmed(&forms.typed.username);
        let realm = trimmed(&forms.typed.realm);
        let address = trimmed(&forms.typed.gmail_address);
        let vpn_password = forms.vpn_password.0.clone();
        let app_password = forms.app_password.0.clone();
        if !vpn_password.is_empty() && (host.is_empty() || username.is_empty()) {
            return fail(forms, AccountSaveError::PasswordNeedsAccount {});
        }
        if !app_password.is_empty() && address.is_empty() {
            return fail(forms, AccountSaveError::AppPasswordNeedsAddress {});
        }
        if let Err(problem) = OpenFortiVpn.check_account(&host, &username, &realm) {
            return fail(forms, AccountSaveError::AccountValue { problem });
        }
        if !vpn_password.is_empty()
            && let Err(error) = OpenFortiVpn.check_vpn_password(&vpn_password)
        {
            return fail(forms, error);
        }
        let (edited, original) = (forms.typed.clone(), forms.base.clone());
        let written = self.config.update(|settings| {
            if edited.host != original.host {
                settings.vpn.host.clone_from(&host);
            }
            if edited.port != original.port {
                settings.vpn.port = port;
            }
            if edited.username != original.username {
                settings.vpn.username.clone_from(&username);
            }
            if edited.realm != original.realm {
                settings.vpn.realm.clone_from(&realm);
            }
            if edited.gmail_address != original.gmail_address {
                settings.otp.gmail_address.clone_from(&address);
            }
        });
        if let Err(error) = written {
            let error = AccountSaveError::Config { error };
            self.forms.feedback = Some(AccountFeedback::Failed { error });
            return;
        }
        self.config_reloaded();
        let saved = self.config.settings().clone();
        let mut writes = Vec::new();
        if !vpn_password.is_empty() {
            let item = SecretItem::VpnPassword {
                username: saved.vpn.username.clone(),
                host: saved.vpn.host.clone(),
            };
            writes.push(PortRequest::WriteSecret {
                item,
                value: SecretText(vpn_password),
            });
        }
        if !app_password.is_empty() {
            let item = SecretItem::GmailAppPassword {
                address: saved.otp.gmail_address.clone(),
            };
            let normalized = normalized_app_password(&app_password);
            writes.push(if normalized.is_empty() {
                PortRequest::DeleteSecret { item }
            } else {
                PortRequest::WriteSecret {
                    item,
                    value: SecretText(normalized),
                }
            });
        }
        self.forms.saving = true;
        self.write_secrets(writes, |runtime, failure| {
            runtime.forms.saving = false;
            runtime.forms.feedback = Some(match failure {
                None => AccountFeedback::Saved {},
                Some(failure) => AccountFeedback::Failed {
                    error: AccountSaveError::SecretStore { failure },
                },
            });
            runtime
                .forms
                .reset_account(&runtime.config.settings().clone());
            runtime.forms.looked_up = None;
            runtime.look_up_stored_secrets();
            // Secret storage changes raise no event of their own.
            runtime.feed(connection::Input::RecheckReadiness);
        });
    }

    /// Writes the secrets in order, stopping at the first failure.
    fn write_secrets(
        &mut self,
        mut writes: Vec<PortRequest>,
        done: impl FnOnce(&mut Runtime, Option<SecretStoreFailure>) + 'static,
    ) {
        if writes.is_empty() {
            return done(self, None);
        }
        let request = writes.remove(0);
        self.request(request, move |runtime, response| match response {
            PortResponse::SecretChanged {} => runtime.write_secrets(writes, done),
            PortResponse::SecretStoreFailed { failure } => done(runtime, Some(failure)),
            other => panic!("a secret write answered with {other:?}"),
        });
    }

    pub(crate) fn dismiss_import_report(&mut self) {
        self.forms.import_report = None;
    }

    /// The first openfortivpn config file found where openfortivpn looks by default.
    pub(crate) fn find_default_import_file(&mut self) {
        let found = OpenFortiVpn
            .default_settings_files(&self.home)
            .into_iter()
            .find(|path| path.exists());
        self.forms.default_import_file = found.map(|path| path.display().to_string());
    }

    /// "Import openfortivpn Settings" with the chosen file, or the default one.
    pub(crate) fn import_client_settings(&mut self, file: Option<String>) {
        let source = file.map(PathBuf::from).or_else(|| {
            OpenFortiVpn
                .default_settings_files(&self.home)
                .into_iter()
                .find(|path| path.exists())
        });
        let Some(source) = source else {
            return;
        };
        self.import_from(
            &source,
            Box::new(|runtime: &mut Runtime| {
                runtime.forms.vpn_password = SecretText(String::new());
                runtime.forms.feedback = None;
                runtime
                    .forms
                    .reset_account(&runtime.config.settings().clone());
                runtime.forms.looked_up = None;
                runtime.look_up_stored_secrets();
                runtime.feed(connection::Input::RecheckReadiness);
            }),
        );
    }

    /// The first launch: imports when there is no config.yaml yet and an openfortivpn config
    /// exists, then calls `then` with whether a report is shown.
    pub(crate) fn import_on_first_launch(
        &mut self,
        then: impl FnOnce(&mut Runtime, bool) + 'static,
    ) {
        let source = (!self.config.exists())
            .then(|| {
                OpenFortiVpn
                    .default_settings_files(&self.home)
                    .into_iter()
                    .find(|path| path.exists())
            })
            .flatten();
        match source {
            Some(source) => self.import_from(
                &source,
                Box::new(move |runtime: &mut Runtime| then(runtime, true)),
            ),
            None => then(self, false),
        }
    }

    /// Replaces `vpn.*` from the openfortivpn config at `source`, saves its password under the
    /// imported account, and keeps the report. Certificate values merge when the gateway is the
    /// same and are replaced otherwise. The source file is only read.
    fn import_from(&mut self, source: &Path, done: Box<dyn FnOnce(&mut Runtime)>) {
        let path = source.display().to_string();
        let bytes = match files::read_regular(source, MAX_IMPORT_BYTES) {
            Ok(Some(bytes)) => bytes,
            Ok(None) => {
                return self.finish_import(
                    ClientSettingsImport::Failed {
                        error: ImportError::SourceNotFound { path },
                    },
                    done,
                );
            }
            Err(error) => {
                let problem = file_problem(&error, &path);
                return self.finish_import(
                    ClientSettingsImport::Failed {
                        error: ImportError::Unreadable { path, problem },
                    },
                    done,
                );
            }
        };
        let imported = OpenFortiVpn.read_settings(&bytes);
        let Some(host) = imported.host.clone() else {
            return self.finish_import(
                ClientSettingsImport::Failed {
                    error: ImportError::MissingHost { path },
                },
                done,
            );
        };
        let port = i64::from(imported.port);
        let written = self.config.update(|settings| {
            let same_gateway = settings.vpn.host == host && settings.vpn.port == port;
            settings.vpn.host.clone_from(&host);
            settings.vpn.port = port;
            if let Some(username) = &imported.username {
                settings.vpn.username.clone_from(username);
            }
            settings.vpn.realm = imported.realm.clone().unwrap_or_default();
            settings.vpn.trusted_certs = if same_gateway {
                connection::merge_trusted_certs(
                    &imported.trusted_certs,
                    &settings.vpn.trusted_certs,
                )
            } else {
                imported
                    .trusted_certs
                    .iter()
                    .take(MAX_TRUSTED_CERTS)
                    .cloned()
                    .collect()
            };
        });
        if let Err(error) = written {
            return self.finish_import(
                ClientSettingsImport::Failed {
                    error: ImportError::Config { error },
                },
                done,
            );
        }
        self.config_reloaded();
        let saved = self.config.settings().clone();
        let report = ImportReport {
            source: path.clone(),
            host: saved.vpn.host.clone(),
            port: u16::try_from(saved.vpn.port).expect("the settings file checks the port range"),
            username: saved.vpn.username.clone(),
            realm: saved.vpn.realm.clone(),
            trusted_certs: u32::try_from(saved.vpn.trusted_certs.len())
                .expect("at most four values"),
            password: None,
            unknown_keys: imported.unknown_keys.clone(),
            ignored_keys: imported.ignored_keys.clone(),
            unreadable_lines: imported.unreadable_lines.clone(),
        };
        self.system_log(
            LogLevel::Notice,
            LogCategory::Import,
            format!(
                "imported {path}: port {}, {} pin(s), unknown keys: {:?}",
                report.port, report.trusted_certs, report.unknown_keys
            ),
        );
        let Some(password) = imported.password else {
            return self.finish_import(ClientSettingsImport::Imported { report }, done);
        };
        if saved.vpn.username.is_empty() {
            let report = ImportReport {
                password: Some(ImportedPassword::NotSavedWithoutUsername {}),
                ..report
            };
            return self.finish_import(ClientSettingsImport::Imported { report }, done);
        }
        let item = SecretItem::VpnPassword {
            username: saved.vpn.username,
            host: saved.vpn.host,
        };
        self.request(
            PortRequest::WriteSecret {
                item,
                value: password,
            },
            move |runtime, response| {
                let password = match response {
                    PortResponse::SecretChanged {} => ImportedPassword::Saved {},
                    PortResponse::SecretStoreFailed { failure } => {
                        ImportedPassword::NotSaved { failure }
                    }
                    other => panic!("a secret write answered with {other:?}"),
                };
                let report = ImportReport {
                    password: Some(password),
                    ..report
                };
                runtime.finish_import(ClientSettingsImport::Imported { report }, done);
            },
        );
    }

    fn finish_import(
        &mut self,
        outcome: ClientSettingsImport,
        done: Box<dyn FnOnce(&mut Runtime)>,
    ) {
        self.forms.import_report = Some(outcome);
        done(self);
    }
}

fn file_problem(error: &io::Error, path: &str) -> FileProblem {
    match error.kind() {
        io::ErrorKind::InvalidInput => FileProblem::NotRegularFile {},
        io::ErrorKind::InvalidData => FileProblem::TooLarge {
            limit_bytes: MAX_IMPORT_BYTES,
        },
        _ => FileProblem::Io {
            operation: "read".into(),
            path: path.into(),
            errno: error.raw_os_error().unwrap_or(0),
        },
    }
}
