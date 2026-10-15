import Dispatch
import Foundation
import FortiShared

/// Accepts connections from the FortiVPNManager app (the listener pins its signing identity) and
/// exports a `HelperService` bound to each one.
final class ListenerDelegate: NSObject, NSXPCListenerDelegate {
    private let controller: TunnelController

    init(controller: TunnelController) {
        self.controller = controller
        super.init()
    }

    func listener(_ listener: NSXPCListener, shouldAcceptNewConnection connection: NSXPCConnection) -> Bool {
        // A replaced binary fails the app's signature check anyway: exit (when idle) so launchd
        // starts the new one; the app retries once.
        guard controller.admitNewConnection() else {
            HelperLog.helper.notice("refusing a connection from pid \(connection.processIdentifier): helper is exiting")
            return false
        }
        let client = ClientLink(connection: connection)
        connection.exportedInterface = NSXPCInterface(with: HelperXPCProtocol.self)
        connection.exportedObject = HelperService(controller: controller, client: client)
        connection.remoteObjectInterface = NSXPCInterface(with: HelperClientXPCProtocol.self)
        connection.invalidationHandler = { @Sendable [controller] in
            controller.clientInvalidated(client)
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

    // EPIPE on a child's stdin means the child exited (ProcessRunner also sets F_SETNOSIGPIPE).
    signal(SIGPIPE, SIG_IGN)
    // Delivered through the dispatch source below instead of terminating the process.
    signal(SIGTERM, SIG_IGN)

    let updateWatcher = SelfUpdateWatcher()
    let controller = TunnelController(updateWatcher: updateWatcher)
    // Installed before the sweep: a SIGTERM during it is handled once the main queue runs.
    let termination = DispatchSource.makeSignalSource(signal: SIGTERM, queue: .main)
    termination.setEventHandler { controller.shutdown() }
    termination.resume()

    OrphanSweeper.sweep()
    updateWatcher.start { controller.binaryReplaced() }

    let delegate = ListenerDelegate(controller: controller)
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
