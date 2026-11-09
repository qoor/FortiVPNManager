//! state.json: what the device keeps that nobody edits (SPEC F-SR-01). The format is the one earlier
//! builds wrote, so either can read the other's file: camelCase keys, times in whole seconds as ISO
//! 8601 UTC, and failure reasons the way Swift encoded its enum.

use std::collections::BTreeMap;
use std::io;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use fvm_contract::app_helper::{AdditionalRoute, HelperErrorCode};
use fvm_core::connection::{Failure, FailureLedger};
use serde::{Deserialize, Serialize};

use crate::files;

/// The state file grows only with routes; anything larger is not one.
const MAX_FILE_BYTES: u64 = 1 << 20;

#[derive(Clone, PartialEq, Debug, Default)]
pub(crate) struct State {
    pub(crate) ledger: Option<FailureLedger>,
    pub(crate) login_item_registered: bool,
    /// The routes the exception rules last wrote, by network service ID.
    pub(crate) applied_routes: BTreeMap<String, Vec<AdditionalRoute>>,
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

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Wire {
    /// Never read, but builds before 2026-11-05 require the key and would otherwise drop the file.
    #[serde(default = "schema_version")]
    schema_version: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    ledger: Option<WireLedger>,
    #[serde(default)]
    login_item_registered: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    applied_exception_routes: Option<BTreeMap<String, Vec<AdditionalRoute>>>,
}

fn schema_version() -> i64 {
    1
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct WireLedger {
    profile_key: String,
    #[serde(default)]
    consecutive_auth_failures: u32,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "optional_time"
    )]
    last_auth_failure_at: Option<SystemTime>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    last_auth_failure_reason: Option<WireFailure>,
    #[serde(default, with = "times")]
    recent_auth_attempts: Vec<SystemTime>,
}

/// Swift's enum coding: a case name holding its labelled values, or `_0` for an unlabelled one.
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
    TunnelModeRejected {},
    SessionDroppedEarly {},
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
            applied_routes: self.applied_exception_routes.unwrap_or_default(),
        }
    }

    fn from_state(state: &State) -> Self {
        Self {
            schema_version: 1,
            ledger: state.ledger.as_ref().map(|ledger| WireLedger {
                profile_key: ledger.account_key.clone(),
                consecutive_auth_failures: ledger.consecutive_auth_failures,
                last_auth_failure_at: ledger.last_auth_failure_at,
                last_auth_failure_reason: ledger.last_auth_failure.map(WireFailure::from_failure),
                recent_auth_attempts: ledger.recent_auth_attempts.clone(),
            }),
            login_item_registered: state.login_item_registered,
            applied_exception_routes: (!state.applied_routes.is_empty())
                .then(|| state.applied_routes.clone()),
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
            Self::TunnelModeRejected {} => Failure::TunnelModeRejected,
            Self::SessionDroppedEarly {} => Failure::SessionDroppedEarly,
            Self::CertificateRejectedAfterLogin {} => Failure::CertificateRejectedAfterLogin,
            Self::GmailAppPassword {} => Failure::GmailRejected,
            Self::TooManyReconnects {} => Failure::TooManyReconnects,
            Self::Network {} => Failure::Network,
            Self::Helper { code } => Failure::Helper(code),
        }
    }

    fn from_failure(failure: Failure) -> Self {
        match failure {
            Failure::AuthenticationRejected => Self::AuthenticationRejected {},
            Failure::OtpNotReceived { in_spam } => Self::OtpNotReceived { in_junk: in_spam },
            Failure::OtpRejected => Self::OtpRejected {},
            Failure::TunnelSetupFailed => Self::TunnelSetupFailed {},
            Failure::TunnelModeRejected => Self::TunnelModeRejected {},
            Failure::SessionDroppedEarly => Self::SessionDroppedEarly {},
            Failure::CertificateRejectedAfterLogin => Self::CertificateRejectedAfterLogin {},
            Failure::GmailRejected => Self::GmailAppPassword {},
            Failure::TooManyReconnects => Self::TooManyReconnects {},
            Failure::Network => Self::Network {},
            Failure::Helper(code) => Self::Helper { code },
        }
    }
}

/// `2026-10-15T06:30:21Z`: Swift's ISO 8601 date coding, whole seconds in UTC. Earlier builds read
/// nothing else.
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

    use serde::{Deserialize, Deserializer, Serializer, de};

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
        let text = String::deserialize(deserializer)?;
        super::parse_time(&text)
            .map(Some)
            .ok_or_else(|| de::Error::custom("not an ISO 8601 time"))
    }
}

mod times {
    use std::time::SystemTime;

    use serde::{Deserialize, Deserializer, Serializer, de, ser::SerializeSeq};

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
        Vec::<String>::deserialize(deserializer)?
            .iter()
            .map(|text| {
                super::parse_time(text).ok_or_else(|| de::Error::custom("not an ISO 8601 time"))
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The shape earlier builds write.
    const SWIFT_FILE: &str = r#"{
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
  "loginItemRegistered" : true,
  "schemaVersion" : 1
}"#;

    fn at(text: &str) -> SystemTime {
        parse_time(text).unwrap()
    }

    #[test]
    fn a_file_earlier_builds_wrote_reads_whole() {
        let state = serde_json::from_str::<Wire>(SWIFT_FILE)
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
        assert_eq!(
            state.applied_routes["F7929342-1853-4DEF-B9C0-9974F585CED1"][0].destination,
            "10.30.161.16"
        );
    }

    #[test]
    fn writing_keeps_the_format_earlier_builds_read() {
        let state = serde_json::from_str::<Wire>(SWIFT_FILE)
            .unwrap()
            .into_state();
        let written: serde_json::Value = serde_json::to_value(Wire::from_state(&state)).unwrap();
        let original: serde_json::Value = serde_json::from_str(SWIFT_FILE).unwrap();
        assert_eq!(written, original);
    }

    #[test]
    fn every_failure_round_trips_through_its_swift_name() {
        for (failure, json) in [
            (
                Failure::AuthenticationRejected,
                r#"{"authenticationRejected":{}}"#,
            ),
            (Failure::GmailRejected, r#"{"gmailAppPassword":{}}"#),
            (
                Failure::Helper(HelperErrorCode::ClientUnavailable),
                r#"{"helper":{"_0":"binaryUnavailable"}}"#,
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
    fn sub_second_times_are_written_in_whole_seconds() {
        let time = at("2026-11-09T04:08:26Z") + Duration::from_millis(750);
        assert_eq!(format_time(time), "2026-11-09T04:08:26Z");
    }

    #[test]
    fn an_empty_state_writes_only_the_required_keys() {
        let written = serde_json::to_value(Wire::from_state(&State::default())).unwrap();
        assert_eq!(
            written,
            serde_json::json!({"schemaVersion": 1, "loginItemRegistered": false})
        );
    }
}
