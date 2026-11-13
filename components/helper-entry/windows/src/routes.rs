//! The routing table, through PowerShell's route cmdlets: their output is JSON the scripts build,
//! while `route.exe` prints in the system's language. The scripts are fixed text; what changes is
//! passed on stdin as JSON, and every address in it was checked as IPv4 first.
//!
//! - The gateway host route that openconnect's route script adds is removed when openconnect ended
//!   without removing it.
//! - The bypass routes are the persistent routes of the primary interface: the hardware Ethernet or
//!   Wi-Fi adapter whose default route has the lowest metric. Virtual adapters, such as FortiClient's
//!   or Hyper-V's, report Ethernet too, so the hardware flag tells them apart. The app sees its GUID
//!   as the service ID and its name as the service name, as networksetup's service on macOS.

use std::collections::HashSet;
use std::net::Ipv4Addr;
use std::time::Duration;

use fvm_contract::app_helper::{
    AdditionalRoute, BypassRoutesReply, BypassRoutesRequest, BypassRoutesSnapshot, HelperError,
    HelperErrorCode,
};
use fvm_contract::backend::RouteRepairOutcome;
use fvm_contract::helper_internal::{LogCategory, LogLevel};
use fvm_core::bypass_rules::{Ipv4Network, MAX_ENTRIES};
use serde::Deserialize;
use serde::de::DeserializeOwned;
use serde_json::json;

use crate::Log;
use crate::command;

pub(crate) const POWERSHELL: &str = r"C:\Windows\System32\WindowsPowerShell\v1.0\powershell.exe";
/// PowerShell starts slowly, slower still while an antivirus program looks at it.
const POWERSHELL_LIMIT: Duration = Duration::from_secs(20);
/// What each route a write changes adds to its limit; the test PC took about 50 ms for one. The app
/// waits 90 s for a route call, past the 60 s of the 200 changes the runtime makes at most.
const ROUTE_CHANGE_TIME: Duration = Duration::from_millis(200);

/// Every script starts so: errors stop it, and its input and output are UTF-8, since an adapter's
/// name may be in the system's language. Without it, stdin is read in the system's code page.
const PRELUDE: &str = "$ErrorActionPreference = 'Stop'
[Console]::InputEncoding = New-Object System.Text.UTF8Encoding $false
[Console]::OutputEncoding = New-Object System.Text.UTF8Encoding $false
";

/// Removes the active host routes to `gateway` through a router that are not persistent, which is
/// how openconnect's route script adds it, then counts what is left. Input: `{"gateway": "…"}`.
const REMOVE_GATEWAY_ROUTE: &str = "try {
  $in = [Console]::In.ReadToEnd() | ConvertFrom-Json
  $prefix = $in.gateway + '/32'
  function Find-Leftover {
    $persistent = @(Get-NetRoute -AddressFamily IPv4 -DestinationPrefix $prefix -PolicyStore PersistentStore -ErrorAction SilentlyContinue)
    Get-NetRoute -AddressFamily IPv4 -DestinationPrefix $prefix -PolicyStore ActiveStore -ErrorAction SilentlyContinue | Where-Object {
      $route = $_
      $route.NextHop -ne '0.0.0.0' -and -not ($persistent | Where-Object { $_.InterfaceIndex -eq $route.InterfaceIndex -and $_.NextHop -eq $route.NextHop })
    }
  }
  $found = @(Find-Leftover)
  foreach ($route in $found) { $route | Remove-NetRoute -Confirm:$false -ErrorAction SilentlyContinue }
  $left = @(Find-Leftover)
  @{ found = $found.Count; left = $left.Count } | ConvertTo-Json -Compress
} catch { @{ error = $_.Exception.Message } | ConvertTo-Json -Compress }
";

