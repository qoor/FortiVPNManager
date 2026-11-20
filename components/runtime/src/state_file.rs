//! state.json: what the device keeps that nobody edits.

use std::io;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use fvm_contract::app_helper::HelperErrorCode;
use fvm_core::auto_update::Version;
use fvm_core::connection::{Failure, FailureLedger};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Deserializer, Serialize};

use crate::files;

/// The state file holds a few small values; anything larger is not one.
const MAX_FILE_BYTES: u64 = 1 << 20;

#[derive(Clone, PartialEq, Debug, Default)]
pub(crate) struct State {
    pub(crate) ledger: Option<FailureLedger>,
    pub(crate) login_item_registered: bool,
    pub(crate) last_notified_update: Option<Version>,
    pub(crate) update_target: Option<Version>,
    pub(crate) connected_before_update: bool,
}

pub(crate) struct StateFile {
    path: PathBuf,
    state: State,
}

impl StateFile {
    /// Reads the file; a missing or unreadable one starts empty and is replaced on the next save.
    pub(crate) fn open(path: PathBuf) -> (Self, Option<String>) {
        let (state, problem) = match files::read_regular(&path, MAX_FILE_BYTES) {
            Ok(Some(bytes)) => match serde_json::from_slice::<Wire>(&bytes) {
                Ok(wire) => (wire.into_state(), None),
                Err(error) => (
                    State::default(),
                    Some(format!("ignoring unreadable state.json: {error}")),
                ),
            },
            Ok(None) => (State::default(), None),
            Err(error) => (
                State::default(),
                Some(format!("ignoring unreadable state.json: {error}")),
            ),
        };
        (Self { path, state }, problem)
    }

    pub(crate) fn state(&self) -> &State {
        &self.state
    }

    /// Applies `change` and writes the file when the state changed; the error describes a failed
    /// write, after which the state in memory still holds the change.
    pub(crate) fn update(&mut self, change: impl FnOnce(&mut State)) -> Result<(), String> {
        let mut next = self.state.clone();
        change(&mut next);
        if next == self.state {
            return Ok(());
        }
        self.state = next;
        let mut bytes =
            serde_json::to_vec_pretty(&Wire::from_state(&self.state)).expect("state.json encodes");
        bytes.push(b'\n');
        write(&self.path, &bytes).map_err(|error| format!("saving state.json failed: {error}"))
    }
}

fn write(path: &Path, bytes: &[u8]) -> io::Result<()> {
    if let Some(directory) = path.parent() {
        files::make_directory(directory)?;
    }
    files::write_atomically(path, bytes)
}

// MARK: Wire format

/// Earlier builds also wrote `appliedExceptionRoutes`, the exception routes they kept in the OS's
/// saved settings: it is ignored when read and left out when written, and those builds read a file
/// without it.
#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Wire {
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "or_absent"
    )]
    ledger: Option<WireLedger>,
    #[serde(default, deserialize_with = "or_absent")]
    login_item_registered: bool,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "optional_version"
    )]
    last_notified_update_version: Option<Version>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "optional_version"
    )]
    update_target_version: Option<Version>,
    #[serde(
        default,
        skip_serializing_if = "std::ops::Not::not",
        deserialize_with = "or_absent"
    )]
    connected_before_update: bool,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct WireLedger {
    profile_key: String,
    #[serde(default, deserialize_with = "or_absent")]
    consecutive_auth_failures: u32,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "optional_time"
    )]
    last_auth_failure_at: Option<SystemTime>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "or_absent"
    )]
    last_auth_failure_reason: Option<WireFailure>,
    #[serde(default, with = "times")]
    recent_auth_attempts: Vec<SystemTime>,
}

/// A case name holding its values, as in the app's other JSON.
#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
enum WireFailure {
    AuthenticationRejected {},
    OtpNotReceived {
        #[serde(rename = "inJunk")]
        in_junk: bool,
    },
    OtpRejected {},
    TunnelSetupFailed {},
    CertificateRejectedAfterLogin {},
    GmailAppPassword {},
    TooManyReconnects {},
    Network {},
    Helper {
        #[serde(rename = "_0")]
        code: HelperErrorCode,
    },
}

