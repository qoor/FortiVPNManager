//! The strict rules on the document and the reading of every field, in this order: the root, the
//! schema version (a newer app's file may break this version's rules), the structure (anchors,
//! aliases, merge keys, tags, complex keys) anywhere in the file, then the fields in file order,
//! then the rules across fields.

use std::ops::RangeInclusive;

use fvm_contract::app_internal::problem::{
    ConfigProblem, ConfigProblemKind, ConfigWarning, ConfigWarningKind, LineColumn, ValueKind,
};
use fvm_contract::app_internal::{TimeOfDay, Weekday};
use saphyr_parser::ScalarStyle;

use super::text::disallowed_in_value;
use super::tree::{Kind, Node, Tree, scalar_text};
use super::{
    AWAY_MINUTES, ClientRules, Connection, Ipv4Network, Loaded, MARGIN_MINUTES, MAX_ROUTE_ENTRIES,
    MIN_SLEEP_MINUTES, OTP_METHODS, Otp, PORTS, ProactiveReconnect, ReconnectAfterSleep, Routes,
    SCHEMA_VERSION, SESSION_MAX_MINUTES, Schedule, Settings, Vpn, WEEKDAYS, minutes,
    normalized_days,
};

type Pairs = [(usize, usize)];

pub(super) fn read(tree: &Tree<'_>, client: &ClientRules<'_>) -> Result<Loaded, ConfigProblem> {
    let Some(root) = tree.root().filter(|root| !is_null(root)) else {
        return Ok(Loaded {
            settings: Settings::default(),
            warnings: Vec::new(),
        });
    };
    let Kind::Mapping(pairs) = &root.kind else {
        let kind = if root.anchored {
            ConfigProblemKind::AnchorOrAlias {}
        } else {
            ConfigProblemKind::RootNotMapping {}
        };
        return Err(problem(kind, root));
    };
    let version = pairs
        .iter()
        .find(|&&(key, _)| scalar_text(tree.node(key)) == Some("schema_version"));
    if let Some(&(_, version)) = version {
        check_schema_version(tree.node(version))?;
    }
    check_structure(tree, root)?;
    let mut fields = Fields {
        tree,
        client,
        warnings: Vec::new(),
        session_location: None,
        margin_location: None,
        start_location: None,
        end_location: None,
    };
    let settings = fields.root(pairs)?;
    Ok(Loaded {
        settings,
        warnings: fields.warnings,
    })
}

fn check_schema_version(node: &Node<'_>) -> Result<(), ConfigProblem> {
    const PATH: &str = "schema_version";
    if node.anchored {
        return Err(problem(ConfigProblemKind::AnchorOrAlias {}, node));
    }
    if !matches!(node.kind, Kind::Scalar { .. }) {
        return Err(type_mismatch(PATH, ValueKind::Integer, node));
    }
    if node.tag.is_some() {
        // Reported where the structure is checked.
        return Ok(());
    }
    let Some(raw) = integer_text(node, PATH)? else {
        return Ok(());
    };
    match raw.parse::<i64>() {
        Ok(version) if version > SCHEMA_VERSION => Err(problem(
            ConfigProblemKind::NewerSchemaVersion { version },
            node,
        )),
        Ok(SCHEMA_VERSION) => Ok(()),
        _ => Err(out_of_range(
            PATH,
            raw,
            SCHEMA_VERSION..=SCHEMA_VERSION,
            node,
        )),
    }
}