/// Defines `Read-Primary`: the primary interface and its persistent routes but the default route,
/// or `found: false`. A missing IPv4 address shows as none, so the app hears the network has none.
const READ_PRIMARY: &str = "function Read-Primary {
  $best = $null
  foreach ($route in @(Get-NetRoute -AddressFamily IPv4 -DestinationPrefix '0.0.0.0/0' -PolicyStore ActiveStore -ErrorAction SilentlyContinue)) {
    $adapter = Get-NetAdapter -InterfaceIndex $route.InterfaceIndex -ErrorAction SilentlyContinue
    if ($null -eq $adapter -or $adapter.Status -ne 'Up' -or -not $adapter.HardwareInterface -or @(6, 71) -notcontains $adapter.InterfaceType) { continue }
    $metric = $route.RouteMetric + (Get-NetIPInterface -InterfaceIndex $route.InterfaceIndex -AddressFamily IPv4).InterfaceMetric
    if ($null -eq $best -or $metric -lt $best.metric) { $best = @{ route = $route; adapter = $adapter; metric = $metric } }
  }
  if ($null -eq $best) { return @{ found = $false } }
  $index = $best.route.InterfaceIndex
  $address = Get-NetIPAddress -InterfaceIndex $index -AddressFamily IPv4 -ErrorAction SilentlyContinue | Where-Object { -not $_.SkipAsSource } | Select-Object -First 1
  $routes = @(Get-NetRoute -AddressFamily IPv4 -InterfaceIndex $index -PolicyStore PersistentStore -ErrorAction SilentlyContinue | Where-Object { $_.DestinationPrefix -ne '0.0.0.0/0' } | ForEach-Object { @{ prefix = $_.DestinationPrefix; nextHop = $_.NextHop } })
  @{ found = $true; index = $index; guid = [string]$best.adapter.InterfaceGuid; name = $best.adapter.Name; router = $best.route.NextHop; address = $address.IPAddress; prefixLength = $address.PrefixLength; routes = $routes }
}
";

/// Answers with `Read-Primary`.
const READ_ROUTES: &str = "try {
  Read-Primary | ConvertTo-Json -Compress -Depth 4
} catch { @{ error = $_.Exception.Message } | ConvertTo-Json -Compress }
";

/// Changes the primary interface's persistent routes, and the same active routes, only while the
/// interface and its persistent routes are still what the app read: checking and changing in one
/// run leaves no time for the network to change in between, such as a new router after a DHCP
/// renewal. Otherwise it changes nothing and answers `{"stale": true}`; after a change it answers
/// with `Read-Primary`. Input: `{"expected": {"guid", "name", "router", "address",
/// "prefixLength", "routes"}, "remove": [routes], "add": [routes]}`, in `Read-Primary`'s terms,
/// with routes as `{"prefix": "a.b.c.d/n", "nextHop": "…"}`. A route to remove that is already
/// gone counts as removed. A new route goes to both lists by default; New-NetRoute refuses to name
/// the persistent one, and fails while the same route is active without being persistent (one DHCP
/// or `route add` put there), so that one is removed first.
const CHANGE_ROUTES: &str = "try {
  $in = [Console]::In.ReadToEnd() | ConvertFrom-Json
  $before = Read-Primary
  $expected = $in.expected
  $same = $before.found -and $before.guid -eq $expected.guid -and $before.name -eq $expected.name -and $before.router -eq $expected.router -and $before.address -eq $expected.address -and $before.prefixLength -eq $expected.prefixLength
  if ($same) {
    $have = foreach ($r in $before.routes) { $r.prefix + ' ' + $r.nextHop }
    $read = foreach ($r in $expected.routes) { $r.prefix + ' ' + $r.nextHop }
    $same = ((@($have) | Sort-Object) -join ',') -eq ((@($read) | Sort-Object) -join ',')
  }
  if (-not $same) {
    @{ found = $false; stale = $true } | ConvertTo-Json -Compress
  } else {
    $index = $before.index
    foreach ($r in @($in.remove)) {
      Get-NetRoute -AddressFamily IPv4 -InterfaceIndex $index -DestinationPrefix $r.prefix -NextHop $r.nextHop -PolicyStore PersistentStore -ErrorAction SilentlyContinue | Remove-NetRoute -Confirm:$false
      Get-NetRoute -AddressFamily IPv4 -InterfaceIndex $index -DestinationPrefix $r.prefix -NextHop $r.nextHop -PolicyStore ActiveStore -ErrorAction SilentlyContinue | Remove-NetRoute -Confirm:$false -ErrorAction SilentlyContinue
    }
    foreach ($r in @($in.add)) {
      Get-NetRoute -AddressFamily IPv4 -InterfaceIndex $index -DestinationPrefix $r.prefix -NextHop $r.nextHop -PolicyStore ActiveStore -ErrorAction SilentlyContinue | Remove-NetRoute -Confirm:$false
      New-NetRoute -AddressFamily IPv4 -InterfaceIndex $index -DestinationPrefix $r.prefix -NextHop $r.nextHop | Out-Null
    }
    Read-Primary | ConvertTo-Json -Compress -Depth 4
  }
} catch { @{ error = $_.Exception.Message } | ConvertTo-Json -Compress }
";

