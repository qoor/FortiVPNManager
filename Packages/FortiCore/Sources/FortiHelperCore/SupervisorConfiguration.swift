import FortiShared

/// What `TunnelSupervisor` runs. Production uses `openfortivpn(path:configPath:)`; tests run
/// `/bin/sh` with a fixture script.
public struct ExecutableSpec: Sendable {
    /// Absolute path passed to `posix_spawn`.
    public var path: String
    /// The complete argv, including argv[0].
    public var arguments: [String]
    /// The complete environment of the child.
    public var environment: [String: String]

    public init(path: String, arguments: [String], environment: [String: String]) {
        self.path = path
        self.arguments = arguments
        self.environment = environment
    }

    /// The only environment variable the helper passes on. openfortivpn honours `https_proxy` /
    /// `all_proxy`, so nothing else may leak through.
    public static let helperPATH = "/usr/bin:/bin:/usr/sbin:/sbin"

    public static func openfortivpn(path: String, configPath: String) -> ExecutableSpec {
        ExecutableSpec(path: path, arguments: [path, "-c", configPath], environment: ["PATH": helperPATH])
    }
}

/// A run phase guarded by a helper watchdog (named in its `‹watchdog: …›` log line).
public enum WatchdogPhase: String, Sendable {
    case spawnToGateway
    case gatewayToPromptOrAuth
    case otpPromptToSubmit
    case otpSubmitToAuth
    case authToTunnel
}

/// Per-phase time limits enforced by the helper. The whole-attempt and OTP limits belong to the
/// app; `otpPromptToSubmit` is only a backstop.
public struct WatchdogLimits: Sendable {
    /// spawn → `Connected to gateway.`
    public var spawnToGateway: Duration
    /// `Connected to gateway.` → OTP prompt or `Authenticated.`
    public var gatewayToPromptOrAuth: Duration
    /// OTP prompt → `submitOTP`.
    public var otpPromptToSubmit: Duration
    /// `submitOTP` → `Authenticated.`
    public var otpSubmitToAuth: Duration
    /// `Authenticated.` → `Tunnel is up and running.`
    public var authToTunnel: Duration
    /// SIGTERM → SIGKILL.
    public var terminate: Duration

    public init(
        spawnToGateway: Duration = .seconds(30),
        gatewayToPromptOrAuth: Duration = .seconds(30),
        otpPromptToSubmit: Duration = .seconds(65),
        otpSubmitToAuth: Duration = .seconds(20),
        authToTunnel: Duration = .seconds(40),
        terminate: Duration = .seconds(10)
    ) {
        self.spawnToGateway = spawnToGateway
        self.gatewayToPromptOrAuth = gatewayToPromptOrAuth
        self.otpPromptToSubmit = otpPromptToSubmit
        self.otpSubmitToAuth = otpSubmitToAuth
        self.authToTunnel = authToTunnel
        self.terminate = terminate
    }

    /// The watchdog that runs once `milestone` is the furthest milestone; `nil` once the tunnel is up.
    public func watchdog(after milestone: Milestone) -> (phase: WatchdogPhase, limit: Duration)? {
        switch milestone {
        case .spawned: (.spawnToGateway, spawnToGateway)
        case .connectedToGateway: (.gatewayToPromptOrAuth, gatewayToPromptOrAuth)
        case .otpPrompted: (.otpPromptToSubmit, otpPromptToSubmit)
        case .otpSubmitted: (.otpSubmitToAuth, otpSubmitToAuth)
        case .authenticated: (.authToTunnel, authToTunnel)
        case .tunnelUp: nil
        }
    }
}

/// Output handling intervals.
public struct SupervisorTiming: Sendable {
    /// Output lines are delivered in batches at most this far apart. Pending lines are also
    /// delivered right before every event.
    public var outputBatchInterval: Duration
    /// How long to wait for stdout/stderr EOF after the child exited (pppd may still hold stderr).
    public var pipeDrainLimit: Duration
    /// An unfinished line (typically a prompt) is shown after this much silence.
    public var lineIdleFlush: Duration

    public init(
        outputBatchInterval: Duration = .milliseconds(100),
        pipeDrainLimit: Duration = .seconds(1),
        lineIdleFlush: Duration = .milliseconds(250)
    ) {
        self.outputBatchInterval = outputBatchInterval
        self.pipeDrainLimit = pipeDrainLimit
        self.lineIdleFlush = lineIdleFlush
    }
}