/// In file order. Keys may carry a tag; keys are read by their text.
fn check_structure(tree: &Tree<'_>, root: &Node<'_>) -> Result<(), ConfigProblem> {
    let mut pending: Vec<(&Node<'_>, bool)> = vec![(root, false)];
    while let Some((node, is_key)) = pending.pop() {
        if node.anchored {
            return Err(problem(ConfigProblemKind::AnchorOrAlias {}, node));
        }
        if is_key {
            match &node.kind {
                Kind::Scalar { text, style } if *style == ScalarStyle::Plain && text == "<<" => {
                    return Err(problem(ConfigProblemKind::MergeKey {}, node));
                }
                Kind::Scalar { .. } => {}
                _ => return Err(problem(ConfigProblemKind::ComplexKey {}, node)),
            }
            continue;
        }
        if let Some(tag) = &node.tag {
            return Err(problem(
                ConfigProblemKind::ExplicitTag { tag: tag.clone() },
                node,
            ));
        }
        match &node.kind {
            Kind::Mapping(pairs) => {
                for &(key, value) in pairs.iter().rev() {
                    pending.push((tree.node(value), false));
                    pending.push((tree.node(key), true));
                }
            }
            Kind::Sequence(items) => {
                pending.extend(items.iter().rev().map(|&item| (tree.node(item), false)));
            }
            Kind::Scalar { .. } | Kind::Alias => {}
        }
    }
    Ok(())
}

/// Reads the fields of a document that passed the structure check: keys are scalars, and there
/// are no anchors, aliases or tags.
struct Fields<'t, 'a> {
    tree: &'t Tree<'a>,
    client: &'t ClientRules<'t>,
    warnings: Vec<ConfigWarning>,
    session_location: Option<LineColumn>,
    margin_location: Option<LineColumn>,
    start_location: Option<LineColumn>,
    end_location: Option<LineColumn>,
}

impl<'t, 'a> Fields<'t, 'a> {
    fn root(&mut self, pairs: &Pairs) -> Result<Settings, ConfigProblem> {
        let mut settings = Settings::default();
        for &(key, value) in pairs {
            let value = self.tree.node(value);
            match self.key(key) {
                "schema_version" => {}
                "vpn" => self.vpn(value, &mut settings.vpn)?,
                "otp" => self.otp(value, &mut settings.otp)?,
                "connection" => self.connection(value, &mut settings.connection)?,
                "routes" => self.routes(value, &mut settings.routes)?,
                _ => self.unknown_key(None, key),
            }
        }
        let connection = &settings.connection;
        let margin = connection.proactive_reconnect.margin_minutes;
        let session = connection.session_max_minutes;
        if margin >= session {
            return Err(ConfigProblem {
                kind: ConfigProblemKind::MarginNotLessThanSession { margin, session },
                location: self.margin_location.or(self.session_location),
            });
        }
        let schedule = &connection.schedule;
        if minutes(schedule.start) >= minutes(schedule.end) {
            return Err(ConfigProblem {
                kind: ConfigProblemKind::ScheduleEndNotAfterStart {
                    start: schedule.start,
                    end: schedule.end,
                },
                location: self.end_location.or(self.start_location),
            });
        }
        Ok(settings)
    }

    fn vpn(&mut self, node: &Node<'_>, vpn: &mut Vpn) -> Result<(), ConfigProblem> {
        let Some(pairs) = section(node, "vpn")? else {
            return Ok(());
        };
        for &(key, value) in pairs {
            let value = self.tree.node(value);
            match self.key(key) {
                "host" => set(&mut vpn.host, string(value, "vpn.host")?.map(str::to_owned)),
                "port" => set(&mut vpn.port, integer(value, "vpn.port", PORTS)?),
                "username" => set(
                    &mut vpn.username,
                    string(value, "vpn.username")?.map(str::to_owned),
                ),
                "realm" => set(
                    &mut vpn.realm,
                    string(value, "vpn.realm")?.map(str::to_owned),
                ),
                "trusted_certs" => vpn.trusted_certs = self.trusted_certs(value)?,
                _ => self.unknown_key(Some("vpn"), key),
            }
        }
        Ok(())
    }

    fn otp(&mut self, node: &Node<'_>, otp: &mut Otp) -> Result<(), ConfigProblem> {
        let Some(pairs) = section(node, "otp")? else {
            return Ok(());
        };
        for &(key, value) in pairs {
            let value = self.tree.node(value);
            match self.key(key) {
                "gmail_address" => set(
                    &mut otp.gmail_address,
                    string(value, "otp.gmail_address")?.map(str::to_owned),
                ),
                "method" => set(&mut otp.method, choice(value, "otp.method", &OTP_METHODS)?),
                "google_client_id" => set(
                    &mut otp.google_client_id,
                    string(value, "otp.google_client_id")?.map(str::to_owned),
                ),
                _ => self.unknown_key(Some("otp"), key),
            }
        }
        Ok(())
    }

    fn connection(
        &mut self,
        node: &Node<'_>,
        connection: &mut Connection,
    ) -> Result<(), ConfigProblem> {
        let Some(pairs) = section(node, "connection")? else {
            return Ok(());
        };
        for &(key, value) in pairs {
            let value = self.tree.node(value);
            match self.key(key) {
                "auto_connect_on_launch" => set(
                    &mut connection.auto_connect_on_launch,
                    boolean(value, "connection.auto_connect_on_launch")?,
                ),
                "session_max_minutes" => {
                    let path = "connection.session_max_minutes";
                    if let Some(minutes) = integer(value, path, SESSION_MAX_MINUTES)? {
                        connection.session_max_minutes = minutes;
                        self.session_location = Some(value.location);
                    }
                }
                "proactive_reconnect" => {
                    self.proactive_reconnect(value, &mut connection.proactive_reconnect)?;
                }
                "reconnect_after_sleep" => {
                    self.reconnect_after_sleep(value, &mut connection.reconnect_after_sleep)?;
                }
                "schedule" => self.schedule(value, &mut connection.schedule)?,
                _ => self.unknown_key(Some("connection"), key),
            }
        }
        Ok(())
    }

    fn proactive_reconnect(
        &mut self,
        node: &Node<'_>,
        proactive: &mut ProactiveReconnect,
    ) -> Result<(), ConfigProblem> {
        const PATH: &str = "connection.proactive_reconnect";
        let Some(pairs) = section(node, PATH)? else {
            return Ok(());
        };
        for &(key, value) in pairs {
            let value = self.tree.node(value);
            match self.key(key) {
                "enabled" => set(
                    &mut proactive.enabled,
                    boolean(value, "connection.proactive_reconnect.enabled")?,
                ),
                "margin_minutes" => {
                    let path = "connection.proactive_reconnect.margin_minutes";
                    if let Some(minutes) = integer(value, path, MARGIN_MINUTES)? {
                        proactive.margin_minutes = minutes;
                        self.margin_location = Some(value.location);
                    }
                }
                _ => self.unknown_key(Some(PATH), key),
            }
        }
        Ok(())
    }

    fn reconnect_after_sleep(
        &mut self,
        node: &Node<'_>,
        after_sleep: &mut ReconnectAfterSleep,
    ) -> Result<(), ConfigProblem> {
        const PATH: &str = "connection.reconnect_after_sleep";
        let Some(pairs) = section(node, PATH)? else {
            return Ok(());
        };
        for &(key, value) in pairs {
            let value = self.tree.node(value);
            match self.key(key) {
                "enabled" => set(
                    &mut after_sleep.enabled,
                    boolean(value, "connection.reconnect_after_sleep.enabled")?,
                ),
                "min_sleep_minutes" => set(
                    &mut after_sleep.min_sleep_minutes,
                    integer(
                        value,
                        "connection.reconnect_after_sleep.min_sleep_minutes",
                        MIN_SLEEP_MINUTES,
                    )?,
                ),
                _ => self.unknown_key(Some(PATH), key),
            }
        }
        Ok(())
    }

    fn schedule(&mut self, node: &Node<'_>, schedule: &mut Schedule) -> Result<(), ConfigProblem> {
        const PATH: &str = "connection.schedule";
        let Some(pairs) = section(node, PATH)? else {
            return Ok(());
        };
        for &(key, value) in pairs {
            let value = self.tree.node(value);
            match self.key(key) {
                "enabled" => set(
                    &mut schedule.enabled,
                    boolean(value, "connection.schedule.enabled")?,
                ),
                "days" => set(&mut schedule.days, self.days(value)?),
                "start" => {
                    if let Some(time) = time(value, "connection.schedule.start")? {
                        schedule.start = time;
                        self.start_location = Some(value.location);
                    }
                }
                "end" => {
                    if let Some(time) = time(value, "connection.schedule.end")? {
                        schedule.end = time;
                        self.end_location = Some(value.location);
                    }
                }
                "away_minutes" => set(
                    &mut schedule.away_minutes,
                    integer(value, "connection.schedule.away_minutes", AWAY_MINUTES)?,
                ),
                _ => self.unknown_key(Some(PATH), key),
            }
        }
        Ok(())
    }

    fn routes(&mut self, node: &Node<'_>, routes: &mut Routes) -> Result<(), ConfigProblem> {
        let Some(pairs) = section(node, "routes")? else {
            return Ok(());
        };
        for &(key, value) in pairs {
            let value = self.tree.node(value);
            match self.key(key) {
                "networks" => routes.networks = self.route_list(value, "routes.networks")?,
                "bypass_vpn" => routes.bypass_vpn = self.route_list(value, "routes.bypass_vpn")?,
                _ => self.unknown_key(Some("routes"), key),
            }
        }
        Ok(())
    }

    /// Certificate values the client does not accept, and repeats, are dropped with a warning
    /// rather than failing the file: the app manages the list and learns a value again.
    fn trusted_certs(&mut self, node: &Node<'_>) -> Result<Vec<String>, ConfigProblem> {
        let Some(items) = sequence(node, "vpn.trusted_certs")? else {
            return Ok(Vec::new());
        };
        let mut values: Vec<String> = Vec::new();
        for (index, &item) in items.iter().enumerate() {
            let item = self.tree.node(item);
            let value = scalar_text(item)
                .filter(|_| !is_null(item))
                .and_then(self.client.certificate_value);
            let kind = match value {
                Some(value) if !values.contains(&value) => {
                    values.push(value);
                    continue;
                }
                Some(_) => ConfigWarningKind::DuplicateTrustedCert {
                    index: index as u32,
                },
                None => ConfigWarningKind::InvalidTrustedCert {
                    index: index as u32,
                },
            };
            self.warn(kind, item);
        }
        Ok(values)
    }

    /// Entries are read into their canonical spelling. One that is not a usable host or network,
    /// or overlaps an earlier one, fails the file, since it would change what goes around the
    /// VPN; a repeat is dropped with a warning.
    fn route_list(
        &mut self,
        node: &Node<'_>,
        path: &str,
    ) -> Result<Vec<Ipv4Network>, ConfigProblem> {
        let Some(items) = sequence(node, path)? else {
            return Ok(Vec::new());
        };
        let mut entries: Vec<Ipv4Network> = Vec::new();
        for (index, &item) in items.iter().enumerate() {
            let item = self.tree.node(item);
            let index = index as u32;
            let raw = string(item, path)?.unwrap_or_default();
            let entry = raw.parse::<Ipv4Network>().map_err(|address_problem| {
                problem(
                    ConfigProblemKind::InvalidRouteEntry {
                        path: path.to_owned(),
                        index,
                        raw: raw.to_owned(),
                        problem: address_problem,
                    },
                    item,
                )
            })?;
            if entries.contains(&entry) {
                self.warn(
                    ConfigWarningKind::DuplicateRouteEntry {
                        path: path.to_owned(),
                        index,
                    },
                    item,
                );
                continue;
            }
            if let Some(other) = entries.iter().find(|other| other.overlaps(&entry)) {
                return Err(problem(
                    ConfigProblemKind::OverlappingRouteEntry {
                        path: path.to_owned(),
                        index,
                        other: other.to_string(),
                    },
                    item,
                ));
            }
            entries.push(entry);
        }
        if entries.len() > MAX_ROUTE_ENTRIES {
            return Err(problem(
                ConfigProblemKind::TooManyRouteEntries {
                    path: path.to_owned(),
                    count: entries.len() as u32,
                    limit: MAX_ROUTE_ENTRIES as u32,
                },
                node,
            ));
        }
        Ok(entries)
    }

    /// Repeats are dropped and the order normalized.
    fn days(&self, node: &Node<'_>) -> Result<Option<Vec<Weekday>>, ConfigProblem> {
        const PATH: &str = "connection.schedule.days";
        if is_null(node) {
            return Ok(None);
        }
        let Kind::Sequence(items) = &node.kind else {
            return Err(type_mismatch(PATH, ValueKind::Sequence, node));
        };
        let mut days = Vec::new();
        for &item in items {
            let item = self.tree.node(item);
            match choice(item, PATH, &WEEKDAYS)? {
                Some(day) => days.push(day),
                None => return Err(invalid_choice(PATH, "", &WEEKDAYS, item)),
            }
        }
        Ok(Some(normalized_days(&days)))
    }

    fn key(&self, key: usize) -> &'t str {
        scalar_text(self.tree.node(key)).expect("keys are scalars after the structure check")
    }

    fn unknown_key(&mut self, parent: Option<&str>, key: usize) {
        let name = self.key(key);
        let path = match parent {
            Some(parent) => format!("{parent}.{name}"),
            None => name.to_owned(),
        };
        self.warn(ConfigWarningKind::UnknownKey { path }, self.tree.node(key));
    }

    fn warn(&mut self, kind: ConfigWarningKind, node: &Node<'_>) {
        self.warnings.push(ConfigWarning {
            kind,
            location: Some(node.location),
        });
    }
}