/// Removes the gateway host route openconnect's route script left, if any, and checks it is gone.
pub(crate) fn repair(gateway: Option<Ipv4Addr>, log: &Log) -> RouteRepairOutcome {
    #[derive(Deserialize)]
    struct Answer {
        found: usize,
        left: usize,
    }
    let Some(gateway) = gateway else {
        return RouteRepairOutcome::NotNeeded;
    };
    let input = json!({ "gateway": gateway.to_string() });
    match powershell::<Answer>(REMOVE_GATEWAY_ROUTE, &input, POWERSHELL_LIMIT) {
        Err(message) => {
            let message = format!("removing the gateway host route to {gateway} failed: {message}");
            log(LogLevel::Error, LogCategory::Routes, &message);
            RouteRepairOutcome::Failed
        }
        Ok(Answer { found: 0, .. }) => RouteRepairOutcome::NotNeeded,
        Ok(Answer { left: 0, .. }) => {
            let message = format!("removed the gateway host route to {gateway}");
            log(LogLevel::Notice, LogCategory::Routes, &message);
            RouteRepairOutcome::Repaired
        }
        Ok(Answer { left, .. }) => {
            let message =
                format!("{left} gateway host route(s) to {gateway} still present after removing");
            log(LogLevel::Error, LogCategory::Routes, &message);
            RouteRepairOutcome::Failed
        }
    }
}

pub(crate) fn read_bypass_routes(log: &Log) -> BypassRoutesReply {
    match primary(log) {
        Ok(snapshot) => BypassRoutesReply::Success { snapshot },
        Err(error) => BypassRoutesReply::Failure { error },
    }
}

/// Applies `request` (see `BypassRoutesRequest`) and answers with the routes read back.
pub(crate) fn write_bypass_routes(request: &BypassRoutesRequest, log: &Log) -> BypassRoutesReply {
    match write(request, log) {
        Ok(snapshot) => BypassRoutesReply::Success { snapshot },
        Err(error) => BypassRoutesReply::Failure { error },
    }
}

