// swift-tools-version: 6.2
import PackageDescription

let package = Package(
    name: "FortiCore",
    platforms: [.macOS(.v15)],
    products: [
        .library(name: "FortiShared", targets: ["FortiShared"]),
        .library(name: "FortiCore", targets: ["FortiCore"]),
        .library(name: "FortiConfig", targets: ["FortiConfig"]),
        .library(name: "FortiHelperCore", targets: ["FortiHelperCore"]),
    ],
    dependencies: [
        .package(url: "https://github.com/jpsim/Yams.git", from: "6.2.2"),
    ],
    targets: [
        // XPC contract shared by the app and the root helper.
        .target(name: "FortiShared"),
        // Pure logic: reducer, parsers, renderers, IMAP/OTP matching.
        .target(name: "FortiCore", dependencies: ["FortiShared"]),
        // config.yaml model and codec. Only the app links this (the helper never parses user YAML).
        .target(name: "FortiConfig", dependencies: [.product(name: "Yams", package: "Yams")]),
        // Process supervision used by the root helper.
        .target(name: "FortiHelperCore", dependencies: ["FortiShared", "FortiCore"]),

        .testTarget(name: "FortiCoreTests", dependencies: ["FortiCore", "FortiShared"], resources: [.copy("Fixtures")]),
        .testTarget(name: "FortiConfigTests", dependencies: ["FortiConfig"]),
        .testTarget(name: "FortiHelperCoreTests", dependencies: ["FortiHelperCore", "FortiCore", "FortiShared"], resources: [.copy("Fixtures")]),
    ]
)