/// Keeps the default for a null value.
fn set<T>(field: &mut T, value: Option<T>) {
    if let Some(value) = value {
        *field = value;
    }
}

/// A plain `~`, `null`, `Null`, `NULL` or nothing.
fn is_null(node: &Node<'_>) -> bool {
    matches!(
        &node.kind,
        Kind::Scalar { text, style: ScalarStyle::Plain }
            if node.tag.is_none() && matches!(&**text, "" | "~" | "null" | "Null" | "NULL")
    )
}

/// `None` for a null section, which keeps the defaults.
fn section<'n>(node: &'n Node<'_>, path: &str) -> Result<Option<&'n Pairs>, ConfigProblem> {
    if is_null(node) {
        return Ok(None);
    }
    match &node.kind {
        Kind::Mapping(pairs) => Ok(Some(pairs)),
        _ => Err(type_mismatch(path, ValueKind::Mapping, node)),
    }
}

/// `None` for a null list, which is empty.
fn sequence<'n>(node: &'n Node<'_>, path: &str) -> Result<Option<&'n [usize]>, ConfigProblem> {
    if is_null(node) {
        return Ok(None);
    }
    match &node.kind {
        Kind::Sequence(items) => Ok(Some(items)),
        _ => Err(type_mismatch(path, ValueKind::Sequence, node)),
    }
}