impl Wire {
    fn into_state(self) -> State {
        State {
            ledger: self.ledger.map(|ledger| FailureLedger {
                account_key: ledger.profile_key,
                consecutive_auth_failures: ledger.consecutive_auth_failures,
                last_auth_failure_at: ledger.last_auth_failure_at,
                last_auth_failure: ledger
                    .last_auth_failure_reason
                    .map(WireFailure::into_failure),
                recent_auth_attempts: ledger.recent_auth_attempts,
            }),
            login_item_registered: self.login_item_registered,
            last_notified_update: self.last_notified_update_version,
            update_target: self.update_target_version,
            connected_before_update: self.connected_before_update,
        }
    }

    fn from_state(state: &State) -> Self {
        Self {
            ledger: state.ledger.as_ref().map(|ledger| WireLedger {
                profile_key: ledger.account_key.clone(),
                consecutive_auth_failures: ledger.consecutive_auth_failures,
                last_auth_failure_at: ledger.last_auth_failure_at,
                last_auth_failure_reason: ledger.last_auth_failure.map(WireFailure::from_failure),
                recent_auth_attempts: ledger.recent_auth_attempts.clone(),
            }),
            login_item_registered: state.login_item_registered,
            last_notified_update_version: state.last_notified_update,
            update_target_version: state.update_target,
            connected_before_update: state.connected_before_update,
        }
    }
}

impl WireFailure {
    fn into_failure(self) -> Failure {
        match self {
            Self::AuthenticationRejected {} => Failure::AuthenticationRejected,
            Self::OtpNotReceived { in_junk } => Failure::OtpNotReceived { in_spam: in_junk },
            Self::OtpRejected {} => Failure::OtpRejected,
            Self::TunnelSetupFailed {} => Failure::TunnelSetupFailed,
            Self::CertificateRejectedAfterLogin {} => Failure::CertificateRejectedAfterLogin,
            Self::GmailAppPassword {} => Failure::GmailRejected,
            Self::TooManyReconnects {} => Failure::TooManyReconnects,
            Self::Network {} => Failure::Network,
            Self::Helper { code } => Failure::Helper { code },
        }
    }

    fn from_failure(failure: Failure) -> Self {
        match failure {
            Failure::AuthenticationRejected => Self::AuthenticationRejected {},
            Failure::OtpNotReceived { in_spam } => Self::OtpNotReceived { in_junk: in_spam },
            Failure::OtpRejected => Self::OtpRejected {},
            Failure::TunnelSetupFailed => Self::TunnelSetupFailed {},
            Failure::CertificateRejectedAfterLogin => Self::CertificateRejectedAfterLogin {},
            Failure::GmailRejected => Self::GmailAppPassword {},
            Failure::TooManyReconnects => Self::TooManyReconnects {},
            Failure::Network => Self::Network {},
            Failure::Helper { code } => Self::Helper { code },
        }
    }
}

fn or_absent<'de, D: Deserializer<'de>, T: DeserializeOwned + Default>(
    deserializer: D,
) -> Result<T, D::Error> {
    let value = serde_json::Value::deserialize(deserializer)?;
    Ok(T::deserialize(value).unwrap_or_default())
}

/// `2026-10-15T06:30:21Z`: whole seconds in UTC, readable when someone opens the file.
fn format_time(time: SystemTime) -> String {
    let seconds = time
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let timestamp =
        jiff::Timestamp::from_second(i64::try_from(seconds).expect("a time within the calendar"))
            .expect("a time within the calendar");
    timestamp.strftime("%Y-%m-%dT%H:%M:%SZ").to_string()
}

