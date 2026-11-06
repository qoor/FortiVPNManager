import Dispatch
import Foundation
import FortiShared

/// Accepts connections from the FortiVPNManager app (the listener pins its signing identity) and
/// exports a `HelperService` bound to each one.
final class ListenerDelegate: NSObject, NSXPCListenerDelegate {
    private let updateWatcher: SelfUpdateWatcher

    init(updateWatcher: SelfUpdateWatcher) {
        self.updateWatcher = updateWatcher
        super.init()
    }

    func listener(_ listener: NSXPCListener, shouldAcceptNewConnection connection: NSXPCConnection) -> Bool {
        guard let client = EntryRelay.admit(connection, executableReplaced: updateWatcher.isReplaced) else {
            HelperLog.helper.notice("refusing a connection from pid \(connection.processIdentifier): helper is exiting")
            return false
        }
        connection.exportedInterface = NSXPCInterface(with: HelperXPCProtocol.self)
        connection.exportedObject = HelperService(client: client)
        connection.remoteObjectInterface = NSXPCInterface(with: HelperClientXPCProtocol.self)
        connection.invalidationHandler = { @Sendable in
            EntryRelay.disconnected(client)
        }
        connection.resume()
        HelperLog.helper.notice("accepted client pid \(client.pid), uid \(client.uid)")
        return true
    }
}

/// Root LaunchDaemon entry point, demand-launched through the Mach service.
private func runHelper() {
    guard let teamID = CodeSigningIdentity.currentTeamID() else {
        HelperLog.helper.fault("helper is not signed with a team identity; refusing to start")
        exit(EXIT_FAILURE)
    }

    // Rust ignores SIGPIPE only in a Rust main, and this main is Swift: without this, the Helper
    // entry writing to the stdin of a VPN client that already exited would kill the process.
    // fvm-watchdog relies on this: it cannot switch SIGPIPE off for the pipe without unsafe Rust.
    signal(SIGPIPE, SIG_IGN)
    // A no-op handler for the dispatch source below; unlike SIG_IGN it is not inherited across exec.
    signal(SIGTERM) { _ in }

    // Created before the entry starts, so a reinstall during its cleanup counts as a replacement.
    let updateWatcher = SelfUpdateWatcher()
    // Installed before the entry starts: a SIGTERM during its cleanup is passed on once the main
    // queue runs.
    let termination = DispatchSource.makeSignalSource(signal: SIGTERM, queue: .main)
    termination.setEventHandler { EntryRelay.terminationRequested() }
    termination.resume()

    EntryRelay.start()
    updateWatcher.start { EntryRelay.executableReplaced() }

    let delegate = ListenerDelegate(updateWatcher: updateWatcher)
    let listener = NSXPCListener(machServiceName: FortiIdentifiers.helperMachService)
    listener.setConnectionCodeSigningRequirement(
        CodeSigningIdentity.requirement(identifier: FortiIdentifiers.appBundleID, teamID: teamID)
    )
    listener.delegate = delegate
    listener.resume()
    HelperLog.helper.notice("helper \(HelperBuild.id.value, privacy: .public) listening (team \(teamID, privacy: .public))")

    // The listener holds its delegate weakly; keep everything alive for the process lifetime.
    withExtendedLifetime((updateWatcher, termination, delegate, listener)) {
        dispatchMain()
    }
}

runHelper()