/// The text as written, in any style: a plain `0123` stays text.
fn string<'n>(node: &'n Node<'_>, path: &str) -> Result<Option<&'n str>, ConfigProblem> {
    let Some(text) = scalar_text(node) else {
        return Err(type_mismatch(path, ValueKind::String, node));
    };
    if is_null(node) {
        return Ok(None);
    }
    if let Some(character) = disallowed_in_value(text) {
        return Err(problem(
            ConfigProblemKind::InvalidCharacter {
                path: path.to_owned(),
                scalar: u32::from(character),
            },
            node,
        ));
    }
    Ok(Some(text))
}

fn integer(
    node: &Node<'_>,
    path: &str,
    range: RangeInclusive<i64>,
) -> Result<Option<i64>, ConfigProblem> {
    let Some(raw) = integer_text(node, path)? else {
        return Ok(None);
    };
    match raw.parse::<i64>() {
        Ok(value) if range.contains(&value) => Ok(Some(value)),
        _ => Err(out_of_range(path, raw, range, node)),
    }
}

/// An unquoted `0` or a decimal without a leading zero, the only spelling YAML 1.1 and 1.2 read
/// the same: `0443`, `2:30`, `0x1BB` and `1_0` are errors. A number in quotes has its own error.
fn integer_text<'n>(node: &'n Node<'_>, path: &str) -> Result<Option<&'n str>, ConfigProblem> {
    let Kind::Scalar { text, style } = &node.kind else {
        return Err(type_mismatch(path, ValueKind::Integer, node));
    };
    match style {
        ScalarStyle::Plain => {
            if is_null(node) {
                return Ok(None);
            }
            let canonical = !text.is_empty()
                && text.bytes().all(|byte| byte.is_ascii_digit())
                && (text.len() == 1 || !text.starts_with('0'));
            if !canonical {
                return Err(problem(
                    ConfigProblemKind::InvalidInteger {
                        path: path.to_owned(),
                        raw: text.to_string(),
                    },
                    node,
                ));
            }
            Ok(Some(text))
        }
        ScalarStyle::SingleQuoted | ScalarStyle::DoubleQuoted if looks_numeric(text) => {
            Err(problem(
                ConfigProblemKind::QuotedNumber {
                    path: path.to_owned(),
                    raw: text.to_string(),
                },
                node,
            ))
        }
        _ => Err(type_mismatch(path, ValueKind::Integer, node)),
    }
}

