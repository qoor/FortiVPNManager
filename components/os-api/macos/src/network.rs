use std::ptr;

use core_foundation::array::CFArray;
use core_foundation::base::{CFAllocatorRef, CFType, TCFType};
use core_foundation::dictionary::CFDictionary;
use core_foundation::string::{CFString, CFStringRef};
use system_configuration::dynamic_store::SCDynamicStore;
use system_configuration::sys::network_configuration::{
    SCNetworkServiceCopy, SCNetworkServiceGetName,
};
use system_configuration::sys::preferences::SCPreferencesCreate;

pub const INTERFACE_NAME: &str = "InterfaceName";
pub const PRIMARY_SERVICE: &str = "PrimaryService";
pub const PRIMARY_INTERFACE: &str = "PrimaryInterface";
pub const SERVICE_ORDER: &str = "ServiceOrder";
pub const OVERRIDE_PRIMARY: &str = "OverridePrimary";
pub const ROUTER: &str = "Router";
pub const ADDRESSES: &str = "Addresses";
pub const SUBNET_MASKS: &str = "SubnetMasks";
pub const DNS_SERVER_ADDRESSES: &str = "ServerAddresses";
pub const DNS_SEARCH_DOMAINS: &str = "SearchDomains";
pub const DNS_SUPPLEMENTAL_MATCH_DOMAINS: &str = "SupplementalMatchDomains";

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Entity {
    Dns,
    Ipv4,
    Ipv6,
}

#[link(name = "SystemConfiguration", kind = "framework")]
unsafe extern "C" {
    static kSCDynamicStoreDomainState: CFStringRef;
    static kSCEntNetDNS: CFStringRef;
    static kSCEntNetIPv4: CFStringRef;
    static kSCEntNetIPv6: CFStringRef;
    fn SCDynamicStoreKeyCreateNetworkServiceEntity(
        allocator: CFAllocatorRef,
        domain: CFStringRef,
        service_id: CFStringRef,
        entity: CFStringRef,
    ) -> CFStringRef;
}

/// The dynamic store key of the network service `service_id`'s state for `entity`.
///
/// # Panics
///
/// Panics if the system returns no key.
#[must_use]
pub fn service_state_key(service_id: &str, entity: Entity) -> String {
    let service_id = CFString::new(service_id);
    // SAFETY: constants of the SystemConfiguration framework.
    let (domain, entity) = unsafe {
        let entity = match entity {
            Entity::Dns => kSCEntNetDNS,
            Entity::Ipv4 => kSCEntNetIPv4,
            Entity::Ipv6 => kSCEntNetIPv6,
        };
        (kSCDynamicStoreDomainState, entity)
    };
    // SAFETY: all three are valid strings; the result is a +1 reference.
    let key = unsafe {
        SCDynamicStoreKeyCreateNetworkServiceEntity(
            ptr::null(),
            domain,
            service_id.as_concrete_TypeRef(),
            entity,
        )
    };
    assert!(
        !key.is_null(),
        "SCDynamicStoreKeyCreateNetworkServiceEntity returned no key"
    );
    // SAFETY: a +1 reference to a string.
    unsafe { CFString::wrap_under_create_rule(key) }.to_string()
}

#[must_use]
pub fn dictionary(store: &SCDynamicStore, key: &str) -> Option<CFDictionary<CFString, CFType>> {
    let value = store.get(key)?.downcast_into::<CFDictionary>()?;
    // SAFETY: the same dictionary; only `find` with a string key reads it, which compares keys
    // with CFEqual whatever their type.
    Some(unsafe { CFDictionary::wrap_under_get_rule(value.as_concrete_TypeRef()) })
}

#[must_use]
pub fn strings(value: &CFType) -> Option<Vec<String>> {
    let array = value.downcast::<CFArray>()?;
    array
        .iter()
        .map(|item| {
            // SAFETY: the array holds CF objects, kept alive by it while this runs.
            let item = unsafe { CFType::wrap_under_get_rule(*item) };
            item.downcast::<CFString>().map(|text| text.to_string())
        })
        .collect()
}