/// Checks the wanted routes against what the app read, then lets one PowerShell run check that the
/// interface still is as read and change it.
fn write(request: &BypassRoutesRequest, log: &Log) -> Result<BypassRoutesSnapshot, HelperError> {
    let expected = &request.expected;
    // The Helper reads only these forms, so other values come from the app itself, not from a read.
    // Refusing them also keeps every address the scripts get IPv4, so the route lists the script
    // joins with spaces and commas cannot run into each other.
    let is_ipv4 = |text: &str| text.parse::<Ipv4Addr>().is_ok();
    let as_read = is_ipv4(&expected.router)
        && is_ipv4(&expected.address)
        && expected
            .routes
            .iter()
            .all(|route| is_ipv4(&route.gateway) && prefix(route).is_some());
    let Some(prefix_length) = prefix_length(&expected.subnet_mask).filter(|_| as_read) else {
        return Err(error(
            HelperErrorCode::InvalidRequest,
            "what was read has values this helper never reads",
        ));
    };
    let existing: HashSet<&AdditionalRoute> = expected.routes.iter().collect();
    let wanted: HashSet<&AdditionalRoute> = request.routes.iter().collect();
    let allowed = request.routes.len() <= expected.routes.len() + MAX_ENTRIES
        && wanted.len() == request.routes.len()
        && request.routes.iter().all(|route| {
            existing.contains(route)
                || (route.gateway == expected.router && Ipv4Network::from_route(route).is_ok())
        });
    if !allowed {
        return Err(error(
            HelperErrorCode::InvalidRequest,
            "the route list has repeats, too many routes, or routes this helper does not write",
        ));
    }
    // The one form the script takes a route in.
    let as_input = |route: &AdditionalRoute| {
        let prefix = prefix(route).expect("read routes and checked new ones have one");
        json!({ "prefix": prefix, "nextHop": route.gateway })
    };
    let read_routes: Vec<_> = expected.routes.iter().map(as_input).collect();
    let remove: Vec<_> = expected
        .routes
        .iter()
        .filter(|route| !wanted.contains(route))
        .map(as_input)
        .collect();
    let add: Vec<_> = request
        .routes
        .iter()
        .filter(|route| !existing.contains(route))
        .map(as_input)
        .collect();
    let (removing, adding) = (remove.len(), add.len());
    let changes = u32::try_from(removing + adding).unwrap_or(u32::MAX);
    let limit = POWERSHELL_LIMIT.saturating_add(ROUTE_CHANGE_TIME.saturating_mul(changes));
    let input = json!({
        "expected": {
            "guid": expected.service_id,
            "name": expected.service_name,
            "router": expected.router,
            "address": expected.address,
            "prefixLength": prefix_length,
            "routes": read_routes,
        },
        "remove": remove,
        "add": add,
    });
    let script = [READ_PRIMARY, CHANGE_ROUTES].concat();
    let answer = powershell::<PrimaryAnswer>(&script, &input, limit).map_err(|message| {
        let line = format!(
            "removing {removing} and adding {adding} persistent route(s) on {} failed: {message}",
            expected.service_name
        );
        log(LogLevel::Error, LogCategory::Routes, &line);
        error(HelperErrorCode::InternalError, message)
    })?;
    if answer.stale {
        return Err(error(
            HelperErrorCode::WrongPhase,
            "the network or its routes changed since they were read; read them again",
        ));
    }
    let line = format!(
        "removed {removing} and added {adding} persistent route(s) on {}",
        expected.service_name
    );
    log(LogLevel::Notice, LogCategory::Routes, &line);
    read_primary(answer, log)
}

/// What `Read-Primary` answers, or the change script's `stale` answer.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct PrimaryAnswer {
    found: bool,
    #[serde(default)]
    stale: bool,
    guid: Option<String>,
    name: Option<String>,
    router: Option<String>,
    address: Option<String>,
    prefix_length: Option<u8>,
    #[serde(default)]
    routes: Vec<RouteAnswer>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RouteAnswer {
    prefix: String,
    next_hop: String,
}

/// The primary interface as the app sees it.
fn primary(log: &Log) -> Result<BypassRoutesSnapshot, HelperError> {
    let script = [READ_PRIMARY, READ_ROUTES].concat();
    let answer =
        powershell::<PrimaryAnswer>(&script, &json!({}), POWERSHELL_LIMIT).map_err(|message| {
            let line = format!("reading the primary interface failed: {message}");
            log(LogLevel::Error, LogCategory::Routes, &line);
            error(HelperErrorCode::InternalError, message)
        })?;
    read_primary(answer, log)
}

