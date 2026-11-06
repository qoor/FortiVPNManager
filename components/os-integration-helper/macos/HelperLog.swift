import FortiShared
import os

/// os.Logger categories of the helper. Never log passwords, OTP codes or config file contents.
enum HelperLog {
    static let helper = Logger(subsystem: FortiIdentifiers.logSubsystem, category: "helper")
    static let routes = Logger(subsystem: FortiIdentifiers.logSubsystem, category: "routes")
}
