//! Safe functions over the Windows IP Helper API, with which the Helper applies the VPN's network
//! settings itself and reads them back to confirm them. The workspace's only unsafe blocks are here
//! (user decision, 2026-11-16); nothing else calls windows-sys.
//!
//! What Windows keeps, as checked on Windows 11: addresses, routes, MTUs and metrics written here
//! live only in the active store, so a restart drops them; DNS servers go to the registry and stay
//! until they are cleared.
#![allow(unsafe_code)]

use std::fmt;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};
use std::{ptr, slice};

use widestring::{U16CStr, U16CString};
use windows_sys::Win32::Foundation::{
    ERROR_FILE_NOT_FOUND, ERROR_OBJECT_ALREADY_EXISTS, NO_ERROR, WIN32_ERROR,
};
use windows_sys::Win32::NetworkManagement::IpHelper::{
    ConvertInterfaceIndexToLuid, ConvertInterfaceLuidToGuid, ConvertInterfaceLuidToIndex,
    CreateIpForwardEntry2, CreateUnicastIpAddressEntry, DNS_INTERFACE_SETTINGS,
    DNS_INTERFACE_SETTINGS_VERSION1, DNS_SETTING_IPV6, DNS_SETTING_NAMESERVER,
    DeleteIpForwardEntry2, DeleteUnicastIpAddressEntry, FreeInterfaceDnsSettings, FreeMibTable,
    GetBestRoute2, GetInterfaceDnsSettings, GetIpForwardTable2, GetIpInterfaceEntry,
    GetUnicastIpAddressTable, InitializeIpForwardEntry, InitializeIpInterfaceEntry,
    InitializeUnicastIpAddressEntry, MIB_IPFORWARD_ROW2, MIB_IPFORWARD_TABLE2, MIB_IPINTERFACE_ROW,
    MIB_UNICASTIPADDRESS_ROW, MIB_UNICASTIPADDRESS_TABLE, SetInterfaceDnsSettings,
    SetIpInterfaceEntry,
};
use windows_sys::Win32::NetworkManagement::Ndis::NET_LUID_LH;
use windows_sys::Win32::Networking::WinSock::{
    ADDRESS_FAMILY, AF_INET, AF_INET6, IN_ADDR, IN_ADDR_0, IN6_ADDR, IN6_ADDR_0,
    IpDadStateDeprecated, IpDadStateDuplicate, IpDadStatePreferred, IpDadStateTentative,
    NL_DAD_STATE, SOCKADDR_IN, SOCKADDR_IN6, SOCKADDR_INET,
};
use windows_sys::core::GUID;

/// A failed IP Helper call: which call, and the Win32 error code it returned.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Error {
    pub operation: &'static str,
    pub code: u32,
}

impl Error {
    /// The address or route to add is there already, which the caller checks rather than fails on.
    pub fn already_exists(self) -> bool {
        self.code == ERROR_OBJECT_ALREADY_EXISTS
    }
}

impl fmt::Display for Error {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "{} failed with Win32 error {}",
            self.operation, self.code
        )
    }
}

impl std::error::Error for Error {}