/// Checks `Read-Primary`'s answer and turns it into what the app sees.
fn read_primary(answer: PrimaryAnswer, log: &Log) -> Result<BypassRoutesSnapshot, HelperError> {
    if !answer.found {
        return Err(error(
            HelperErrorCode::WrongPhase,
            "no active network service",
        ));
    }
    let unreadable = |what: &str| {
        let message = format!("the primary interface's {what} could not be read");
        log(LogLevel::Error, LogCategory::Routes, &message);
        error(HelperErrorCode::InternalError, message)
    };
    let router = answer
        .router
        .filter(|router| router.parse::<Ipv4Addr>().is_ok())
        .ok_or_else(|| {
            error(
                HelperErrorCode::WrongPhase,
                "the active network service has no IPv4 router or address",
            )
        })?;
    let address = answer
        .address
        .filter(|address| address.parse::<Ipv4Addr>().is_ok())
        .ok_or_else(|| {
            error(
                HelperErrorCode::WrongPhase,
                "the active network service has no IPv4 router or address",
            )
        })?;
    let subnet_mask = answer
        .prefix_length
        .and_then(mask)
        .ok_or_else(|| unreadable("subnet"))?;
    let (Some(service_id), Some(service_name)) = (answer.guid, answer.name) else {
        return Err(unreadable("GUID or name"));
    };
    let mut routes = Vec::new();
    for route in answer.routes {
        let parsed = route
            .prefix
            .split_once('/')
            .and_then(|(destination, length)| {
                let destination = destination.parse::<Ipv4Addr>().ok()?;
                let mask = mask(length.parse().ok()?)?;
                route.next_hop.parse::<Ipv4Addr>().ok()?;
                Some(AdditionalRoute {
                    destination: destination.to_string(),
                    subnet_mask: mask,
                    gateway: route.next_hop.clone(),
                })
            })
            .ok_or_else(|| unreadable(&format!("route {}", route.prefix)))?;
        routes.push(parsed);
    }
    Ok(BypassRoutesSnapshot {
        service_id,
        service_name,
        router,
        address,
        subnet_mask,
        routes,
    })
}

/// A route's destination as PowerShell writes it, such as `10.0.0.0/8`, if it can be one.
fn prefix(route: &AdditionalRoute) -> Option<String> {
    let destination = route.destination.parse::<Ipv4Addr>().ok()?;
    let length = prefix_length(&route.subnet_mask)?;
    // Windows writes a destination as its network, without host bits.
    let host_bits = u32::MAX.checked_shr(u32::from(length)).unwrap_or(0);
    (u32::from(destination) & host_bits == 0).then(|| format!("{destination}/{length}"))
}

/// The prefix length of a subnet mask, such as 24 for `255.255.255.0`, if it is one.
fn prefix_length(mask: &str) -> Option<u8> {
    let bits = u32::from(mask.parse::<Ipv4Addr>().ok()?);
    let length = bits.leading_ones();
    (bits.checked_shl(length).unwrap_or(0) == 0).then(|| u8::try_from(length).expect("at most 32"))
}

/// The subnet mask of a prefix length, such as `255.255.255.0` for 24.
fn mask(prefix_length: u8) -> Option<String> {
    let length = u32::from(prefix_length);
    (length <= 32).then(|| {
        let bits = u32::MAX.checked_shl(32 - length).unwrap_or(0);
        Ipv4Addr::from(bits).to_string()
    })
}