/// Only the YAML 1.2 spellings: `yes`, `on` and the like are errors. A boolean in quotes has its
/// own error.
fn boolean(node: &Node<'_>, path: &str) -> Result<Option<bool>, ConfigProblem> {
    let Kind::Scalar { text, style } = &node.kind else {
        return Err(type_mismatch(path, ValueKind::Boolean, node));
    };
    match style {
        ScalarStyle::Plain => match &**text {
            _ if is_null(node) => Ok(None),
            "true" | "True" | "TRUE" => Ok(Some(true)),
            "false" | "False" | "FALSE" => Ok(Some(false)),
            _ => Err(problem(
                ConfigProblemKind::InvalidBoolean {
                    path: path.to_owned(),
                    raw: text.to_string(),
                },
                node,
            )),
        },
        ScalarStyle::SingleQuoted | ScalarStyle::DoubleQuoted => {
            let word = trim_spaces(text).to_lowercase();
            if ["true", "false", "yes", "no", "on", "off", "y", "n"].contains(&word.as_str()) {
                Err(problem(
                    ConfigProblemKind::QuotedBoolean {
                        path: path.to_owned(),
                        raw: text.to_string(),
                    },
                    node,
                ))
            } else {
                Err(type_mismatch(path, ValueKind::Boolean, node))
            }
        }
        _ => Err(type_mismatch(path, ValueKind::Boolean, node)),
    }
}