fn check(operation: &'static str, code: WIN32_ERROR) -> Result<(), Error> {
    if code == NO_ERROR {
        Ok(())
    } else {
        Err(Error { operation, code })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Family {
    Ipv4,
    Ipv6,
}

impl Family {
    fn of(address: IpAddr) -> Self {
        match address {
            IpAddr::V4(_) => Self::Ipv4,
            IpAddr::V6(_) => Self::Ipv6,
        }
    }

    fn raw(self) -> ADDRESS_FAMILY {
        match self {
            Self::Ipv4 => AF_INET,
            Self::Ipv6 => AF_INET6,
        }
    }

    /// Without the IPv6 flag, the DNS settings calls work on IPv4's, reading too.
    fn dns_flags(self) -> u64 {
        match self {
            Self::Ipv4 => 0,
            Self::Ipv6 => u64::from(DNS_SETTING_IPV6),
        }
    }
}

/// An interface's locally unique identifier, by which the IP Helper names it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Luid(u64);

impl Luid {
    /// The interface with this index, such as the one openconnect gives its script.
    pub fn from_index(index: u32) -> Result<Self, Error> {
        let mut luid = NET_LUID_LH { Value: 0 };
        // SAFETY: `luid` is live for the call, which writes it.
        let code = unsafe { ConvertInterfaceIndexToLuid(index, &mut luid) };
        check("ConvertInterfaceIndexToLuid", code)?;
        Ok(Self::from_raw(luid))
    }

    pub fn index(self) -> Result<u32, Error> {
        let mut index = 0;
        // SAFETY: Both values are live for the call, which reads the first and writes the second.
        let code = unsafe { ConvertInterfaceLuidToIndex(&self.raw(), &mut index) };
        check("ConvertInterfaceLuidToIndex", code)?;
        Ok(index)
    }

    /// The interface's GUID, by which the DNS settings calls name it.
    fn guid(self) -> Result<GUID, Error> {
        let mut guid = GUID::default();
        // SAFETY: Both values are live for the call, which reads the first and writes the second.
        let code = unsafe { ConvertInterfaceLuidToGuid(&self.raw(), &mut guid) };
        check("ConvertInterfaceLuidToGuid", code)?;
        Ok(guid)
    }

    fn raw(self) -> NET_LUID_LH {
        NET_LUID_LH { Value: self.0 }
    }

    fn from_raw(raw: NET_LUID_LH) -> Self {
        // SAFETY: The union's views are plain integers over the same 8 bytes, so any bytes are a
        // valid `Value`.
        Self(unsafe { raw.Value })
    }
}

/// An interface's settings for one address family.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct InterfaceSettings {
    pub mtu: u32,
    /// What Windows adds to the metric of each route through the interface; while
    /// `automatic_metric` is on, Windows sets it from the link speed.
    pub metric: u32,
    pub automatic_metric: bool,
}

pub fn interface_settings(luid: Luid, family: Family) -> Result<InterfaceSettings, Error> {
    let row = ip_interface(luid, family)?;
    Ok(InterfaceSettings {
        mtu: row.NlMtu,
        metric: row.Metric,
        automatic_metric: row.UseAutomaticMetric,
    })
}

pub fn set_mtu(luid: Luid, family: Family, mtu: u32) -> Result<(), Error> {
    let mut row = ip_interface(luid, family)?;
    row.NlMtu = mtu;
    write_ip_interface(row, family)
}

/// Sets the interface metric and turns the automatic metric off, or with `None` turns it back on.
pub fn set_metric(luid: Luid, family: Family, metric: Option<u32>) -> Result<(), Error> {
    let mut row = ip_interface(luid, family)?;
    row.UseAutomaticMetric = metric.is_none();
    if let Some(metric) = metric {
        row.Metric = metric;
    }
    write_ip_interface(row, family)
}

/// Writes `row`, just read and changed in one setting, so the call puts back the others as they
/// were rather than as a caller read them earlier.
fn write_ip_interface(mut row: MIB_IPINTERFACE_ROW, family: Family) -> Result<(), Error> {
    // Windows asks for 0 in an IPv4 row.
    if family == Family::Ipv4 {
        row.SitePrefixLength = 0;
    }
    // SAFETY: `row` is live for the call, which reads it and writes the LUID back.
    let code = unsafe { SetIpInterfaceEntry(&mut row) };
    check("SetIpInterfaceEntry", code)
}

fn ip_interface(luid: Luid, family: Family) -> Result<MIB_IPINTERFACE_ROW, Error> {
    let mut row = MIB_IPINTERFACE_ROW::default();
    // SAFETY: `row` is live for the call, which writes it. The system writes BOOLEAN members as 0
    // or 1, the only bytes a bool may hold.
    unsafe { InitializeIpInterfaceEntry(&mut row) };
    row.Family = family.raw();
    row.InterfaceLuid = luid.raw();
    // SAFETY: `row` is live for the call, which reads its family and LUID and writes the rest,
    // BOOLEAN members as 0 or 1.
    let code = unsafe { GetIpInterfaceEntry(&mut row) };
    check("GetIpInterfaceEntry", code)?;
    Ok(row)
}

/// The state of duplicate address detection, which decides whether an address can be used.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DadState {
    /// Not usable; also a state Windows did not have when this was written.
    Invalid,
    /// Still being checked.
    Tentative,
    /// Another host has it; not usable.
    Duplicate,
    /// Usable, but no longer for new connections.
    Deprecated,
    /// Usable.
    Preferred,
}