/// Runs `script` after the prelude with `input` on stdin, for up to `limit`, and reads its one JSON
/// answer; a script's `{"error": …}` answer or any other failure is the error's text, in the
/// system's language where PowerShell wrote it.
fn powershell<T: DeserializeOwned>(
    script: &str,
    input: &serde_json::Value,
    limit: Duration,
) -> Result<T, String> {
    #[derive(Deserialize)]
    struct Failed {
        error: String,
    }
    let command = format!("{PRELUDE}{script}");
    let input = serde_json::to_vec(input).expect("JSON values serialize");
    let output = command::run(
        POWERSHELL,
        &[
            "-NoLogo",
            "-NoProfile",
            "-NonInteractive",
            "-Command",
            &command,
        ],
        input,
        limit,
    )
    .map_err(|error| format!("cannot start PowerShell: {error}"))?;
    let Some(status) = output.status else {
        return Err(format!(
            "PowerShell did not finish within {:.1}s",
            limit.as_secs_f64()
        ));
    };
    let stdout = String::from_utf8_lossy(&output.stdout);
    let answer = stdout.trim_start_matches('\u{feff}').trim();
    if let Ok(Failed { error }) = serde_json::from_str::<Failed>(answer) {
        return Err(error);
    }
    serde_json::from_str(answer).map_err(|_| {
        let stderr = String::from_utf8_lossy(&output.stderr);
        format!(
            "unexpected PowerShell answer ({status}): {} {}",
            answer.trim(),
            stderr.trim()
        )
    })
}

fn error(code: HelperErrorCode, message: impl Into<String>) -> HelperError {
    HelperError {
        code,
        message: message.into(),
    }
}

#[cfg(test)]
mod tests {
    //! These change the test PC's routing table, only for TEST-NET-1 addresses (192.0.2.0/24),
    //! which nothing uses, and put back what they changed.

    use std::sync::{Arc, Mutex, PoisonError};

    use super::*;

    /// The tests that write bypass routes change the same list, so they take turns.
    static ROUTE_WRITES: Mutex<()> = Mutex::new(());

    fn quiet() -> Log {
        Arc::new(|_, _, _| {})
    }

    fn snapshot(reply: BypassRoutesReply) -> BypassRoutesSnapshot {
        match reply {
            BypassRoutesReply::Success { snapshot } => snapshot,
            BypassRoutesReply::Failure { error } => panic!("{error:?}"),
        }
    }

    fn failure(reply: BypassRoutesReply) -> HelperError {
        match reply {
            BypassRoutesReply::Failure { error } => error,
            BypassRoutesReply::Success { snapshot } => panic!("written: {snapshot:?}"),
        }
    }

    fn host(destination: &str, gateway: &str) -> AdditionalRoute {
        AdditionalRoute {
            destination: destination.to_owned(),
            subnet_mask: "255.255.255.255".to_owned(),
            gateway: gateway.to_owned(),
        }
    }

    /// Adds a host route through the primary interface's router that is active but not persistent,
    /// as openconnect's route script, DHCP or `route add` put one there.
    fn add_active_only(destination: Ipv4Addr) {
        let add = "$in = [Console]::In.ReadToEnd() | ConvertFrom-Json
$primary = Read-Primary
New-NetRoute -AddressFamily IPv4 -DestinationPrefix $in.prefix -InterfaceIndex $primary.index -NextHop $primary.router -PolicyStore ActiveStore | Out-Null
@{ added = $true } | ConvertTo-Json -Compress
";
        let script = [READ_PRIMARY, add].concat();
        let input = json!({ "prefix": format!("{destination}/32") });
        powershell::<serde_json::Value>(&script, &input, POWERSHELL_LIMIT).unwrap();
    }

    #[test]
    fn a_gateway_host_route_left_behind_is_removed() {
        let log = quiet();
        let gateway: Ipv4Addr = "192.0.2.77".parse().unwrap();
        add_active_only(gateway);
        assert_eq!(repair(Some(gateway), &log), RouteRepairOutcome::Repaired);
        assert_eq!(repair(Some(gateway), &log), RouteRepairOutcome::NotNeeded);
        assert_eq!(repair(None, &log), RouteRepairOutcome::NotNeeded);
    }