#[must_use]
pub fn service_name(process_name: &str, service_id: &str) -> Option<String> {
    let process_name = CFString::new(process_name);
    // SAFETY: a valid name; the result is a +1 reference or NULL.
    let preferences = unsafe {
        SCPreferencesCreate(ptr::null(), process_name.as_concrete_TypeRef(), ptr::null())
    };
    if preferences.is_null() {
        return None;
    }
    // SAFETY: a +1 reference from SCPreferencesCreate.
    let preferences = unsafe { CFType::wrap_under_create_rule(preferences.cast()) };
    let service_id = CFString::new(service_id);
    // SAFETY: valid preferences and string; the result is a +1 reference or NULL.
    let service = unsafe {
        SCNetworkServiceCopy(
            preferences.as_CFTypeRef().cast(),
            service_id.as_concrete_TypeRef(),
        )
    };
    if service.is_null() {
        return None;
    }
    // SAFETY: a +1 reference from SCNetworkServiceCopy.
    let service = unsafe { CFType::wrap_under_create_rule(service.cast()) };
    // SAFETY: a valid service; the name, if any, belongs to it.
    let name = unsafe { SCNetworkServiceGetName(service.as_CFTypeRef().cast()) };
    if name.is_null() {
        return None;
    }
    // SAFETY: a string the service keeps while `service` lives.
    Some(unsafe { CFString::wrap_under_get_rule(name) }.to_string())
}

#[cfg(test)]
mod tests {
    use system_configuration::sys::schema_definitions::{
        kSCDynamicStorePropNetPrimaryInterface, kSCDynamicStorePropNetPrimaryService,
        kSCPropInterfaceName, kSCPropNetDNSSearchDomains, kSCPropNetDNSServerAddresses,
        kSCPropNetDNSSupplementalMatchDomains, kSCPropNetIPv4Addresses, kSCPropNetIPv4Router,
        kSCPropNetIPv4SubnetMasks, kSCPropNetIPv6Router, kSCPropNetOverridePrimary,
        kSCPropNetServiceOrder,
    };

    use super::*;

    fn text(key: CFStringRef) -> String {
        // SAFETY: a constant of the SystemConfiguration framework.
        unsafe { CFString::wrap_under_get_rule(key) }.to_string()
    }

    #[test]
    fn the_property_names_are_the_frameworks() {
        // SAFETY: constants of the SystemConfiguration framework.
        let names = unsafe {
            [
                (INTERFACE_NAME, kSCPropInterfaceName),
                (PRIMARY_SERVICE, kSCDynamicStorePropNetPrimaryService),
                (PRIMARY_INTERFACE, kSCDynamicStorePropNetPrimaryInterface),
                (SERVICE_ORDER, kSCPropNetServiceOrder),
                (OVERRIDE_PRIMARY, kSCPropNetOverridePrimary),
                (ROUTER, kSCPropNetIPv4Router),
                (ROUTER, kSCPropNetIPv6Router),
                (ADDRESSES, kSCPropNetIPv4Addresses),
                (SUBNET_MASKS, kSCPropNetIPv4SubnetMasks),
                (DNS_SERVER_ADDRESSES, kSCPropNetDNSServerAddresses),
                (DNS_SEARCH_DOMAINS, kSCPropNetDNSSearchDomains),
                (
                    DNS_SUPPLEMENTAL_MATCH_DOMAINS,
                    kSCPropNetDNSSupplementalMatchDomains,
                ),
            ]
        };
        for (ours, framework) in names {
            assert_eq!(ours, text(framework));
        }
    }

    #[test]
    fn a_service_state_key_names_the_service_and_the_entity() {
        assert_eq!(
            service_state_key("ABC-1", Entity::Dns),
            "State:/Network/Service/ABC-1/DNS"
        );
        assert_eq!(
            service_state_key("ABC-1", Entity::Ipv4),
            "State:/Network/Service/ABC-1/IPv4"
        );
        assert_eq!(
            service_state_key("ABC-1", Entity::Ipv6),
            "State:/Network/Service/ABC-1/IPv6"
        );
    }

    #[test]
    fn strings_takes_only_an_array_of_strings() {
        let words = CFArray::from_CFTypes(&[CFString::new("a"), CFString::new("b")]);
        assert_eq!(
            strings(&words.as_CFType()),
            Some(vec!["a".into(), "b".into()])
        );
        let mixed = CFArray::from_CFTypes(&[
            CFString::new("a").as_CFType(),
            core_foundation::number::CFNumber::from(1).as_CFType(),
        ]);
        assert_eq!(strings(&mixed.as_CFType()), None);
        assert_eq!(strings(&CFString::new("a").as_CFType()), None);
    }
}