impl DadState {
    // windows-sys keeps the constants' Windows names.
    #[allow(non_upper_case_globals)]
    fn from_raw(raw: NL_DAD_STATE) -> Self {
        match raw {
            IpDadStateTentative => Self::Tentative,
            IpDadStateDuplicate => Self::Duplicate,
            IpDadStateDeprecated => Self::Deprecated,
            IpDadStatePreferred => Self::Preferred,
            _ => Self::Invalid,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct UnicastAddress {
    pub address: IpAddr,
    pub prefix_length: u8,
    pub dad_state: DadState,
}

/// Adds `address` with the on-link prefix of `prefix_length` bits. Windows forgets it at a restart,
/// and when the interface goes away.
pub fn add_address(luid: Luid, address: IpAddr, prefix_length: u8) -> Result<(), Error> {
    let mut row = unicast_row(luid, address);
    row.OnLinkPrefixLength = prefix_length;
    // Windows 10 and later then check for a duplicate while the address is already in use, rather
    // than holding it back for the second or more that the check takes.
    row.DadState = IpDadStatePreferred;
    // SAFETY: `row` is live for the call, which only reads it.
    let code = unsafe { CreateUnicastIpAddressEntry(&row) };
    check("CreateUnicastIpAddressEntry", code)
}

pub fn delete_address(luid: Luid, address: IpAddr) -> Result<(), Error> {
    let row = unicast_row(luid, address);
    // SAFETY: `row` is live for the call, which only reads it.
    let code = unsafe { DeleteUnicastIpAddressEntry(&row) };
    check("DeleteUnicastIpAddressEntry", code)
}

fn unicast_row(luid: Luid, address: IpAddr) -> MIB_UNICASTIPADDRESS_ROW {
    let mut row = MIB_UNICASTIPADDRESS_ROW::default();
    // SAFETY: `row` is live for the call, which writes it, BOOLEAN members as 0 or 1.
    unsafe { InitializeUnicastIpAddressEntry(&mut row) };
    row.InterfaceLuid = luid.raw();
    row.Address = sockaddr(address);
    row
}

/// The interface's addresses of `family`.
pub fn addresses(luid: Luid, family: Family) -> Result<Vec<UnicastAddress>, Error> {
    let mut table: *mut MIB_UNICASTIPADDRESS_TABLE = ptr::null_mut();
    // SAFETY: `table` is live for the call, which points it to a table it allocates.
    let code = unsafe { GetUnicastIpAddressTable(family.raw(), &mut table) };
    check("GetUnicastIpAddressTable", code)?;
    // SAFETY: After a success, `table` points to a count and that many rows, with BOOLEAN members of
    // 0 or 1, and stays valid until it is freed once, after the rows were read.
    let addresses = unsafe {
        let rows = slice::from_raw_parts(
            (&raw const (*table).Table).cast::<MIB_UNICASTIPADDRESS_ROW>(),
            (*table).NumEntries as usize,
        );
        let addresses = rows
            .iter()
            .filter(|row| Luid::from_raw(row.InterfaceLuid) == luid)
            .map(|row| UnicastAddress {
                address: ip(&row.Address, family),
                prefix_length: row.OnLinkPrefixLength,
                dad_state: DadState::from_raw(row.DadState),
            })
            .collect();
        FreeMibTable(table.cast());
        addresses
    };
    Ok(addresses)
}

/// A route of the active routing table. Windows forgets the routes added here at a restart.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Route {
    pub interface: Luid,
    pub destination: IpAddr,
    pub prefix_length: u8,
    /// The gateway, or the unspecified address of the destination's family for an on-link route.
    pub next_hop: IpAddr,
    /// What Windows adds to the interface's metric for this route.
    pub metric: u32,
}

pub fn add_route(route: &Route) -> Result<(), Error> {
    let row = forward_row(route);
    // SAFETY: `row` is live for the call, which only reads it.
    let code = unsafe { CreateIpForwardEntry2(&row) };
    check("CreateIpForwardEntry2", code)
}

/// Deletes the route to the same destination through the same next hop and interface, whatever its
/// metric.
pub fn delete_route(route: &Route) -> Result<(), Error> {
    let row = forward_row(route);
    // SAFETY: `row` is live for the call, which only reads it.
    let code = unsafe { DeleteIpForwardEntry2(&row) };
    check("DeleteIpForwardEntry2", code)
}

fn forward_row(route: &Route) -> MIB_IPFORWARD_ROW2 {
    let mut row = MIB_IPFORWARD_ROW2::default();
    // SAFETY: `row` is live for the call, which writes it, BOOLEAN members as 0 or 1.
    unsafe { InitializeIpForwardEntry(&mut row) };
    row.InterfaceLuid = route.interface.raw();
    row.DestinationPrefix.Prefix = sockaddr(route.destination);
    row.DestinationPrefix.PrefixLength = route.prefix_length;
    row.NextHop = sockaddr(route.next_hop);
    // Initialization leaves an invalid placeholder in the metric.
    row.Metric = route.metric;
    row
}

pub fn routes(family: Family) -> Result<Vec<Route>, Error> {
    let mut table: *mut MIB_IPFORWARD_TABLE2 = ptr::null_mut();
    // SAFETY: `table` is live for the call, which points it to a table it allocates.
    let code = unsafe { GetIpForwardTable2(family.raw(), &mut table) };
    check("GetIpForwardTable2", code)?;
    // SAFETY: After a success, `table` points to a count and that many rows, with BOOLEAN members of
    // 0 or 1, and stays valid until it is freed once, after the rows were read.
    let routes = unsafe {
        let rows = slice::from_raw_parts(
            (&raw const (*table).Table).cast::<MIB_IPFORWARD_ROW2>(),
            (*table).NumEntries as usize,
        );
        let routes = rows.iter().map(|row| route(row, family)).collect();
        FreeMibTable(table.cast());
        routes
    };
    Ok(routes)
}

/// The route Windows would send a packet to `destination` by, through any interface.
pub fn best_route(destination: IpAddr) -> Result<Route, Error> {
    let mut row = MIB_IPFORWARD_ROW2::default();
    let mut source = SOCKADDR_INET::default();
    // SAFETY: The destination, `row` and `source` are live for the call, which reads the first and
    // writes the others, BOOLEAN members as 0 or 1; the null LUID, index 0 and null source let it
    // choose among all interfaces and source addresses.
    let code = unsafe {
        GetBestRoute2(
            ptr::null(),
            0,
            ptr::null(),
            &sockaddr(destination),
            0,
            &mut row,
            &mut source,
        )
    };
    check("GetBestRoute2", code)?;
    Ok(route(&row, Family::of(destination)))
}

fn route(row: &MIB_IPFORWARD_ROW2, family: Family) -> Route {
    Route {
        interface: Luid::from_raw(row.InterfaceLuid),
        destination: ip(&row.DestinationPrefix.Prefix, family),
        prefix_length: row.DestinationPrefix.PrefixLength,
        next_hop: ip(&row.NextHop, family),
        metric: row.Metric,
    }
}

/// Replaces the interface's DNS servers of `family` with `servers`, which are of that family; none
/// clears them. Windows keeps them in the registry until they are replaced or cleared.
pub fn set_dns_servers(luid: Luid, family: Family, servers: &[IpAddr]) -> Result<(), Error> {
    let guid = luid.guid()?;
    let list: Vec<String> = servers.iter().map(ToString::to_string).collect();
    let mut list = U16CString::from_str(list.join(",")).expect("addresses have no NUL");
    let settings = DNS_INTERFACE_SETTINGS {
        Version: DNS_INTERFACE_SETTINGS_VERSION1,
        Flags: u64::from(DNS_SETTING_NAMESERVER) | family.dns_flags(),
        NameServer: list.as_mut_ptr(),
        ..Default::default()
    };
    // SAFETY: `settings` and the list it points to are live for the call, which only reads them.
    let code = unsafe { SetInterfaceDnsSettings(guid, &settings) };
    check("SetInterfaceDnsSettings", code)
}

/// The interface's own DNS servers of `family`, as `set_dns_servers` wrote them. Entries that are
/// not addresses are left out. Microsoft's documentation asks for no flags here, but the IPv6 one
/// does select IPv6's servers.
pub fn dns_servers(luid: Luid, family: Family) -> Result<Vec<IpAddr>, Error> {
    let guid = luid.guid()?;
    let mut settings = DNS_INTERFACE_SETTINGS {
        Version: DNS_INTERFACE_SETTINGS_VERSION1,
        Flags: family.dns_flags(),
        ..Default::default()
    };
    // SAFETY: `settings` is live for the call, which writes it and the strings it allocates.
    let code = unsafe { GetInterfaceDnsSettings(guid, &mut settings) };
    // The family's registry key, which the call reads, comes with the first write; until then the
    // interface has no servers of its own, as the loopback interface for IPv6.
    if code == ERROR_FILE_NOT_FOUND {
        return Ok(Vec::new());
    }
    check("GetInterfaceDnsSettings", code)?;
    // SAFETY: After a success, the server list is null or a NUL-terminated string that stays valid
    // until the settings are freed once, after it was copied.
    let list = unsafe {
        let list = if settings.NameServer.is_null() {
            String::new()
        } else {
            U16CStr::from_ptr_str(settings.NameServer).to_string_lossy()
        };
        FreeInterfaceDnsSettings(&mut settings);
        list
    };
    Ok(list
        .split([',', ' '])
        .filter_map(|server| server.parse().ok())
        .collect())
}

/// `address` as the calls take it: no port, flow or scope, and zeros past an IPv4 address.
fn sockaddr(address: IpAddr) -> SOCKADDR_INET {
    let mut raw = SOCKADDR_INET::default();
    match address {
        IpAddr::V4(address) => {
            raw.Ipv4 = SOCKADDR_IN {
                sin_family: AF_INET,
                // In network order, as the octets lie in memory.
                sin_addr: IN_ADDR {
                    S_un: IN_ADDR_0 {
                        S_addr: u32::from_ne_bytes(address.octets()),
                    },
                },
                ..Default::default()
            };
        }
        IpAddr::V6(address) => {
            raw.Ipv6 = SOCKADDR_IN6 {
                sin6_family: AF_INET6,
                sin6_addr: IN6_ADDR {
                    u: IN6_ADDR_0 {
                        Byte: address.octets(),
                    },
                },
                ..Default::default()
            };
        }
    }
    raw
}

/// The address of `family`, the one the rows were asked for, in `raw`.
fn ip(raw: &SOCKADDR_INET, family: Family) -> IpAddr {
    // SAFETY: The union's views are plain integers, so any bytes are valid for each of them.
    unsafe {
        match family {
            Family::Ipv4 => Ipv4Addr::from(raw.Ipv4.sin_addr.S_un.S_addr.to_ne_bytes()).into(),
            Family::Ipv6 => Ipv6Addr::from(raw.Ipv6.sin6_addr.u.Byte).into(),
        }
    }
}

#[cfg(test)]
mod tests {
    //! The tests that write change only the loopback interface, which no connection to the PC goes
    //! through, and routes to TEST-NET addresses (192.0.2.0/24, 2001:db8::/32), which nothing uses,
    //! through the primary interface. Each puts back what it changed, also when it fails.

    use std::sync::PoisonError;
    use std::time::Duration;

    use super::*;
    use crate::routes::ROUTE_WRITES;

    /// Runs its closure when dropped, so a failing test still puts back what it changed.
    struct Undo<F: FnMut()>(F);

    impl<F: FnMut()> Drop for Undo<F> {
        fn drop(&mut self) {
            (self.0)();
        }
    }

    fn loopback() -> Luid {
        best_route(Ipv4Addr::LOCALHOST.into()).unwrap().interface
    }

    #[test]
    fn an_error_names_the_call_and_tells_an_existing_object() {
        let exists = Error {
            operation: "CreateIpForwardEntry2",
            code: ERROR_OBJECT_ALREADY_EXISTS,
        };
        assert!(exists.already_exists());
        assert_eq!(
            exists.to_string(),
            "CreateIpForwardEntry2 failed with Win32 error 5010"
        );
        let denied = Error {
            operation: "CreateIpForwardEntry2",
            code: 5,
        };
        assert!(!denied.already_exists());
    }

    #[test]
    fn the_loopback_interface_is_read() {
        let loopback = loopback();
        assert_eq!(Luid::from_index(loopback.index().unwrap()), Ok(loopback));
        let localhost = UnicastAddress {
            address: Ipv4Addr::LOCALHOST.into(),
            prefix_length: 8,
            dad_state: DadState::Preferred,
        };
        assert!(
            addresses(loopback, Family::Ipv4)
                .unwrap()
                .contains(&localhost)
        );
        let best = best_route(Ipv4Addr::LOCALHOST.into()).unwrap();
        assert!(routes(Family::Ipv4).unwrap().contains(&best), "{best:?}");
        assert!(interface_settings(loopback, Family::Ipv6).unwrap().mtu > 0);
    }

    #[test]
    fn routes_are_added_found_and_deleted() {
        let _turn = ROUTE_WRITES.lock().unwrap_or_else(PoisonError::into_inner);
        // With no route to TEST-NET-1, the best one is the primary interface's default route.
        let default = best_route(Ipv4Addr::new(192, 0, 2, 1).into()).unwrap();
        assert_eq!(default.prefix_length, 0, "{default:?}");
        let through_router = Route {
            interface: default.interface,
            destination: Ipv4Addr::new(192, 0, 2, 94).into(),
            prefix_length: 32,
            next_hop: default.next_hop,
            metric: 5,
        };
        let on_link = Route {
            interface: default.interface,
            destination: "2001:db8:94::".parse().unwrap(),
            prefix_length: 48,
            next_hop: Ipv6Addr::UNSPECIFIED.into(),
            metric: 5,
        };
        for route in [through_router, on_link] {
            let family = Family::of(route.destination);
            let _undo = Undo(move || {
                let _ = delete_route(&route);
            });
            add_route(&route).unwrap();
            assert!(add_route(&route).unwrap_err().already_exists());
            assert!(routes(family).unwrap().contains(&route), "{route:?}");
            assert_eq!(best_route(route.destination), Ok(route));
            delete_route(&route).unwrap();
            assert!(!routes(family).unwrap().contains(&route), "{route:?}");
        }
    }

    #[test]
    fn loopback_addresses_are_added_found_and_deleted() {
        // Each address brings its on-link routes.
        let _turn = ROUTE_WRITES.lock().unwrap_or_else(PoisonError::into_inner);
        let loopback = loopback();
        let added: [(IpAddr, u8); 2] = [
            (Ipv4Addr::new(198, 51, 100, 94).into(), 24),
            ("2001:db8:5::94".parse().unwrap(), 64),
        ];
        for (address, prefix_length) in added {
            let family = Family::of(address);
            let _undo = Undo(move || {
                let _ = delete_address(loopback, address);
            });
            add_address(loopback, address, prefix_length).unwrap();
            let again = add_address(loopback, address, prefix_length).unwrap_err();
            assert!(again.already_exists(), "{again}");
            let added = UnicastAddress {
                address,
                prefix_length,
                dad_state: DadState::Preferred,
            };
            assert!(addresses(loopback, family).unwrap().contains(&added));
            delete_address(loopback, address).unwrap();
            let left = addresses(loopback, family).unwrap();
            assert!(!left.iter().any(|found| found.address == address));
        }
    }

    #[test]
    fn loopback_mtu_and_metric_are_written_apart_and_put_back() {
        let loopback = loopback();
        for family in [Family::Ipv4, Family::Ipv6] {
            let before = interface_settings(loopback, family).unwrap();
            let metric_before = (!before.automatic_metric).then_some(before.metric);
            let _undo = Undo(move || {
                let _ = set_mtu(loopback, family, before.mtu);
                let _ = set_metric(loopback, family, metric_before);
            });
            set_mtu(loopback, family, 1400).unwrap();
            let mtu_only = InterfaceSettings {
                mtu: 1400,
                ..before
            };
            assert_eq!(interface_settings(loopback, family), Ok(mtu_only));
            set_metric(loopback, family, Some(7)).unwrap();
            let both = InterfaceSettings {
                mtu: 1400,
                metric: 7,
                automatic_metric: false,
            };
            assert_eq!(interface_settings(loopback, family), Ok(both));
            set_mtu(loopback, family, before.mtu).unwrap();
            set_metric(loopback, family, metric_before).unwrap();
            assert_eq!(interface_settings(loopback, family), Ok(before));
        }
    }

    /// The registry key that holds the interface's DNS settings of `family`.
    fn dns_key(luid: Luid, family: Family) -> String {
        let GUID {
            data1,
            data2,
            data3,
            data4: d,
        } = luid.guid().unwrap();
        let service = match family {
            Family::Ipv4 => "Tcpip",
            Family::Ipv6 => "Tcpip6",
        };
        format!(
            r"HKLM\SYSTEM\CurrentControlSet\Services\{service}\Parameters\Interfaces\{{{data1:08X}-{data2:04X}-{data3:04X}-{:02X}{:02X}-{:02X}{:02X}{:02X}{:02X}{:02X}{:02X}}}",
            d[0], d[1], d[2], d[3], d[4], d[5], d[6], d[7]
        )
    }

    /// Runs reg.exe and answers its output, or `None` when it failed, as for a missing key or value.
    fn reg(args: &[&str]) -> Option<String> {
        let output = crate::command::run(
            r"C:\Windows\System32\reg.exe",
            args,
            Vec::new(),
            Duration::from_secs(10),
        )
        .ok()?;
        let succeeded = output.status.is_some_and(|status| status.success());
        succeeded.then(|| String::from_utf8_lossy(&output.stdout).into_owned())
    }

    /// The key's server list as the registry holds it: `None` without the key, `Some(None)` with
    /// the key but no list.
    fn registered_servers(key: &str) -> Option<Option<String>> {
        reg(&["query", key])?;
        let list = reg(&["query", key, "/v", "NameServer"]).map(|output| {
            let line = output.lines().find(|line| line.contains("REG_SZ"));
            let (_, list) = line.and_then(|line| line.split_once("REG_SZ")).unwrap();
            list.trim().to_owned()
        });
        Some(list)
    }

    #[test]
    fn loopback_dns_servers_are_written_cleared_and_put_back() {
        let loopback = loopback();
        let written: [(Family, Vec<IpAddr>); 2] = [
            (
                Family::Ipv4,
                vec![
                    Ipv4Addr::new(192, 0, 2, 53).into(),
                    Ipv4Addr::new(192, 0, 2, 54).into(),
                ],
            ),
            (Family::Ipv6, vec!["2001:db8::53".parse().unwrap()]),
        ];
        for (family, servers) in written {
            let key = dns_key(loopback, family);
            // Put back through reg.exe as it was, key and value, rather than as a write here would.
            let before = registered_servers(&key);
            let (undo_key, undo_before) = (key.clone(), before.clone());
            let undo = Undo(move || {
                let key = undo_key.as_str();
                match &undo_before {
                    None => reg(&["delete", key, "/f"]),
                    Some(None) => reg(&["delete", key, "/v", "NameServer", "/f"]),
                    Some(Some(list)) => reg(&[
                        "add",
                        key,
                        "/v",
                        "NameServer",
                        "/t",
                        "REG_SZ",
                        "/d",
                        list,
                        "/f",
                    ]),
                };
            });
            if before.is_none() {
                assert_eq!(dns_servers(loopback, family), Ok(Vec::new()));
            }
            set_dns_servers(loopback, family, &servers).unwrap();
            assert_eq!(dns_servers(loopback, family), Ok(servers));
            set_dns_servers(loopback, family, &[]).unwrap();
            assert_eq!(dns_servers(loopback, family), Ok(Vec::new()));
            drop(undo);
            assert_eq!(registered_servers(&key), before);
        }
    }
}