    #[test]
    fn bypass_routes_are_written_only_as_read_and_through_the_router() {
        let log = quiet();
        let _turn = ROUTE_WRITES.lock().unwrap_or_else(PoisonError::into_inner);
        let before = snapshot(read_bypass_routes(&log));
        let added = host("192.0.2.88", &before.router);
        let mut routes = before.routes.clone();
        routes.push(added.clone());
        let after = snapshot(write_bypass_routes(
            &BypassRoutesRequest {
                expected: before.clone(),
                routes: routes.clone(),
            },
            &log,
        ));
        assert!(after.routes.contains(&added), "{after:?}");

        let stale = failure(write_bypass_routes(
            &BypassRoutesRequest {
                expected: before.clone(),
                routes: before.routes.clone(),
            },
            &log,
        ));
        assert_eq!(stale.code, HelperErrorCode::WrongPhase);
        let mut foreign = routes.clone();
        foreign.push(host("192.0.2.89", "192.0.2.1"));
        let refused = failure(write_bypass_routes(
            &BypassRoutesRequest {
                expected: after.clone(),
                routes: foreign,
            },
            &log,
        ));
        assert_eq!(refused.code, HelperErrorCode::InvalidRequest);

        let restored = snapshot(write_bypass_routes(
            &BypassRoutesRequest {
                expected: after,
                routes: before.routes.clone(),
            },
            &log,
        ));
        let left: HashSet<_> = restored.routes.iter().collect();
        assert_eq!(left, before.routes.iter().collect::<HashSet<_>>());
    }

    #[test]
    fn a_bypass_route_already_active_but_not_persistent_is_written() {
        let log = quiet();
        let _turn = ROUTE_WRITES.lock().unwrap_or_else(PoisonError::into_inner);
        let before = snapshot(read_bypass_routes(&log));
        let wanted = host("192.0.2.92", &before.router);
        add_active_only("192.0.2.92".parse().unwrap());
        let mut routes = before.routes.clone();
        routes.push(wanted.clone());
        let after = snapshot(write_bypass_routes(
            &BypassRoutesRequest {
                expected: before.clone(),
                routes,
            },
            &log,
        ));
        assert!(after.routes.contains(&wanted), "{after:?}");

        let restored = snapshot(write_bypass_routes(
            &BypassRoutesRequest {
                expected: after,
                routes: before.routes.clone(),
            },
            &log,
        ));
        let left: HashSet<_> = restored.routes.iter().collect();
        assert_eq!(left, before.routes.iter().collect::<HashSet<_>>());
    }

    #[test]
    fn a_write_for_a_network_that_changed_since_it_was_read_changes_nothing() {
        let log = quiet();
        let _turn = ROUTE_WRITES.lock().unwrap_or_else(PoisonError::into_inner);
        let before = snapshot(read_bypass_routes(&log));
        // As if the app had read another router, which the network no longer has.
        let mut read = before.clone();
        read.router = "192.0.2.1".to_owned();
        let mut routes = before.routes.clone();
        routes.push(host("192.0.2.93", "192.0.2.1"));
        let refused = failure(write_bypass_routes(
            &BypassRoutesRequest {
                expected: read,
                routes,
            },
            &log,
        ));
        assert_eq!(refused.code, HelperErrorCode::WrongPhase, "{refused:?}");
        let after = snapshot(read_bypass_routes(&log));
        let left: HashSet<_> = after.routes.iter().collect();
        assert_eq!(left, before.routes.iter().collect::<HashSet<_>>());
    }

    /// A network that is not the test PC's, as if the app had read it: a write for it that passes the
    /// checks reaches PowerShell, which answers that the network changed.
    fn made_up_read() -> BypassRoutesSnapshot {
        BypassRoutesSnapshot {
            service_id: "{00000000-0000-0000-0000-000000000000}".to_owned(),
            service_name: "Ethernet".to_owned(),
            router: "192.0.2.1".to_owned(),
            address: "192.0.2.10".to_owned(),
            subnet_mask: "255.255.255.0".to_owned(),
            routes: vec![host("192.0.2.50", "192.0.2.1")],
        }
    }