fn parse_time(text: &str) -> Option<SystemTime> {
    let timestamp: jiff::Timestamp = text.parse().ok()?;
    let seconds = u64::try_from(timestamp.as_second()).ok()?;
    Some(UNIX_EPOCH + Duration::from_secs(seconds))
}

mod optional_time {
    use std::time::SystemTime;

    use serde::{Deserialize, Deserializer, Serializer};

    pub(super) fn serialize<S: Serializer>(
        time: &Option<SystemTime>,
        serializer: S,
    ) -> Result<S::Ok, S::Error> {
        let time = time.expect("skipped when absent");
        serializer.serialize_str(&super::format_time(time))
    }

    pub(super) fn deserialize<'de, D: Deserializer<'de>>(
        deserializer: D,
    ) -> Result<Option<SystemTime>, D::Error> {
        let value = serde_json::Value::deserialize(deserializer)?;
        Ok(value.as_str().and_then(super::parse_time))
    }
}

mod optional_version {
    use fvm_core::auto_update::Version;
    use serde::{Deserialize, Deserializer, Serializer};

    pub(super) fn serialize<S: Serializer>(
        version: &Option<Version>,
        serializer: S,
    ) -> Result<S::Ok, S::Error> {
        let version = version.expect("skipped when absent");
        serializer.collect_str(&version)
    }

    pub(super) fn deserialize<'de, D: Deserializer<'de>>(
        deserializer: D,
    ) -> Result<Option<Version>, D::Error> {
        let value = serde_json::Value::deserialize(deserializer)?;
        Ok(value.as_str().and_then(Version::parse))
    }
}

mod times {
    use std::time::SystemTime;

    use serde::{Deserialize, Deserializer, Serializer, ser::SerializeSeq};

    pub(super) fn serialize<S: Serializer>(
        times: &[SystemTime],
        serializer: S,
    ) -> Result<S::Ok, S::Error> {
        let mut sequence = serializer.serialize_seq(Some(times.len()))?;
        for time in times {
            sequence.serialize_element(&super::format_time(*time))?;
        }
        sequence.end()
    }

    pub(super) fn deserialize<'de, D: Deserializer<'de>>(
        deserializer: D,
    ) -> Result<Vec<SystemTime>, D::Error> {
        let value = serde_json::Value::deserialize(deserializer)?;
        Ok(value
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|time| time.as_str().and_then(super::parse_time))
            .collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// As an earlier build wrote it, with the exception routes it kept.
    const SAVED_FILE: &str = r#"{
  "appliedExceptionRoutes" : {
    "F7929342-1853-4DEF-B9C0-9974F585CED1" : [
      {
        "destination" : "10.30.161.16",
        "gateway" : "10.20.161.254",
        "subnetMask" : "255.255.255.255"
      }
    ]
  },
  "ledger" : {
    "consecutiveAuthFailures" : 1,
    "lastAuthFailureAt" : "2026-10-15T06:30:21Z",
    "lastAuthFailureReason" : {
      "otpNotReceived" : {
        "inJunk" : false
      }
    },
    "profileKey" : "alice@vpn.example.com",
    "recentAuthAttempts" : [
      "2026-11-09T04:08:26Z"
    ]
  },
  "loginItemRegistered" : true
}"#;

    fn at(text: &str) -> SystemTime {
        parse_time(text).unwrap()
    }

    #[test]
    fn a_saved_file_reads_whole() {
        let state = serde_json::from_str::<Wire>(SAVED_FILE)
            .unwrap()
            .into_state();
        assert_eq!(
            state.ledger,
            Some(FailureLedger {
                account_key: "alice@vpn.example.com".into(),
                consecutive_auth_failures: 1,
                last_auth_failure_at: Some(at("2026-10-15T06:30:21Z")),
                last_auth_failure: Some(Failure::OtpNotReceived { in_spam: false }),
                recent_auth_attempts: vec![at("2026-11-09T04:08:26Z")],
            })
        );
        assert!(state.login_item_registered);
    }

