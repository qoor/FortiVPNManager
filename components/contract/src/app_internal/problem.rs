//! Problems the UI explains, shared by several sections: the settings file, files the user picks,
//! account values and IPv4 entries. Settings-file paths such as `vpn.port` are written as in the
//! file. Entry indexes start at 0.

use serde::Serialize;

use super::TimeOfDay;

/// Why the settings file could not be read or written.
#[derive(Clone, PartialEq, Eq, Debug, Serialize)]
#[serde(rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum ConfigFileError {
    /// The settings to write break a rule; nothing was written.
    InvalidSettings {
        problem: ConfigProblem,
    },
    /// The file at `path` does not pass; when it is in use, the last valid settings stay.
    UnreadableFile {
        path: String,
        problem: ConfigProblem,
    },
    NotRegularFile {
        path: String,
    },
    /// A symbolic link to nothing.
    DanglingSymlink {
        path: String,
    },
    /// The file kept changing while it was being written, so it was not.
    KeptChanging {
        path: String,
    },
    Io {
        operation: String,
        path: String,
        errno: i32,
    },
}

#[derive(Clone, PartialEq, Eq, Debug, Serialize)]
pub struct ConfigProblem {
    pub kind: ConfigProblemKind,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub location: Option<LineColumn>,
}

/// 1-based; columns count Unicode scalars.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize)]
pub struct LineColumn {
    pub line: u32,
    pub column: u32,
}

#[derive(Clone, PartialEq, Eq, Debug, Serialize)]
#[serde(rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum ConfigProblemKind {
    FileTooLarge {
        byte_count: u64,
        limit: u64,
    },
    InvalidUtf8 {},
    /// A raw character the file may not hold anywhere, as its Unicode scalar value.
    DisallowedCharacter {
        scalar: u32,
    },
    /// Any other syntax error, as the YAML parser describes it.
    Syntax {
        problem: String,
    },
    MultipleDocuments {},
    NestingTooDeep {},
    DuplicateKey {
        key: String,
    },
    RootNotMapping {},
    AnchorOrAlias {},
    MergeKey {},
    ExplicitTag {
        tag: String,
    },
    ComplexKey {},
    TypeMismatch {
        path: String,
        expected: ValueKind,
    },
    /// An unquoted number in a form other than plain decimal, such as `0443`.
    InvalidInteger {
        path: String,
        raw: String,
    },
    /// An unquoted boolean other than `true` or `false`, such as `yes`.
    InvalidBoolean {
        path: String,
        raw: String,
    },
    QuotedNumber {
        path: String,
        raw: String,
    },
    QuotedBoolean {
        path: String,
        raw: String,
    },
    /// `allowed` as written in the file.
    InvalidChoice {
        path: String,
        raw: String,
        allowed: Vec<String>,
    },
    OutOfRange {
        path: String,
        raw: String,
        minimum: i64,
        maximum: i64,
    },
    MarginNotLessThanSession {
        margin: i64,
        session: i64,
    },
    InvalidTime {
        path: String,
        raw: String,
    },
    ScheduleEndNotAfterStart {
        start: TimeOfDay,
        end: TimeOfDay,
    },
    /// A control character, line or paragraph separator, BOM or noncharacter in a string value.
    InvalidCharacter {
        path: String,
        scalar: u32,
    },
    /// Written by a newer app; the file is never overwritten.
    NewerSchemaVersion {
        version: i64,
    },
    InvalidTrustedCert {
        index: u32,
    },
    InvalidRouteEntry {
        path: String,
        index: u32,
        raw: String,
        problem: AddressProblem,
    },
    TooManyRouteEntries {
        path: String,
        count: u32,
        limit: u32,
    },
    DuplicateRouteEntry {
        path: String,
        index: u32,
    },
    /// The entry overlaps the earlier entry `other`, one containing the other.
    OverlappingRouteEntry {
        path: String,
        index: u32,
        other: String,
    },
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum ValueKind {
    Mapping,
    Sequence,
    String,
    Integer,
    Boolean,
}

/// Something the settings file reader ignored or noticed without refusing the file.
#[derive(Clone, PartialEq, Eq, Debug, Serialize)]
pub struct ConfigWarning {
    pub kind: ConfigWarningKind,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub location: Option<LineColumn>,
}

#[derive(Clone, PartialEq, Eq, Debug, Serialize)]
#[serde(rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum ConfigWarningKind {
    /// Removed by the next save.
    UnknownKey {
        path: String,
    },
    InvalidTrustedCert {
        index: u32,
    },
    DuplicateTrustedCert {
        index: u32,
    },
    DuplicateRouteEntry {
        path: String,
        index: u32,
    },
    /// A save replaces only this name; the other links keep the old contents.
    MultipleHardLinks {
        count: u32,
    },
    /// Permission bits other than 0600, fixed by the next save.
    InsecurePermissions {
        mode: u32,
    },
    /// The last save changed the permission bits from `previous_mode` to 0600.
    PermissionsCorrected {
        previous_mode: u32,
    },
}

/// Why text is not an IPv4 host (`10.30.161.16`) or network (`10.30.170.0/24`).
#[derive(Clone, PartialEq, Eq, Debug, Serialize)]
#[serde(rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum AddressProblem {
    Malformed {},
    PrefixOutOfRange {
        length: u32,
        minimum: u32,
        maximum: u32,
    },
    NonContiguousMask {},
    /// Loopback, multicast or reserved.
    Unroutable {},
    /// Bits are set after the prefix; `network` is the same network with them cleared.
    HostBitsSet {
        network: String,
    },
}

/// Why a file the user picked could not be read.
#[derive(Clone, PartialEq, Eq, Debug, Serialize)]
#[serde(rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum FileProblem {
    NotFound {},
    NotRegularFile {},
    TooLarge {
        limit_bytes: u64,
    },
    Io {
        operation: String,
        path: String,
        errno: i32,
    },
}

/// Why the VPN client would refuse a saved or typed account value.
#[derive(Clone, PartialEq, Eq, Debug, Serialize)]
#[serde(rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum AccountValueProblem {
    /// The server address is not just a host name or IPv4 address, such as with a scheme or port.
    InvalidHost {},
    TooLong {
        field: AccountField,
        max_bytes: u32,
    },
    LeadingOrTrailingSpace {
        field: AccountField,
    },
    DisallowedCharacters {
        field: AccountField,
    },
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum AccountField {
    Host,
    Username,
    Realm,
}