    #[test]
    fn values_never_read_are_refused_before_powershell() {
        let read = made_up_read();
        let write = |expected: BypassRoutesSnapshot| {
            failure(write_bypass_routes(
                &BypassRoutesRequest {
                    expected,
                    routes: Vec::new(),
                },
                &quiet(),
            ))
        };
        assert_eq!(write(read.clone()).code, HelperErrorCode::WrongPhase);
        type Change = fn(&mut BypassRoutesSnapshot);
        let changes: [(&str, Change); 7] = [
            ("router", |read| read.router = "192.0.2.1,".to_owned()),
            ("address", |read| read.address = "192.0.2.10 ".to_owned()),
            ("gateway", |read| {
                read.routes[0].gateway = "192.0.2.1,192.0.2.0/24 192.0.2.1".to_owned();
            }),
            ("mask", |read| read.subnet_mask = "255.0.255.0".to_owned()),
            ("destination", |read| {
                read.routes[0].destination = "192.0.2.50 ".to_owned();
            }),
            ("route mask", |read| {
                read.routes[0].subnet_mask = "255.0.255.0".to_owned();
            }),
            ("host bits", |read| {
                read.routes[0].subnet_mask = "255.255.255.0".to_owned();
            }),
        ];
        for (what, change) in changes {
            let mut expected = read.clone();
            change(&mut expected);
            let refused = write(expected);
            assert_eq!(
                refused.code,
                HelperErrorCode::InvalidRequest,
                "{what}: {refused:?}"
            );
        }
    }

    #[test]
    fn routes_this_helper_does_not_write_are_refused_before_powershell() {
        let read = made_up_read();
        let write = |routes: Vec<AdditionalRoute>| {
            failure(write_bypass_routes(
                &BypassRoutesRequest {
                    expected: read.clone(),
                    routes,
                },
                &quiet(),
            ))
        };
        let new = |third: usize| host(&format!("10.0.{third}.1"), "192.0.2.1");
        let network = |destination: &str, mask: &str| AdditionalRoute {
            destination: destination.to_owned(),
            subnet_mask: mask.to_owned(),
            gateway: "192.0.2.1".to_owned(),
        };
        // The read routes and as many new ones as a list holds pass.
        let mut most = read.routes.clone();
        most.extend((0..MAX_ENTRIES).map(new));
        assert_eq!(write(most.clone()).code, HelperErrorCode::WrongPhase);
        let mut too_many = most;
        too_many.push(new(MAX_ENTRIES));
        let refusals = [
            ("repeat", vec![new(0), new(0)]),
            ("too many", too_many),
            ("another router", vec![host("10.0.0.1", "192.0.2.2")]),
            ("prefix too short", vec![network("10.0.0.0", "254.0.0.0")]),
            ("loopback", vec![host("127.0.0.2", "192.0.2.1")]),
            ("multicast", vec![host("224.0.0.9", "192.0.2.1")]),
        ];
        for (what, routes) in refusals {
            let refused = write(routes);
            assert_eq!(
                refused.code,
                HelperErrorCode::InvalidRequest,
                "{what}: {refused:?}"
            );
        }
    }

    #[test]
    fn masks_and_prefixes_follow_each_other() {
        assert_eq!(mask(0).as_deref(), Some("0.0.0.0"));
        assert_eq!(mask(24).as_deref(), Some("255.255.255.0"));
        assert_eq!(mask(32).as_deref(), Some("255.255.255.255"));
        assert_eq!(mask(33), None);
        assert_eq!(prefix_length("0.0.0.0"), Some(0));
        assert_eq!(prefix_length("255.255.255.0"), Some(24));
        assert_eq!(prefix_length("255.255.255.255"), Some(32));
        assert_eq!(prefix_length("255.0.255.0"), None);
        assert_eq!(
            prefix(&host("192.0.2.5", "192.0.2.1")).as_deref(),
            Some("192.0.2.5/32")
        );
    }
}
