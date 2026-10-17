use std::collections::BTreeSet;

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct RouteState {
    pub(crate) destination: String,
    pub(crate) prefix: u8,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(tag = "action", content = "route", rename_all = "snake_case")]
pub(crate) enum RouteOperation {
    Remove(RouteState),
    Add(RouteState),
}

/// Owns only the routes whose creation was journaled by this Agent.
///
/// Planning is deliberately separate from OS execution: callers persist the
/// returned pending operations before invoking a platform route command, then
/// call `complete` and persist again.  A process crash therefore leaves enough
/// evidence to safely retry or clean up the exact Tinc-interface route.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct RouteManager {
    desired: Vec<RouteState>,
    owned: Vec<RouteState>,
    pending: Vec<RouteOperation>,
}

impl RouteManager {
    pub(crate) fn restore(
        desired: Vec<RouteState>,
        owned: Vec<RouteState>,
        pending: Vec<RouteOperation>,
    ) -> Self {
        Self {
            desired,
            owned,
            pending,
        }
    }

    pub(crate) fn desired(&self) -> &[RouteState] {
        &self.desired
    }

    pub(crate) fn owned(&self) -> &[RouteState] {
        &self.owned
    }

    pub(crate) fn pending(&self) -> &[RouteOperation] {
        &self.pending
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.owned.is_empty() && self.pending.is_empty()
    }

    pub(crate) fn plan(&mut self, desired: &[RouteState]) -> bool {
        if !self.pending.is_empty() {
            return false;
        }
        self.desired = desired.to_vec();
        self.pending = route_changes(&self.owned, desired);
        !self.pending.is_empty()
    }

    pub(crate) fn begin_cleanup(&mut self) {
        // A pending add was journaled before the OS command.  Treat it as
        // potentially owned during recovery, so a crash after route creation
        // cannot leave an untracked route behind.
        for operation in self.pending.drain(..) {
            if let RouteOperation::Add(route) = operation {
                insert_route(&mut self.owned, route);
            }
        }
        self.desired.clear();
        self.pending = self
            .owned
            .iter()
            .cloned()
            .map(RouteOperation::Remove)
            .collect();
    }

    pub(crate) fn complete(&mut self, completed: &RouteOperation) {
        match completed {
            RouteOperation::Add(route) => insert_route(&mut self.owned, route.clone()),
            RouteOperation::Remove(route) => remove_route(&mut self.owned, route),
        }
        if let Some(index) = self.pending.iter().position(|item| item == completed) {
            self.pending.remove(index);
        }
    }
}

fn insert_route(routes: &mut Vec<RouteState>, route: RouteState) {
    if !routes.contains(&route) {
        routes.push(route);
    }
}

fn remove_route(routes: &mut Vec<RouteState>, deleted: &RouteState) {
    if let Some(index) = routes.iter().position(|route| route == deleted) {
        routes.remove(index);
    }
}

fn route_changes(current: &[RouteState], desired: &[RouteState]) -> Vec<RouteOperation> {
    current
        .iter()
        .filter(|route| !desired.contains(route))
        .cloned()
        .map(RouteOperation::Remove)
        .chain(
            desired
                .iter()
                .filter(|route| !current.contains(route))
                .cloned()
                .map(RouteOperation::Add),
        )
        .collect()
}

pub(crate) fn validate_routes(
    routes: &[RouteState],
    own_ip: std::net::Ipv4Addr,
    max_routes: usize,
) -> bool {
    if routes.len() > max_routes {
        return false;
    }
    let mut destinations = BTreeSet::new();
    routes.iter().all(|route| {
        let Ok(destination) = route.destination.parse::<std::net::Ipv4Addr>() else {
            return false;
        };
        let octets = destination.octets();
        route.prefix == 32
            && destination != own_ip
            && !destination.is_unspecified()
            && !destination.is_loopback()
            && !destination.is_multicast()
            && octets != [255, 255, 255, 255]
            && !(octets[0] == 169 && octets[1] == 254)
            && destinations.insert(destination)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn route(last: u8) -> RouteState {
        RouteState {
            destination: format!("10.0.11.{last}"),
            prefix: 32,
        }
    }

    #[test]
    fn repeated_snapshot_is_idempotent_and_revocation_precedes_addition() {
        let gateway = route(1);
        let old_peer = route(211);
        let new_peer = route(212);
        let mut manager = RouteManager::restore(
            vec![gateway.clone(), old_peer.clone()],
            vec![gateway.clone(), old_peer.clone()],
            Vec::new(),
        );

        assert!(!manager.plan(&[gateway.clone(), old_peer]));
        assert!(manager.pending().is_empty());

        assert!(manager.plan(&[gateway, new_peer.clone()]));
        assert!(matches!(manager.pending()[0], RouteOperation::Remove(_)));
        assert_eq!(
            manager.pending().last(),
            Some(&RouteOperation::Add(new_peer))
        );
    }

    #[test]
    fn partial_failure_keeps_remaining_journal_and_completed_ownership() {
        let gateway = route(1);
        let peer = route(212);
        let mut manager = RouteManager::default();
        assert!(manager.plan(&[gateway.clone(), peer.clone()]));
        let first = manager.pending()[0].clone();

        manager.complete(&first);

        assert_eq!(manager.owned(), &[gateway]);
        assert_eq!(manager.pending(), &[RouteOperation::Add(peer)]);
    }

    #[test]
    fn cleanup_claims_journaled_add_to_close_crash_window() {
        let peer = route(212);
        let mut manager = RouteManager::default();
        manager.plan(std::slice::from_ref(&peer));

        manager.begin_cleanup();

        assert_eq!(manager.owned(), std::slice::from_ref(&peer));
        assert_eq!(manager.pending(), &[RouteOperation::Remove(peer)]);
        assert!(manager.desired().is_empty());
    }

    #[test]
    fn cleanup_failure_keeps_failed_route_while_other_results_are_recorded() {
        let gateway = route(1);
        let peer_a = route(211);
        let peer_b = route(212);
        let routes = vec![gateway.clone(), peer_a.clone(), peer_b.clone()];
        let mut manager = RouteManager::restore(routes.clone(), routes, Vec::new());
        manager.begin_cleanup();

        // The gateway deletion failed. The executor can still record the two
        // successful peer deletions without losing ownership of the failure.
        manager.complete(&RouteOperation::Remove(peer_a));
        manager.complete(&RouteOperation::Remove(peer_b));

        assert_eq!(manager.owned(), std::slice::from_ref(&gateway));
        assert_eq!(manager.pending(), &[RouteOperation::Remove(gateway)]);
    }

    #[test]
    fn two_nodes_receive_gateway_and_other_peer_only() {
        let gateway = route(1);
        let node_a = route(211);
        let node_b = route(212);
        let mut a = RouteManager::default();
        let mut b = RouteManager::default();

        a.plan(&[gateway.clone(), node_b.clone()]);
        b.plan(&[gateway.clone(), node_a.clone()]);

        assert_eq!(
            a.pending(),
            &[
                RouteOperation::Add(gateway.clone()),
                RouteOperation::Add(node_b)
            ]
        );
        assert_eq!(
            b.pending(),
            &[RouteOperation::Add(gateway), RouteOperation::Add(node_a)]
        );
    }
}
