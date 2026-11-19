#![allow(unsafe_code)]

use std::ffi::OsString;
use std::fmt;
use std::io;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};
use std::os::windows::ffi::OsStrExt;
use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle, RawHandle};
use std::time::Duration;
use std::{iter, mem, ptr, slice};

use widestring::{U16CStr, U16CString};
use windows_sys::Win32::Foundation::{
    ERROR_FILE_NOT_FOUND, ERROR_INVALID_PARAMETER, ERROR_NOT_FOUND, ERROR_OBJECT_ALREADY_EXISTS,
    HANDLE_FLAG_INHERIT, LocalFree, NO_ERROR, SetHandleInformation, TRUE, WAIT_OBJECT_0,
    WAIT_TIMEOUT, WIN32_ERROR,
};
use windows_sys::Win32::NetworkManagement::IpHelper::{
    ConvertInterfaceAliasToLuid, ConvertInterfaceIndexToLuid, ConvertInterfaceLuidToGuid,
    ConvertInterfaceLuidToIndex, CreateIpForwardEntry2, CreateUnicastIpAddressEntry,
    DNS_INTERFACE_SETTINGS, DNS_INTERFACE_SETTINGS_VERSION1, DNS_SETTING_IPV6,
    DNS_SETTING_NAMESERVER, DeleteIpForwardEntry2, DeleteUnicastIpAddressEntry,
    FreeInterfaceDnsSettings, FreeMibTable, GetBestRoute2, GetIfEntry2, GetInterfaceDnsSettings,
    GetIpForwardTable2, GetIpInterfaceEntry, GetUnicastIpAddressTable, IF_TYPE_ETHERNET_CSMACD,
    IF_TYPE_IEEE80211, InitializeIpForwardEntry, InitializeIpInterfaceEntry,
    InitializeUnicastIpAddressEntry, MIB_IF_ROW2, MIB_IPFORWARD_ROW2, MIB_IPFORWARD_TABLE2,
    MIB_IPINTERFACE_ROW, MIB_UNICASTIPADDRESS_ROW, MIB_UNICASTIPADDRESS_TABLE,
    SetInterfaceDnsSettings, SetIpInterfaceEntry,
};
use windows_sys::Win32::NetworkManagement::Ndis::{IfOperStatusUp, NET_LUID_LH};
use windows_sys::Win32::Networking::WinSock::{
    ADDRESS_FAMILY, AF_INET, AF_INET6, IN_ADDR, IN_ADDR_0, IN6_ADDR, IN6_ADDR_0,
    IpDadStateDeprecated, IpDadStateDuplicate, IpDadStatePreferred, IpDadStateTentative,
    NL_DAD_STATE, SOCKADDR_IN, SOCKADDR_IN6, SOCKADDR_INET,
};
use windows_sys::Win32::Security::Cryptography::{
    CRYPT_INTEGER_BLOB, CRYPTPROTECT_UI_FORBIDDEN, CryptProtectData, CryptUnprotectData,
};
use windows_sys::Win32::System::LibraryLoader::{
    LOAD_LIBRARY_SEARCH_APPLICATION_DIR, LoadLibraryExW,
};
use windows_sys::Win32::System::Registry::{
    HKEY_LOCAL_MACHINE, KEY_QUERY_VALUE, KEY_SET_VALUE, REG_MULTI_SZ, RegCloseKey, RegOpenKeyExW,
    RegQueryValueExW, RegSetValueExW,
};
use windows_sys::Win32::System::Threading::{
    CREATE_NO_WINDOW, CREATE_UNICODE_ENVIRONMENT, CreateProcessW, DeleteProcThreadAttributeList,
    EXTENDED_STARTUPINFO_PRESENT, GetCurrentProcess, GetExitCodeProcess, INFINITE,
    InitializeProcThreadAttributeList, LPPROC_THREAD_ATTRIBUTE_LIST,
    PROC_THREAD_ATTRIBUTE_HANDLE_LIST, PROCESS_INFORMATION, STARTF_USESTDHANDLES, STARTUPINFOEXW,
    TerminateProcess, UpdateProcThreadAttribute, WaitForSingleObject,
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

    pub fn not_found(self) -> bool {
        self.code == ERROR_NOT_FOUND || self.code == ERROR_FILE_NOT_FOUND
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
    pub fn of(address: IpAddr) -> Self {
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
    pub fn from_index(index: u32) -> Result<Self, Error> {
        let mut luid = NET_LUID_LH { Value: 0 };
        // SAFETY: `luid` is live for the call, which writes it.
        let code = unsafe { ConvertInterfaceIndexToLuid(index, &mut luid) };
        check("ConvertInterfaceIndexToLuid", code)?;
        Ok(Self::from_raw(luid))
    }

    pub fn from_alias(alias: &str) -> Result<Self, Error> {
        let alias = U16CString::from_str(alias).expect("an interface alias has no NUL");
        let mut luid = NET_LUID_LH { Value: 0 };
        // SAFETY: `alias` is NUL-terminated and `luid` is live for the call, which reads the first
        // and writes the second.
        let code = unsafe { ConvertInterfaceAliasToLuid(alias.as_ptr(), &mut luid) };
        check("ConvertInterfaceAliasToLuid", code)?;
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

/// What kind of link an interface is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Medium {
    Ethernet,
    Wifi,
    Other,
}

/// An interface as Windows describes it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Interface {
    /// The name Windows shows for the adapter, such as `Ethernet` or `Wi-Fi`.
    pub alias: String,
    /// The interface GUID as Windows writes it: in braces, in upper case.
    pub guid: String,
    pub medium: Medium,
    /// A hardware adapter, unlike a virtual one such as a VPN client's or Hyper-V's, which may
    /// report Ethernet too.
    pub hardware: bool,
    /// Able to pass packets.
    pub up: bool,
}

pub fn interface(luid: Luid) -> Result<Interface, Error> {
    let mut row = MIB_IF_ROW2 {
        InterfaceLuid: luid.raw(),
        ..Default::default()
    };
    // SAFETY: `row` is live for the call, which reads its LUID and writes the rest. Every member
    // is a plain integer, array of them or GUID, so any bytes the system writes are valid.
    let code = unsafe { GetIfEntry2(&mut row) };
    check("GetIfEntry2", code)?;
    Ok(Interface {
        alias: U16CStr::from_slice_truncate(&row.Alias)
            .map(U16CStr::to_string_lossy)
            .unwrap_or_default(),
        guid: guid_text(row.InterfaceGuid),
        medium: match row.Type {
            IF_TYPE_ETHERNET_CSMACD => Medium::Ethernet,
            IF_TYPE_IEEE80211 => Medium::Wifi,
            _ => Medium::Other,
        },
        // HardwareInterface is the lowest bit of the flags.
        hardware: row.InterfaceAndOperStatusFlags._bitfield & 1 != 0,
        up: row.OperStatus == IfOperStatusUp,
    })
}

/// `{XXXXXXXX-XXXX-XXXX-XXXX-XXXXXXXXXXXX}`.
fn guid_text(guid: GUID) -> String {
    let GUID {
        data1,
        data2,
        data3,
        data4: d,
    } = guid;
    format!(
        "{{{data1:08X}-{data2:04X}-{data3:04X}-{:02X}{:02X}-{:02X}{:02X}{:02X}{:02X}{:02X}{:02X}}}",
        d[0], d[1], d[2], d[3], d[4], d[5], d[6], d[7]
    )
}

fn guid_from_text(text: &str) -> Result<GUID, Error> {
    let invalid = Error {
        operation: "reading a GUID",
        code: ERROR_INVALID_PARAMETER,
    };
    let digits: String = text
        .strip_prefix('{')
        .and_then(|rest| rest.strip_suffix('}'))
        .ok_or(invalid)?
        .chars()
        .filter(|&c| c != '-')
        .collect();
    if text.len() != 38 || digits.len() != 32 {
        return Err(invalid);
    }
    u128::from_str_radix(&digits, 16)
        .map(GUID::from_u128)
        .map_err(|_| invalid)
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
    pub interface: Luid,
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

/// The addresses of `family` on every interface.
pub fn addresses(family: Family) -> Result<Vec<UnicastAddress>, Error> {
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
            .map(|row| UnicastAddress {
                interface: Luid::from_raw(row.InterfaceLuid),
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

/// The route Windows would send a packet to `destination` by, through any interface, and the source
/// address it would send it from.
pub fn best_route(destination: IpAddr) -> Result<(Route, IpAddr), Error> {
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
    let family = Family::of(destination);
    Ok((route(&row, family), ip(&source, family)))
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
pub fn set_dns_servers(interface: &str, family: Family, servers: &[IpAddr]) -> Result<(), Error> {
    let guid = guid_from_text(interface)?;
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

pub fn clear_wins_servers(interface: &str) -> Result<(), Error> {
    let path =
        format!(r"SYSTEM\CurrentControlSet\Services\NetBT\Parameters\Interfaces\Tcpip_{interface}");
    let path = U16CString::from_str(path).expect("the path has no NUL");
    let mut key = ptr::null_mut();
    // SAFETY: `path` is a NUL-terminated string and `key` is live for the call, which reads the
    // first and writes the second.
    let code = unsafe {
        RegOpenKeyExW(
            HKEY_LOCAL_MACHINE,
            path.as_ptr(),
            0,
            KEY_QUERY_VALUE | KEY_SET_VALUE,
            &mut key,
        )
    };
    check("RegOpenKeyExW", code)?;
    let name = U16CString::from_str("NameServerList").expect("the name has no NUL");
    // SAFETY: `key` is open, and `name` is live for the call, which only reads it; the null
    // pointers ask for nothing but whether the value exists.
    let code = unsafe {
        RegQueryValueExW(
            key,
            name.as_ptr(),
            ptr::null(),
            ptr::null_mut(),
            ptr::null_mut(),
            ptr::null_mut(),
        )
    };
    let cleared = check("RegQueryValueExW", code).and_then(|()| {
        let empty = [0u16; 2];
        // SAFETY: `key` is open, and `name` and `empty` are live for the call, which only reads
        // them; `empty` is an empty REG_MULTI_SZ list, two NULs, and its size is given in bytes.
        let code = unsafe {
            RegSetValueExW(
                key,
                name.as_ptr(),
                0,
                REG_MULTI_SZ,
                empty.as_ptr().cast(),
                size_of_val(&empty) as u32,
            )
        };
        check("RegSetValueExW", code)
    });
    // SAFETY: `key` is open and closed once, after its last use.
    unsafe { RegCloseKey(key) };
    cleared
}

pub fn protect(data: &[u8]) -> Result<Vec<u8>, Error> {
    let input = blob(data);
    let mut output = CRYPT_INTEGER_BLOB::default();
    // SAFETY: `input` points to `data`, which the call only reads, and `output` is live for it; the
    // other pointers may be null.
    let done = unsafe {
        CryptProtectData(
            &input,
            ptr::null(),
            ptr::null(),
            ptr::null(),
            ptr::null(),
            CRYPTPROTECT_UI_FORBIDDEN,
            &mut output,
        )
    };
    if done == 0 {
        return Err(last_error("CryptProtectData"));
    }
    Ok(take(output))
}

pub fn unprotect(data: &[u8]) -> Result<Vec<u8>, Error> {
    let input = blob(data);
    let mut output = CRYPT_INTEGER_BLOB::default();
    // SAFETY: As in `protect`; the description pointer may be null too.
    let done = unsafe {
        CryptUnprotectData(
            &input,
            ptr::null_mut(),
            ptr::null(),
            ptr::null(),
            ptr::null(),
            CRYPTPROTECT_UI_FORBIDDEN,
            &mut output,
        )
    };
    if done == 0 {
        return Err(last_error("CryptUnprotectData"));
    }
    Ok(take(output))
}

fn blob(data: &[u8]) -> CRYPT_INTEGER_BLOB {
    CRYPT_INTEGER_BLOB {
        cbData: u32::try_from(data.len()).expect("a secret is far smaller than 4 GiB"),
        pbData: data.as_ptr().cast_mut(),
    }
}

fn take(output: CRYPT_INTEGER_BLOB) -> Vec<u8> {
    let length = output.cbData as usize;
    // SAFETY: DPAPI's buffer holds `cbData` bytes and is this function's alone.
    let bytes = unsafe { slice::from_raw_parts_mut(output.pbData, length) };
    let copy = bytes.to_vec();
    for byte in bytes.iter_mut() {
        // SAFETY: `byte` is a valid byte of the buffer.
        unsafe { ptr::write_volatile(byte, 0) };
    }
    // SAFETY: DPAPI allocated the buffer with LocalAlloc, and it is freed once, after its last use.
    unsafe { LocalFree(output.pbData.cast()) };
    copy
}

pub struct Process(OwnedHandle);

impl Process {
    pub fn kill(&self) -> Result<(), Error> {
        // SAFETY: The handle is the process's own and open.
        if unsafe { TerminateProcess(self.0.as_raw_handle(), 1) } == 0 {
            return Err(last_error("TerminateProcess"));
        }
        Ok(())
    }

    pub fn ended_within(&self, limit: Duration) -> Result<bool, Error> {
        let milliseconds =
            u32::try_from(limit.as_millis()).expect("a wait limit is shorter than 49 days");
        // SAFETY: As in `kill`.
        match unsafe { WaitForSingleObject(self.0.as_raw_handle(), milliseconds) } {
            WAIT_OBJECT_0 => Ok(true),
            WAIT_TIMEOUT => Ok(false),
            _ => Err(last_error("WaitForSingleObject")),
        }
    }

    pub fn wait(&self) -> Result<u32, Error> {
        // SAFETY: As in `kill`.
        if unsafe { WaitForSingleObject(self.0.as_raw_handle(), INFINITE) } != WAIT_OBJECT_0 {
            return Err(last_error("WaitForSingleObject"));
        }
        let mut code = 0;
        // SAFETY: As in `kill`; `code` is live for the call.
        if unsafe { GetExitCodeProcess(self.0.as_raw_handle(), &mut code) } == 0 {
            return Err(last_error("GetExitCodeProcess"));
        }
        Ok(code)
    }
}

pub fn spawn(
    program: &str,
    args: &[&str],
    environment: &[(&str, OsString)],
    stdio: [OwnedHandle; 3],
) -> Result<Process, Error> {
    for handle in &stdio {
        // SAFETY: The handle is open and this process's own.
        let marked = unsafe {
            SetHandleInformation(
                handle.as_raw_handle(),
                HANDLE_FLAG_INHERIT,
                HANDLE_FLAG_INHERIT,
            )
        };
        if marked == 0 {
            return Err(last_error("SetHandleInformation"));
        }
    }
    let handles = stdio.each_ref().map(AsRawHandle::as_raw_handle);
    let mut size = 0;
    // SAFETY: Without a list the call only writes the size a list for one attribute needs.
    unsafe { InitializeProcThreadAttributeList(ptr::null_mut(), 1, 0, &mut size) };
    let mut list = vec![0_usize; size.div_ceil(mem::size_of::<usize>())];
    let attributes: LPPROC_THREAD_ATTRIBUTE_LIST = list.as_mut_ptr().cast();
    // SAFETY: `list` holds `size` bytes, aligned for the pointers it keeps.
    if unsafe { InitializeProcThreadAttributeList(attributes, 1, 0, &mut size) } == 0 {
        return Err(last_error("InitializeProcThreadAttributeList"));
    }
    // SAFETY: The list was initialized above for one attribute and is deleted only after this.
    let started = unsafe { start(program, args, environment, &handles, attributes) };
    // SAFETY: The list was initialized above, and no call uses it after this.
    unsafe { DeleteProcThreadAttributeList(attributes) };
    started
}

/// # Safety
///
/// `attributes` points to a list that InitializeProcThreadAttributeList initialized for one
/// attribute, and the list is not deleted before this returns.
unsafe fn start(
    program: &str,
    args: &[&str],
    environment: &[(&str, OsString)],
    handles: &[RawHandle; 3],
    attributes: LPPROC_THREAD_ATTRIBUTE_LIST,
) -> Result<Process, Error> {
    // SAFETY: The caller passes a list initialized for one attribute, and `handles` outlives its
    // use by CreateProcessW.
    let listed = unsafe {
        UpdateProcThreadAttribute(
            attributes,
            0,
            PROC_THREAD_ATTRIBUTE_HANDLE_LIST as usize,
            handles.as_ptr().cast(),
            mem::size_of_val(handles),
            ptr::null_mut(),
            ptr::null(),
        )
    };
    if listed == 0 {
        return Err(last_error("UpdateProcThreadAttribute"));
    }
    let mut startup = STARTUPINFOEXW::default();
    startup.StartupInfo.cb = mem::size_of::<STARTUPINFOEXW>() as u32;
    startup.StartupInfo.dwFlags = STARTF_USESTDHANDLES;
    [
        startup.StartupInfo.hStdInput,
        startup.StartupInfo.hStdOutput,
        startup.StartupInfo.hStdError,
    ] = *handles;
    startup.lpAttributeList = attributes;
    let application: Vec<u16> = program.encode_utf16().chain([0]).collect();
    let mut command_line = command_line(program, args);
    let environment = environment_block(environment);
    let mut information = PROCESS_INFORMATION::default();
    // SAFETY: The application name and command line are NUL-terminated and the command line
    // writable, the environment is a UTF-16 block ending in two NULs, and `startup` and
    // `information` are live for the call.
    let created = unsafe {
        CreateProcessW(
            application.as_ptr(),
            command_line.as_mut_ptr(),
            ptr::null(),
            ptr::null(),
            TRUE,
            EXTENDED_STARTUPINFO_PRESENT | CREATE_NO_WINDOW | CREATE_UNICODE_ENVIRONMENT,
            environment.as_ptr().cast(),
            ptr::null(),
            ptr::from_ref(&startup).cast(),
            &mut information,
        )
    };
    if created == 0 {
        return Err(last_error("CreateProcessW"));
    }
    // SAFETY: CreateProcessW opened both handles for this process, and only these close them.
    let (process, _thread) = unsafe {
        (
            OwnedHandle::from_raw_handle(information.hProcess),
            OwnedHandle::from_raw_handle(information.hThread),
        )
    };
    Ok(Process(process))
}

fn command_line(program: &str, args: &[&str]) -> Vec<u16> {
    let mut line = String::new();
    for (index, argument) in iter::once(program).chain(args.iter().copied()).enumerate() {
        if index > 0 {
            line.push(' ');
        }
        let quoted = argument.is_empty() || argument.contains([' ', '\t']);
        if quoted {
            line.push('"');
        }
        let mut backslashes = 0;
        for character in argument.chars() {
            if character == '\\' {
                backslashes += 1;
            } else {
                if character == '"' {
                    line.extend(iter::repeat_n('\\', backslashes + 1));
                }
                backslashes = 0;
            }
            line.push(character);
        }
        if quoted {
            line.extend(iter::repeat_n('\\', backslashes));
            line.push('"');
        }
    }
    line.encode_utf16().chain([0]).collect()
}

fn environment_block(environment: &[(&str, OsString)]) -> Vec<u16> {
    let mut sorted: Vec<_> = environment.iter().collect();
    sorted.sort_by_key(|(name, _)| name.to_uppercase());
    let mut block = Vec::new();
    for (name, value) in sorted {
        block.extend(name.encode_utf16());
        block.push(u16::from(b'='));
        block.extend(value.encode_wide());
        block.push(0);
    }
    if environment.is_empty() {
        block.push(0);
    }
    block.push(0);
    block
}

pub fn terminate_this_process(exit_code: u32) -> ! {
    // SAFETY: The call takes no pointers, and the pseudo handle stands for this process.
    unsafe { TerminateProcess(GetCurrentProcess(), exit_code) };
    unreachable!("TerminateProcess does not return when a process terminates itself")
}

pub fn keep_library_loaded(name: &str) -> Result<(), Error> {
    let name = U16CString::from_str(name).expect("a library name has no NUL");
    // SAFETY: `name` is NUL-terminated and live for the call, which only reads it; the reserved
    // file handle is null.
    let module = unsafe {
        LoadLibraryExW(
            name.as_ptr(),
            ptr::null_mut(),
            LOAD_LIBRARY_SEARCH_APPLICATION_DIR,
        )
    };
    if module.is_null() {
        return Err(last_error("LoadLibraryExW"));
    }
    Ok(())
}

fn last_error(operation: &'static str) -> Error {
    let code = io::Error::last_os_error()
        .raw_os_error()
        .expect("a failed call leaves an error code");
    Error {
        operation,
        code: code as u32,
    }
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

    use std::env;
    use std::fs::File;
    use std::io::{Read, Write};
    use std::process::Command;
    use std::sync::{Mutex, PoisonError, mpsc};
    use std::thread;
    use std::time::{Duration, Instant};

    use super::*;

    static ROUTE_WRITES: Mutex<()> = Mutex::new(());

    struct Undo<F: FnMut()>(F);

    impl<F: FnMut()> Drop for Undo<F> {
        fn drop(&mut self) {
            (self.0)();
        }
    }

    fn loopback() -> Luid {
        best_route(Ipv4Addr::LOCALHOST.into()).unwrap().0.interface
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
    fn protected_data_is_unprotected_to_the_same_bytes() {
        let secret = br#"{"cookie":"SVPNCOOKIE=Ab12"}"#;
        let protected = protect(secret).unwrap();
        assert!(
            !protected
                .windows(secret.len())
                .any(|window| window == secret),
            "the protected bytes hold the secret"
        );
        assert_eq!(unprotect(&protected).unwrap(), secret);
        let mut damaged = protected;
        let last = damaged.len() - 1;
        damaged[last] ^= 0xff;
        assert_eq!(
            unprotect(&damaged).unwrap_err().operation,
            "CryptUnprotectData"
        );
    }

    #[test]
    fn the_loopback_interface_is_read() {
        let loopback = loopback();
        assert_eq!(Luid::from_index(loopback.index().unwrap()), Ok(loopback));
        let interface = interface(loopback).unwrap();
        assert_eq!(Luid::from_alias(&interface.alias), Ok(loopback));
        assert_eq!(
            Luid::from_alias("no such interface").unwrap_err().operation,
            "ConvertInterfaceAliasToLuid"
        );
        assert_eq!(interface.medium, Medium::Other, "{interface:?}");
        assert!(!interface.hardware && interface.up, "{interface:?}");
        assert!(interface.guid.starts_with('{') && interface.guid.len() == 38);
        let localhost = UnicastAddress {
            interface: loopback,
            address: Ipv4Addr::LOCALHOST.into(),
            prefix_length: 8,
            dad_state: DadState::Preferred,
        };
        assert!(addresses(Family::Ipv4).unwrap().contains(&localhost));
        let (best, source) = best_route(Ipv4Addr::LOCALHOST.into()).unwrap();
        assert!(routes(Family::Ipv4).unwrap().contains(&best), "{best:?}");
        assert_eq!(source, IpAddr::from(Ipv4Addr::LOCALHOST));
        assert!(interface_settings(loopback, Family::Ipv6).unwrap().mtu > 0);
    }

    #[test]
    fn routes_are_added_found_and_deleted() {
        let _turn = ROUTE_WRITES.lock().unwrap_or_else(PoisonError::into_inner);
        // With no route to TEST-NET-1, the best one is the primary interface's default route.
        let (default, _) = best_route(Ipv4Addr::new(192, 0, 2, 1).into()).unwrap();
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
            assert_eq!(
                best_route(route.destination).map(|(best, _)| best),
                Ok(route)
            );
            delete_route(&route).unwrap();
            assert!(!routes(family).unwrap().contains(&route), "{route:?}");
        }
    }

    #[test]
    fn the_best_route_to_the_unspecified_address_is_a_default_route() {
        let (best, source) = best_route(Ipv4Addr::UNSPECIFIED.into()).unwrap();
        assert_eq!(best.prefix_length, 0, "{best:?}");
        assert!(
            addresses(Family::Ipv4)
                .unwrap()
                .iter()
                .any(|found| found.interface == best.interface && found.address == source),
            "{source}"
        );
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
                interface: loopback,
                address,
                prefix_length,
                dad_state: DadState::Preferred,
            };
            assert!(addresses(family).unwrap().contains(&added));
            delete_address(loopback, address).unwrap();
            let left = addresses(family).unwrap();
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
        let service = match family {
            Family::Ipv4 => "Tcpip",
            Family::Ipv6 => "Tcpip6",
        };
        let guid = guid_text(luid.guid().unwrap());
        format!(r"HKLM\SYSTEM\CurrentControlSet\Services\{service}\Parameters\Interfaces\{guid}")
    }

    /// Runs reg.exe and answers its output, or `None` when it failed, as for a missing key or value.
    fn reg(args: &[&str]) -> Option<String> {
        let output = Command::new(r"C:\Windows\System32\reg.exe")
            .args(args)
            .output()
            .ok()?;
        let succeeded = output.status.success();
        succeeded.then(|| String::from_utf8_lossy(&output.stdout).into_owned())
    }

    /// The key's server list as the registry holds it: `None` without the key, `Some(None)` with
    /// the key but no list.
    fn registered_servers(key: &str, value: &str, kind: &str) -> Option<Option<String>> {
        reg(&["query", key])?;
        let list = reg(&["query", key, "/v", value]).map(|output| {
            let line = output.lines().find(|line| line.contains(kind));
            let (_, list) = line.and_then(|line| line.split_once(kind)).unwrap();
            list.trim().to_owned()
        });
        Some(list)
    }

    #[test]
    fn clearing_the_wins_servers_of_an_interface_without_a_key_creates_none() {
        let guid = "{0F0E0D0C-0B0A-0908-0706-050403020100}";
        let key = format!(
            r"HKLM\SYSTEM\CurrentControlSet\Services\NetBT\Parameters\Interfaces\Tcpip_{guid}"
        );
        assert_eq!(
            registered_servers(&key, "NameServerList", "REG_MULTI_SZ"),
            None
        );
        let error = clear_wins_servers(guid).unwrap_err();
        assert!(error.not_found(), "{error}");
        assert_eq!(
            registered_servers(&key, "NameServerList", "REG_MULTI_SZ"),
            None
        );
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
            let before = registered_servers(&key, "NameServer", "REG_SZ");
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
            let guid = interface(loopback).unwrap().guid;
            set_dns_servers(&guid, family, &servers).unwrap();
            assert_eq!(dns_servers(loopback, family), Ok(servers));
            set_dns_servers(&guid, family, &[]).unwrap();
            assert_eq!(dns_servers(loopback, family), Ok(Vec::new()));
            drop(undo);
            assert_eq!(registered_servers(&key, "NameServer", "REG_SZ"), before);
        }
    }

    #[test]
    fn a_spawned_process_inherits_only_its_standard_handles() {
        let (stdin_read, mut stdin) = io::pipe().unwrap();
        let (mut stdout, stdout_write) = io::pipe().unwrap();
        let stderr = stdout_write.try_clone().unwrap();
        let (mut other, other_write) = io::pipe().unwrap();
        // SAFETY: The handle is open and this process's own.
        let marked = unsafe {
            SetHandleInformation(
                other_write.as_raw_handle(),
                HANDLE_FLAG_INHERIT,
                HANDLE_FLAG_INHERIT,
            )
        };
        assert_ne!(marked, 0);
        let process = spawn(
            r"C:\Windows\System32\sort.exe",
            &[],
            &[("SystemRoot", env::var_os("SystemRoot").unwrap())],
            [stdin_read.into(), stdout_write.into(), stderr.into()],
        )
        .unwrap();
        drop(other_write);
        let (sender, other_ended) = mpsc::channel();
        thread::spawn(move || {
            let mut rest = Vec::new();
            let _ = other.read_to_end(&mut rest);
            let _ = sender.send(rest);
        });
        let other_ended = other_ended.recv_timeout(Duration::from_secs(5));
        stdin.write_all(b"b\r\na\r\n").unwrap();
        drop(stdin);
        let mut output = String::new();
        stdout.read_to_string(&mut output).unwrap();
        assert_eq!(process.wait(), Ok(0));
        assert_eq!(output.lines().collect::<Vec<_>>(), ["a", "b"]);
        assert_eq!(
            other_ended,
            Ok(Vec::new()),
            "the child held the other pipe open"
        );
    }

    #[test]
    fn a_process_still_running_at_the_wait_limit_is_ended_by_a_kill() {
        let null = File::options().read(true).write(true).open("NUL").unwrap();
        let process = spawn(
            r"C:\Windows\System32\ping.exe",
            &["-n", "30", "127.0.0.1"],
            &[("SystemRoot", env::var_os("SystemRoot").unwrap())],
            [
                null.try_clone().unwrap().into(),
                null.try_clone().unwrap().into(),
                null.into(),
            ],
        )
        .unwrap();
        let started = Instant::now();
        assert_eq!(process.ended_within(Duration::from_millis(300)), Ok(false));
        let waited = started.elapsed();
        assert!(waited < Duration::from_secs(5), "{waited:?}");
        process.kill().unwrap();
        assert_eq!(process.ended_within(Duration::from_secs(5)), Ok(true));
        assert_eq!(process.wait(), Ok(1));
    }
}