    #[test]
    fn a_failure_this_build_does_not_know_leaves_the_rest_of_the_ledger() {
        let file = r#"{
  "ledger" : {
    "consecutiveAuthFailures" : 1,
    "lastAuthFailureAt" : "2026-10-15T06:30:21Z",
    "lastAuthFailureReason" : {
      "sessionDroppedEarly" : {}
    },
    "profileKey" : "alice@vpn.example.com",
    "recentAuthAttempts" : [
      "2026-10-15T06:28:02Z",
      "2026-10-15T06:30:01Z"
    ]
  },
  "loginItemRegistered" : true
}"#;
        let state = serde_json::from_str::<Wire>(file).unwrap().into_state();
        assert_eq!(
            state.ledger,
            Some(FailureLedger {
                account_key: "alice@vpn.example.com".into(),
                consecutive_auth_failures: 1,
                last_auth_failure_at: Some(at("2026-10-15T06:30:21Z")),
                last_auth_failure: None,
                recent_auth_attempts: vec![at("2026-10-15T06:28:02Z"), at("2026-10-15T06:30:01Z")],
            })
        );
        assert!(state.login_item_registered);
    }

    #[test]
    fn a_malformed_value_is_absent_and_the_others_are_read() {
        let file = r#"{
  "ledger" : {
    "consecutiveAuthFailures" : "one",
    "lastAuthFailureAt" : "yesterday",
    "lastAuthFailureReason" : {
      "otpRejected" : {}
    },
    "profileKey" : "alice@vpn.example.com",
    "recentAuthAttempts" : [
      "2026-10-15T06:28:02Z",
      7,
      "2026-10-15T06:30:01Z"
    ]
  },
  "loginItemRegistered" : "yes"
}"#;
        let state = serde_json::from_str::<Wire>(file).unwrap().into_state();
        assert_eq!(
            state.ledger,
            Some(FailureLedger {
                account_key: "alice@vpn.example.com".into(),
                consecutive_auth_failures: 0,
                last_auth_failure_at: None,
                last_auth_failure: Some(Failure::OtpRejected),
                recent_auth_attempts: vec![at("2026-10-15T06:28:02Z"), at("2026-10-15T06:30:01Z")],
            })
        );
        assert!(!state.login_item_registered);
    }

    #[test]
    fn a_ledger_without_its_account_is_absent_and_the_rest_is_read() {
        let file = r#"{
  "ledger" : {
    "consecutiveAuthFailures" : 2
  },
  "loginItemRegistered" : true
}"#;
        let state = serde_json::from_str::<Wire>(file).unwrap().into_state();
        assert_eq!(state.ledger, None);
        assert!(state.login_item_registered);
    }

    #[test]
    fn writing_gives_back_the_saved_file_without_the_exception_routes() {
        let state = serde_json::from_str::<Wire>(SAVED_FILE)
            .unwrap()
            .into_state();
        let written: serde_json::Value = serde_json::to_value(Wire::from_state(&state)).unwrap();
        let mut original: serde_json::Value = serde_json::from_str(SAVED_FILE).unwrap();
        original
            .as_object_mut()
            .unwrap()
            .remove("appliedExceptionRoutes");
        assert_eq!(written, original);
    }

    #[test]
    fn every_failure_round_trips_through_its_name() {
        for (failure, json) in [
            (
                Failure::AuthenticationRejected,
                r#"{"authenticationRejected":{}}"#,
            ),
            (Failure::GmailRejected, r#"{"gmailAppPassword":{}}"#),
            (
                Failure::Helper {
                    code: HelperErrorCode::InternalError,
                },
                r#"{"helper":{"_0":"internalError"}}"#,
            ),
            (
                Failure::OtpNotReceived { in_spam: true },
                r#"{"otpNotReceived":{"inJunk":true}}"#,
            ),
        ] {
            assert_eq!(
                serde_json::to_string(&WireFailure::from_failure(failure)).unwrap(),
                json
            );
            assert_eq!(
                serde_json::from_str::<WireFailure>(json)
                    .unwrap()
                    .into_failure(),
                failure
            );
        }
    }

    #[test]
    fn a_helper_failure_of_an_earlier_build_reads_without_its_client() {
        let json = r#"{"helper":{"_0":"internalError","client":"openfortivpn"}}"#;
        assert_eq!(
            serde_json::from_str::<WireFailure>(json)
                .unwrap()
                .into_failure(),
            Failure::Helper {
                code: HelperErrorCode::InternalError
            }
        );
    }

    #[test]
    fn sub_second_times_are_written_in_whole_seconds() {
        let time = at("2026-11-09T04:08:26Z") + Duration::from_millis(750);
        assert_eq!(format_time(time), "2026-11-09T04:08:26Z");
    }

    #[test]
    fn a_file_without_the_update_fields_reads_them_empty() {
        let state = serde_json::from_str::<Wire>(SAVED_FILE)
            .unwrap()
            .into_state();
        assert_eq!(state.last_notified_update, None);
        assert_eq!(state.update_target, None);
        assert!(!state.connected_before_update);
    }

    #[test]
    fn the_update_fields_round_trip() {
        let version = |text| Version::parse(text).unwrap();
        let state = State {
            last_notified_update: Some(version("26.3.0")),
            update_target: Some(version("26.2.0")),
            connected_before_update: true,
            ..State::default()
        };
        let written = serde_json::to_value(Wire::from_state(&state)).unwrap();
        assert_eq!(
            written,
            serde_json::json!({
                "loginItemRegistered": false,
                "lastNotifiedUpdateVersion": "26.3.0",
                "updateTargetVersion": "26.2.0",
                "connectedBeforeUpdate": true,
            })
        );
        let read = serde_json::from_value::<Wire>(written)
            .unwrap()
            .into_state();
        assert_eq!(read, state);
    }

    #[test]
    fn a_malformed_update_value_is_absent_and_the_others_are_read() {
        let version = |text| Version::parse(text).unwrap();
        let read = |malformed: &str, value: &str| {
            let mut file = serde_json::json!({
                "ledger": {
                    "consecutiveAuthFailures": 2,
                    "profileKey": "alice@vpn.example.com"
                },
                "loginItemRegistered": true,
                "lastNotifiedUpdateVersion": "26.3.0",
                "updateTargetVersion": "26.2.0",
                "connectedBeforeUpdate": true
            });
            file[malformed] = serde_json::from_str(value).unwrap();
            serde_json::from_value::<Wire>(file).unwrap().into_state()
        };
        for value in [r#""soon""#, "7", "null", r#"{"major":26}"#] {
            let state = read("lastNotifiedUpdateVersion", value);
            assert_eq!(state.last_notified_update, None, "{value}");
            assert_eq!(state.update_target, Some(version("26.2.0")));
            assert!(state.connected_before_update);
            assert!(state.login_item_registered);
            assert_eq!(
                state.ledger.map(|ledger| ledger.consecutive_auth_failures),
                Some(2)
            );

            let state = read("updateTargetVersion", value);
            assert_eq!(state.update_target, None, "{value}");
            assert_eq!(state.last_notified_update, Some(version("26.3.0")));
            assert!(state.connected_before_update);
            assert!(state.login_item_registered);
        }
        for value in [r#""yes""#, "1", "null", "[]"] {
            let state = read("connectedBeforeUpdate", value);
            assert!(!state.connected_before_update, "{value}");
            assert_eq!(state.last_notified_update, Some(version("26.3.0")));
            assert_eq!(state.update_target, Some(version("26.2.0")));
            assert!(state.login_item_registered);
        }
    }

    #[test]
    fn an_empty_state_writes_only_the_required_keys() {
        let written = serde_json::to_value(Wire::from_state(&State::default())).unwrap();
        assert_eq!(written, serde_json::json!({"loginItemRegistered": false}));
    }
}