/// One of the spellings in `choices`, quoted or not, matched exactly.
fn choice<T: Copy>(
    node: &Node<'_>,
    path: &str,
    choices: &[(&str, T)],
) -> Result<Option<T>, ConfigProblem> {
    let Some(raw) = string(node, path)? else {
        return Ok(None);
    };
    match choices.iter().find(|&&(spelling, _)| spelling == raw) {
        Some(&(_, value)) => Ok(Some(value)),
        None => Err(invalid_choice(path, raw, choices, node)),
    }
}

/// `H:mm` or `HH:mm` from 00:00 to 23:59, quoted or not: a plain `18:00` is read as text, never as
/// a YAML 1.1 base-60 number.
fn time(node: &Node<'_>, path: &str) -> Result<Option<TimeOfDay>, ConfigProblem> {
    let Some(raw) = string(node, path)? else {
        return Ok(None);
    };
    let parsed = raw.split_once(':').and_then(|(hour, minute)| {
        let digits = |text: &str| text.bytes().all(|byte| byte.is_ascii_digit());
        if !matches!(hour.len(), 1 | 2) || minute.len() != 2 || !digits(hour) || !digits(minute) {
            return None;
        }
        let (hour, minute): (u8, u8) = (hour.parse().ok()?, minute.parse().ok()?);
        (hour < 24 && minute < 60).then_some(TimeOfDay { hour, minute })
    });
    match parsed {
        Some(time) => Ok(Some(time)),
        None => Err(problem(
            ConfigProblemKind::InvalidTime {
                path: path.to_owned(),
                raw: raw.to_owned(),
            },
            node,
        )),
    }
}

/// Quoted text the user probably meant as a number, such as `"443"`, `'0x1BB'` or `"-5"`.
fn looks_numeric(text: &str) -> bool {
    let text = trim_spaces(text);
    text.starts_with(|c: char| c.is_ascii_digit() || matches!(c, '+' | '-' | '.'))
        && text.bytes().any(|byte| byte.is_ascii_digit())
}

/// Trims tabs and space separators (Unicode category Zs).
fn trim_spaces(text: &str) -> &str {
    text.trim_matches(|c: char| {
        matches!(
            c,
            '\t' | ' ' | '\u{A0}' | '\u{1680}' | '\u{2000}'
                ..='\u{200A}' | '\u{202F}' | '\u{205F}' | '\u{3000}'
        )
    })
}

fn problem(kind: ConfigProblemKind, node: &Node<'_>) -> ConfigProblem {
    ConfigProblem {
        kind,
        location: Some(node.location),
    }
}

fn type_mismatch(path: &str, expected: ValueKind, node: &Node<'_>) -> ConfigProblem {
    problem(
        ConfigProblemKind::TypeMismatch {
            path: path.to_owned(),
            expected,
        },
        node,
    )
}

fn out_of_range(
    path: &str,
    raw: &str,
    range: RangeInclusive<i64>,
    node: &Node<'_>,
) -> ConfigProblem {
    problem(
        ConfigProblemKind::OutOfRange {
            path: path.to_owned(),
            raw: raw.to_owned(),
            minimum: *range.start(),
            maximum: *range.end(),
        },
        node,
    )
}

fn invalid_choice<T>(
    path: &str,
    raw: &str,
    choices: &[(&str, T)],
    node: &Node<'_>,
) -> ConfigProblem {
    problem(
        ConfigProblemKind::InvalidChoice {
            path: path.to_owned(),
            raw: raw.to_owned(),
            allowed: choices
                .iter()
                .map(|&(spelling, _)| spelling.to_owned())
                .collect(),
        },
        node,
    )
}
