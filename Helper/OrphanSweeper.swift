import Darwin

/// Cleans up after a helper that died during a run (DESIGN §5.2 "orphan cleanup"). Runs once at startup,
/// before any connection is accepted: stops the processes recorded in the pidfile that are
/// provably still the same root processes, repairs routes, then deletes the pidfile and stale
/// config files.
enum OrphanSweeper {
    static func sweep() {
        do {
            try RunDirectory.prepare()
        } catch {
            HelperLog.helper.error("cannot prepare the run directory: \(error.description, privacy: .public)")
            return
        }
        defer { RunDirectory.removeLeftovers() }
        guard let record = RunDirectory.readRecord() else { return }

        let orphans = record.processes.filter { isOurExecutable($0.path) && $0.isRunning }
        if !orphans.isEmpty {
            HelperLog.helper.notice("stopping \(orphans.count) process(es) left by a previous helper: \(orphans.map(\.pid), privacy: .public)")
            terminate(orphans)
        }
        let repaired = RouteRepair.repair(gatewayIP: record.gatewayIP)
        HelperLog.helper.notice("cleaned up after an unfinished run (route repair: \(String(describing: repaired), privacy: .public))")
    }

    /// openfortivpn from the Homebrew Cellar or Apple's pppd; nothing else is ever signalled.
    private static func isOurExecutable(_ path: String) -> Bool {
        path == ProcessIdentity.pppdPath || path.hasPrefix(BinaryLocator.cellarPrefix)
    }

    /// SIGTERM, up to 10 s for a clean exit (openfortivpn restores routes), then SIGKILL.
    private static func terminate(_ processes: [ProcessIdentity]) {
        for process in processes {
            kill(process.pid, SIGTERM)
        }
        let survivors = waitForExit(of: processes, limit: .seconds(10))
        guard !survivors.isEmpty else { return }
        HelperLog.helper.notice("sending SIGKILL to \(survivors.map(\.pid), privacy: .public)")
        for process in survivors {
            kill(process.pid, SIGKILL)
        }
        let stuck = waitForExit(of: survivors, limit: .seconds(2))
        if !stuck.isEmpty {
            HelperLog.helper.error("processes still running after SIGKILL: \(stuck.map(\.pid), privacy: .public)")
        }
    }

    /// Polls until the processes are gone or `limit` passed; returns those still running.
    private static func waitForExit(of processes: [ProcessIdentity], limit: Duration) -> [ProcessIdentity] {
        let deadline = ContinuousClock.now + limit
        var remaining = processes.filter(\.isRunning)
        while !remaining.isEmpty, ContinuousClock.now < deadline {
            usleep(100_000)
            remaining = remaining.filter(\.isRunning)
        }
        return remaining
    }
}
