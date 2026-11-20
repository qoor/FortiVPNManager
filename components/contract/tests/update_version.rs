use fvm_contract::app_helper::{Version, installer_version};

const CURRENT: Version = Version {
    major: 26,
    minor: 1,
    patch: 0,
};

fn version(text: &str) -> Version {
    Version::parse(text).unwrap_or_else(|| panic!("{text:?} is not a version"))
}

#[test]
fn versions_order_numerically() {
    let ascending = [
        "0.0.0",
        "0.0.1",
        "0.1.0",
        "1.0.0",
        "26.1.0",
        "26.1.1",
        "26.2.0",
        "26.9.0",
        "26.10.0",
        "26.10.9",
        "26.10.10",
        "27.0.0",
        "4294967295.0.0",
    ];
    for pair in ascending.windows(2) {
        assert!(version(pair[0]) < version(pair[1]), "{pair:?}");
    }
    assert_eq!(version("26.1.0"), CURRENT);
    assert_eq!(version("26.10.3").to_string(), "26.10.3");
}

#[test]
fn invalid_versions() {
    for text in [
        "",
        "26",
        "26.1",
        "26.1.0.0",
        "26.1.",
        ".1.0",
        "26..0",
        "26.1.0 ",
        " 26.1.0",
        "26.1.0\n",
        "26.1.0-beta",
        "26.1.0+build",
        "26.1.a",
        "+26.1.0",
        "-1.0.0",
        "26.01.0",
        "026.1.0",
        "26.1.00",
        "4294967296.0.0",
        "２６.1.0",
        "v26.1.0",
    ] {
        assert_eq!(Version::parse(text), None, "{text:?}");
    }
}

#[test]
fn release_tags() {
    assert_eq!(Version::from_tag("v26.1.0"), Some(CURRENT));
    assert_eq!(Version::from_tag("v0.0.0"), Some(version("0.0.0")));
    for tag in [
        "26.1.0",
        "V26.1.0",
        "vv26.1.0",
        "v 26.1.0",
        " v26.1.0",
        "v26.1",
        "v26.01.0",
        "v26.1.0-beta",
        "version26.1.0",
        "release-26.1.0",
        "test-17",
        "latest",
        "v",
        "",
    ] {
        assert_eq!(Version::from_tag(tag), None, "{tag:?}");
    }
}

#[test]
fn installer_file_names() {
    for (name, extension, expected) in [
        ("FortiVPNManager-26.2.0.pkg", "pkg", "26.2.0"),
        ("FortiVPNManager-26.2.0.msi", "msi", "26.2.0"),
        ("FortiVPNManager-0.0.0.pkg", "pkg", "0.0.0"),
        ("FortiVPNManager-26.10.3.msi", "msi", "26.10.3"),
    ] {
        assert_eq!(
            installer_version(name, extension),
            Some(version(expected)),
            "{name}"
        );
    }
}

#[test]
fn names_that_are_not_installer_file_names() {
    for (name, extension) in [
        ("FortiVPNManager-26.2.0.msi", "pkg"),
        ("FortiVPNManager-26.2.0.pkg", "msi"),
        ("FortiVPNManager-26.2.0.PKG", "pkg"),
        ("FortiVPNManager-26.2.0pkg", "pkg"),
        ("FortiVPNManager-26.2.0.", "pkg"),
        ("FortiVPNManager-26.2.0", "pkg"),
        ("FortiVPNManager-.pkg", "pkg"),
        ("FortiVPNManager-26.2.pkg", "pkg"),
        ("FortiVPNManager-26.02.0.pkg", "pkg"),
        ("FortiVPNManager-26.2.0-beta.pkg", "pkg"),
        ("FortiVPNManager-v26.2.0.pkg", "pkg"),
        ("FortiVPNManager26.2.0.pkg", "pkg"),
        ("fortivpnmanager-26.2.0.pkg", "pkg"),
        ("FortiVPNManager-26.2.0.pkg.pkg", "pkg"),
        (" FortiVPNManager-26.2.0.pkg", "pkg"),
        ("FortiVPNManager-26.2.0.pkg ", "pkg"),
        ("SHA256SUMS.txt", "pkg"),
        ("", "pkg"),
    ] {
        assert_eq!(
            installer_version(name, extension),
            None,
            "{name} {extension}"
        );
    }
}
