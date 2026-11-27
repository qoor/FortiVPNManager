use fvm_common_base::protocol::{ExceptionRulesReadRequest, ExceptionRulesWriteRequest};
use fvm_system_protocol::service::{
    ExceptionRoutesRecord, Requester, ServiceClient, SessionEnd, TunnelSettingsRecord, TunnelSetup,
};

/// One piece of work for the network manager.
pub enum NetworkWork {
    /// Apply a session's tunnel settings to the system.
    SetUpTunnel(TunnelSetup),
    /// Undo the tunnel settings of a session that ended.
    RevertTunnel(SessionEnd),
    /// Replace the user's exception rules.
    WriteExceptionRules(Requester, ExceptionRulesWriteRequest),
    /// Answer with the user's exception rules.
    ReadExceptionRules(Requester, ExceptionRulesReadRequest),
    /// Remove the exception routes of the user whose app is quitting.
    ClearExceptionRoutes(Requester),
    /// Undo tunnel settings an earlier Helper left.
    RevertLeftoverTunnelSettings(TunnelSettingsRecord),
    /// Remove exception routes an earlier Helper left.
    RemoveLeftoverExceptionRoutes(ExceptionRoutesRecord),
    /// Something the network watcher saw.
    NetworkChanged(NetworkChange),
    Stop,
}

/// A change of the system's routes or interfaces.
pub struct NetworkChange;

/// The system's network as the VPN needs it: tunnel settings, exception routes and their state.
pub struct NetworkManager;

impl NetworkManager {
    /// Creates the manager with the client it tells the main service through.
    #[must_use]
    pub fn new(_service: ServiceClient) -> Self {
        todo!("create the manager");
        // 1. Create the queue that holds the manager's work.
        // 2. Keep the client for reporting to the main service.
    }

    /// Makes what the manager needs before it can run.
    pub fn init(&self) {
        todo!("start watching the system's routes and interfaces");
    }

    /// Puts one piece of work into the manager's queue.
    pub fn push(&self, _work: NetworkWork) {
        todo!("put the work into the work queue");
    }

    /// Takes the manager's work one at a time until it is told to stop.
    pub fn run(&self) {
        loop {
            match self.next_work() {
                NetworkWork::SetUpTunnel(setup) => self.set_up_tunnel(&setup),
                NetworkWork::RevertTunnel(end) => self.revert_tunnel(&end),
                NetworkWork::WriteExceptionRules(requester, request) => {
                    self.write_exception_rules(&requester, &request);
                }
                NetworkWork::ReadExceptionRules(requester, request) => {
                    self.read_exception_rules(&requester, &request);
                }
                NetworkWork::ClearExceptionRoutes(requester) => {
                    self.clear_exception_routes(&requester);
                }
                NetworkWork::RevertLeftoverTunnelSettings(record) => {
                    self.revert_leftover_tunnel_settings(&record);
                }
                NetworkWork::RemoveLeftoverExceptionRoutes(record) => {
                    self.remove_leftover_exception_routes(&record);
                }
                NetworkWork::NetworkChanged(change) => self.follow_network_change(&change),
                NetworkWork::Stop => break,
            }
        }
        self.stop();
    }

    fn next_work(&self) -> NetworkWork {
        todo!("wait for the next work in the work queue");
    }

    fn set_up_tunnel(&self, _setup: &TunnelSetup) {
        todo!("apply a session's tunnel settings to the system");
        // 1. Report what is going to be applied, so that it can be undone even when the Helper
        //    ends part-way.
        // 2. Give the tunnel device its address.
        // 3. Add the routes that send the VPN's networks through the tunnel.
        // 4. Set the DNS servers and domains the gateway gave.
        // 5. When another session's tunnel carries the traffic, move the system over to this
        //    one, so that only one tunnel is in use.
        // 6. Report the outcome to the session's connection.
        //
        // A step that fails undoes the steps before it and is reported with its name. The
        // undoing is reported, so that its record can be removed.
    }

    fn revert_tunnel(&self, _end: &SessionEnd) {
        todo!("undo the tunnel settings of a session that ended");
        // Only what this manager applied for the session is undone.
        // The undoing is reported, so that its record can be removed.
    }

    fn write_exception_rules(&self, _requester: &Requester, _request: &ExceptionRulesWriteRequest) {
        todo!("take the user's exception rules and bring the routes in line with them");
        // 1. Keep the rules as the user's.
        // 2. Work out which rules apply on the network the system is on now.
        // 3. Report the routes about to be added together with the ones in place, so that they
        //    can be removed even when the Helper ends part-way.
        // 4. Add the routes that are missing, and remove the ones no rule asks for any more.
        // 5. Report the routes now in place, so that they can be removed later.
        // 6. Answer the request, and report a route that could not be added.
    }

    fn read_exception_rules(&self, _requester: &Requester, _request: &ExceptionRulesReadRequest) {
        todo!("answer with the user's exception rules and the state of their routes");
    }

    fn clear_exception_routes(&self, _requester: &Requester) {
        todo!("remove the exception routes only this user asked for");
        // A route that another user's rules also ask for stays.
        // The routes now in place are reported, so that they can be removed later.
        // The app's request is not answered here; the VPN connection manager answers it.
    }

    fn revert_leftover_tunnel_settings(&self, _record: &TunnelSettingsRecord) {
        todo!("undo tunnel settings an earlier Helper left in the system");
        // This Helper applied nothing, so what to undo comes from the record.
        // The undoing is reported, so that the record can be removed.
    }

    fn remove_leftover_exception_routes(&self, _record: &ExceptionRoutesRecord) {
        todo!("remove exception routes an earlier Helper left in the system");
        // This Helper added nothing, so what to remove comes from the record.
        // The routes now in place are reported, which takes the place of the record.
    }

    fn follow_network_change(&self, _change: &NetworkChange) {
        todo!("bring the exception routes and the tunnel state in line with the changed network");
        // 1. Work out again which exception rules apply.
        // 2. Report the routes about to be added together with the ones in place, so that they
        //    can be removed even when the Helper ends part-way.
        // 3. Add and remove routes to match.
        // 4. Report the routes now in place, so that they can be removed later, and report a
        //    route that could not be added.
        // 5. Read from the OS whether traffic for the VPN's networks still goes through the
        //    session's tunnel, and report it when it changed.
        //
        // The check only reads from the OS; it sends nothing over the network.
    }

    fn stop(&self) {
        todo!("stop watching the network");
    }
}
