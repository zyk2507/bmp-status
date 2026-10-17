//! In-memory runtime state, message processing and aggregation.

use std::cmp::Ordering;
use std::collections::{BTreeMap, HashMap, VecDeque};
use std::net::SocketAddr;
use std::sync::atomic::{AtomicU64, Ordering as AtomicOrdering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Instant;

use bgpkit_parser::Elementor;
use bgpkit_parser::models::{BgpElem, BgpMessage, ElemType};
use bgpkit_parser::parser::bmp::messages::{BmpMessage, BmpMessageBody, InitiationTlvType};
use serde::Serialize;
use tokio::sync::{broadcast, watch};

use crate::admin::{AdminStore, InstanceRecord};
use crate::config::Config;
use crate::demo::DemoSession;
use crate::model::*;
use crate::runtime_config::{RuntimeConfig, RuntimeConfigStore};

pub const MAX_ROUTE_VIEWS: usize = 64;
pub const MAX_CLOSED_CONNECTIONS: usize = 40;

pub fn now_secs() -> f64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs_f64()
}

#[derive(Debug, Clone, Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct Totals {
    pub messages: u64,
    pub route_monitoring: u64,
    pub statistics_report: u64,
    pub peer_up: u64,
    pub peer_down: u64,
    pub initiation: u64,
    pub termination: u64,
    pub route_mirroring: u64,
    pub announcements: u64,
    pub withdrawals: u64,
    pub parse_errors: u64,
    pub rejected_connections: u64,
    pub bytes: u64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Overview {
    pub started_at: f64,
    /// Monotonic sequence for this server process. Used by the frontend to
    /// discard an older HTTP snapshot that arrives after a newer WebSocket
    /// overview.
    pub overview_seq: u64,
    pub uptime_seconds: u64,
    pub bmp_bind: String,
    pub http_bind: String,
    pub bmp_listening: bool,
    pub demo: DemoStatus,
    pub totals: Totals,
    pub rate_per_sec: f64,
    pub active_connections: usize,
    pub connections_total: u64,
    pub max_msg_size: u32,
    pub idle_timeout_secs: u64,
    pub max_connections: usize,
    pub peers_total: usize,
    pub peers_up: usize,
    pub active_routes: usize,
    pub allowlist_enforced: bool,
    pub instances: Vec<InstanceStatus>,
    pub buckets: Vec<BucketView>,
    pub peers: Vec<PeerView>,
    pub top_origins: Vec<AsnCount>,
    pub top_peer_asns: Vec<AsnCount>,
    pub last_error: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RoutesResponse {
    pub total: usize,
    pub returned: usize,
    pub page: usize,
    pub page_size: usize,
    pub total_pages: usize,
    pub truncated: bool,
    pub routes: Vec<RouteEntry>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConnectionsResponse {
    pub active: Vec<ConnectionView>,
    pub recent: Vec<ConnectionView>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", content = "data", rename_all = "snake_case")]
pub enum ServerEvent {
    Overview(Arc<Overview>),
    Message(Arc<MessageView>),
    Lagged { skipped: u64 },
}

#[derive(Debug, Clone, Default)]
struct BucketCounts {
    messages: u64,
    announcements: u64,
    withdrawals: u64,
    errors: u64,
}

#[derive(Debug, Clone, Default)]
struct InstanceActivity {
    messages: u64,
    bytes: u64,
    last_seen: Option<f64>,
}

struct PeerState {
    view: PeerView,
}

#[derive(Default)]
struct Inner {
    next_message_id: u64,
    next_conn_id: u64,
    connections_total: u64,
    totals: Totals,
    messages: VecDeque<Arc<MessageView>>,
    peers: HashMap<String, PeerState>,
    routes: HashMap<(String, String), RouteEntry>,
    route_order: VecDeque<(String, String)>,
    limits: RuntimeConfig,
    buckets: BTreeMap<u64, BucketCounts>,
    top_origins: HashMap<u32, u64>,
    instance_activity: HashMap<String, InstanceActivity>,
    connections: HashMap<u64, ConnectionView>,
    closed_connections: VecDeque<ConnectionView>,
    last_error: Option<String>,
}

pub struct AppState {
    pub config: Arc<Config>,
    pub admin: Arc<AdminStore>,
    pub runtime: Arc<RuntimeConfigStore>,
    pub(crate) demo: Mutex<Option<DemoSession>>,
    pub(crate) demo_seq: AtomicU64,
    started_at: f64,
    started_instant: Instant,
    overview_seq: AtomicU64,
    inner: Mutex<Inner>,
    tx: broadcast::Sender<ServerEvent>,
    shutdown_tx: watch::Sender<bool>,
}

impl AppState {
    pub fn new(
        config: Arc<Config>,
        admin: Arc<AdminStore>,
        runtime: Arc<RuntimeConfigStore>,
    ) -> Self {
        let (tx, _) = broadcast::channel(2_048);
        let (shutdown_tx, _) = watch::channel(false);
        let limits = runtime.get();
        Self {
            config,
            admin,
            runtime,
            demo: Mutex::new(None),
            demo_seq: AtomicU64::new(0),
            started_at: now_secs(),
            started_instant: Instant::now(),
            overview_seq: AtomicU64::new(0),
            inner: Mutex::new(Inner {
                limits,
                ..Inner::default()
            }),
            tx,
            shutdown_tx,
        }
    }

    pub fn runtime_config(&self) -> RuntimeConfig {
        self.runtime.get()
    }

    pub fn update_runtime_config(&self, next: RuntimeConfig) -> anyhow::Result<RuntimeConfig> {
        let next = self.runtime.update(next)?;
        let mut inner = self.lock_inner();
        let previous = inner.limits;
        inner.limits = next;
        if next.max_routes == 0 {
            inner.route_order.clear();
        } else if previous.max_routes == 0 || inner.route_order.is_empty() {
            rebuild_route_order(&mut inner);
        }
        trim_state(&mut inner);
        drop(inner);
        self.broadcast_overview();
        Ok(next)
    }

    fn lock_inner(&self) -> MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(|e| e.into_inner())
    }

    pub fn subscribe(&self) -> broadcast::Receiver<ServerEvent> {
        self.tx.subscribe()
    }

    pub fn subscribe_shutdown(&self) -> watch::Receiver<bool> {
        self.shutdown_tx.subscribe()
    }

    pub fn shutdown(&self) {
        let _ = self.shutdown_tx.send(true);
    }

    pub fn is_shutting_down(&self) -> bool {
        *self.shutdown_tx.borrow()
    }

    pub fn next_demo_id(&self) -> u64 {
        self.demo_seq.fetch_add(1, AtomicOrdering::Relaxed) + 1
    }

    pub fn demo_status(&self) -> DemoStatus {
        let demo = self.demo.lock().unwrap_or_else(|e| e.into_inner());
        match demo.as_ref() {
            Some(session) => DemoStatus {
                running: true,
                started_at: Some(session.started_at),
                sent: session.sent.load(AtomicOrdering::Relaxed),
            },
            None => DemoStatus::default(),
        }
    }

    pub fn clear_demo(&self, id: u64) {
        let mut demo = self.demo.lock().unwrap_or_else(|e| e.into_inner());
        if demo.as_ref().map(|s| s.id) == Some(id) {
            *demo = None;
        }
    }

    pub fn broadcast_overview(&self) {
        let overview = Arc::new(self.overview());
        let _ = self.tx.send(ServerEvent::Overview(overview));
    }

    // ------------------------------------------------------------------
    // Connections
    // ------------------------------------------------------------------

    pub fn connection_open(&self, addr: SocketAddr) -> u64 {
        let now = now_secs();
        let mut inner = self.lock_inner();
        inner.next_conn_id += 1;
        let id = inner.next_conn_id;
        inner.connections_total += 1;
        let view = ConnectionView {
            id,
            peer: addr.to_string(),
            router_ip: addr.ip().to_string(),
            router_name: None,
            since: now,
            last_message_at: now,
            messages: 0,
            bytes: 0,
            state: "active".to_string(),
            close_reason: None,
        };
        inner.connections.insert(id, view);
        id
    }

    pub fn record_rejected_connection(&self, addr: SocketAddr, reason: String) {
        let now = now_secs();
        let mut inner = self.lock_inner();
        inner.next_conn_id += 1;
        let id = inner.next_conn_id;
        inner.connections_total += 1;
        inner.totals.rejected_connections += 1;
        inner.closed_connections.push_back(ConnectionView {
            id,
            peer: addr.to_string(),
            router_ip: addr.ip().to_string(),
            router_name: None,
            since: now,
            last_message_at: now,
            messages: 0,
            bytes: 0,
            state: "rejected".to_string(),
            close_reason: Some(reason),
        });
        while inner.closed_connections.len() > MAX_CLOSED_CONNECTIONS {
            inner.closed_connections.pop_front();
        }
    }

    pub fn connection_close(&self, conn_id: u64, reason: &str) {
        let mut inner = self.lock_inner();
        let Some(mut conn) = inner.connections.remove(&conn_id) else {
            return;
        };
        conn.state = "closed".to_string();
        conn.close_reason = Some(reason.to_string());
        let router_ip = conn.router_ip.clone();
        let router_name = conn.router_name.clone();
        inner.closed_connections.push_back(conn);
        while inner.closed_connections.len() > MAX_CLOSED_CONNECTIONS {
            inner.closed_connections.pop_front();
        }

        // A lost BMP session means we can no longer trust the peer session state.
        for peer in inner.peers.values_mut() {
            if peer.view.router.as_deref() == Some(router_ip.as_str()) && peer.view.session == "up"
            {
                peer.view.session = "unknown".to_string();
                peer.view.session_since = None;
            }
        }
        tracing::info!(
            conn_id,
            router_ip,
            router_name = router_name.as_deref().unwrap_or("-"),
            reason,
            "BMP connection closed"
        );
    }

    pub fn connections(&self) -> ConnectionsResponse {
        let inner = self.lock_inner();
        let mut active: Vec<ConnectionView> = inner.connections.values().cloned().collect();
        active.sort_by(|a, b| b.since.partial_cmp(&a.since).unwrap_or(Ordering::Equal));
        let mut recent: Vec<ConnectionView> = inner.closed_connections.iter().cloned().collect();
        recent.reverse();
        ConnectionsResponse { active, recent }
    }

    // ------------------------------------------------------------------
    // Ingest
    // ------------------------------------------------------------------

    pub fn ingest(
        &self,
        conn_id: u64,
        addr: SocketAddr,
        msg: BmpMessage,
        raw_len: usize,
        raw_hex: String,
        raw_truncated: bool,
    ) {
        let now = now_secs();
        let router_ip = addr.ip().to_string();
        let mut inner = self.lock_inner();

        if let Some(conn) = inner.connections.get_mut(&conn_id) {
            conn.messages += 1;
            conn.bytes += raw_len as u64;
            conn.last_message_at = now;
        }
        let activity = inner
            .instance_activity
            .entry(router_ip.clone())
            .or_default();
        activity.messages += 1;
        activity.bytes += raw_len as u64;
        activity.last_seen = Some(now);

        let view = build_message(
            &mut inner,
            conn_id,
            &router_ip,
            msg,
            now,
            raw_len,
            raw_hex,
            raw_truncated,
        );

        inner.totals.messages += 1;
        inner.totals.bytes += raw_len as u64;

        let bucket = inner.buckets.entry(now as u64).or_default();
        bucket.messages += 1;
        bucket.announcements += view.announced as u64;
        bucket.withdrawals += view.withdrawn as u64;
        if view.error.is_some() {
            bucket.errors += 1;
        }

        let arc = Arc::new(view);
        inner.messages.push_back(arc.clone());
        trim_state(&mut inner);

        drop(inner);
        let _ = self.tx.send(ServerEvent::Message(arc));
    }

    pub fn ingest_error(
        &self,
        conn_id: u64,
        router_ip: std::net::IpAddr,
        msg_type: &str,
        code: u8,
        error: String,
        raw_len: usize,
        raw_hex: String,
        raw_truncated: bool,
    ) {
        let now = now_secs();
        let mut inner = self.lock_inner();
        if let Some(conn) = inner.connections.get_mut(&conn_id) {
            conn.messages += 1;
            conn.bytes += raw_len as u64;
            conn.last_message_at = now;
        }
        let router_ip_str = router_ip.to_string();
        let activity = inner
            .instance_activity
            .entry(router_ip_str.clone())
            .or_default();
        activity.messages += 1;
        activity.bytes += raw_len as u64;
        activity.last_seen = Some(now);

        inner.totals.messages += 1;
        inner.totals.parse_errors += 1;
        inner.totals.bytes += raw_len as u64;
        inner.last_error = Some(error.clone());

        let bucket = inner.buckets.entry(now as u64).or_default();
        bucket.messages += 1;
        bucket.errors += 1;

        inner.next_message_id += 1;
        let router_name = inner
            .connections
            .get(&conn_id)
            .and_then(|c| c.router_name.clone());
        let view = MessageView {
            id: inner.next_message_id,
            ts: now,
            received_at: now,
            msg_type: msg_type.to_string(),
            msg_type_code: code,
            size: raw_len,
            router_addr: router_ip_str,
            router_name,
            peer: None,
            bgp_type: None,
            announced: 0,
            withdrawn: 0,
            routes_total: 0,
            routes_truncated: 0,
            routes: Vec::new(),
            stats: Vec::new(),
            peer_down_reason: None,
            peer_down_detail: None,
            peer_up: None,
            tlvs: Vec::new(),
            termination_reason: None,
            error: Some(error),
            raw_hex,
            raw_truncated,
        };
        let arc = Arc::new(view);
        inner.messages.push_back(arc.clone());
        trim_state(&mut inner);
        drop(inner);
        let _ = self.tx.send(ServerEvent::Message(arc));
    }

    // ------------------------------------------------------------------
    // Queries
    // ------------------------------------------------------------------

    pub fn recent_messages(&self, limit: usize) -> Vec<Arc<MessageView>> {
        let inner = self.lock_inner();
        inner.messages.iter().rev().take(limit).cloned().collect()
    }

    pub fn query_messages(
        &self,
        limit: usize,
        msg_type: Option<&str>,
        q: Option<&str>,
    ) -> Vec<Arc<MessageView>> {
        let needle = q
            .map(|s| s.trim().to_ascii_lowercase())
            .filter(|s| !s.is_empty());
        let inner = self.lock_inner();
        inner
            .messages
            .iter()
            .rev()
            .filter(|m| {
                msg_type
                    .map(|ty| message_type_matches(m, ty))
                    .unwrap_or(true)
            })
            .filter(|m| {
                needle
                    .as_deref()
                    .map(|n| message_matches(m, n))
                    .unwrap_or(true)
            })
            .take(limit)
            .cloned()
            .collect()
    }

    pub fn query_routes(
        &self,
        page: usize,
        limit: usize,
        q: Option<&str>,
        peer: Option<&str>,
        router: Option<&str>,
    ) -> RoutesResponse {
        let needle = q
            .map(|s| s.trim().to_ascii_lowercase())
            .filter(|s| !s.is_empty());
        let peer_filter = peer.map(|s| s.trim().to_string()).filter(|s| !s.is_empty());
        let router_filter = router
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty());
        let limit = limit.max(1);
        let inner = self.lock_inner();
        let mut routes: Vec<RouteEntry> = inner
            .routes
            .values()
            .filter(|r| {
                needle
                    .as_deref()
                    .map(|n| route_matches(r, n))
                    .unwrap_or(true)
            })
            .filter(|r| {
                peer_filter
                    .as_deref()
                    .map(|p| r.peer_ip == p || r.peer_asn.to_string() == p || r.peer_key == p)
                    .unwrap_or(true)
            })
            .filter(|r| {
                router_filter
                    .as_deref()
                    .map(|router_ip| {
                        inner
                            .peers
                            .get(&r.peer_key)
                            .and_then(|p| p.view.router.as_deref())
                            == Some(router_ip)
                    })
                    .unwrap_or(true)
            })
            .cloned()
            .collect();
        routes.sort_by(|a, b| {
            b.updated_at
                .partial_cmp(&a.updated_at)
                .unwrap_or(Ordering::Equal)
        });
        let total = routes.len();
        let total_pages = total.div_ceil(limit).max(1);
        let page = page.max(1).min(total_pages);
        let start = (page - 1) * limit;
        let end = (start + limit).min(total);
        let returned = end.saturating_sub(start);
        routes = if start < total {
            routes[start..end].to_vec()
        } else {
            Vec::new()
        };
        RoutesResponse {
            total,
            returned,
            page,
            page_size: limit,
            total_pages,
            truncated: page < total_pages,
            routes,
        }
    }

    pub fn peers_list(&self) -> Vec<PeerView> {
        let inner = self.lock_inner();
        let mut peers: Vec<PeerView> = inner.peers.values().map(|p| p.view.clone()).collect();
        peers.sort_by(|a, b| {
            b.last_seen
                .partial_cmp(&a.last_seen)
                .unwrap_or(Ordering::Equal)
        });
        peers
    }

    pub fn instance_statuses(&self) -> Vec<InstanceStatus> {
        let records = self.admin.instances();
        let inner = self.lock_inner();
        records
            .into_iter()
            .map(|r| self.instance_status(&inner, r))
            .collect()
    }

    fn instance_status(&self, inner: &Inner, record: InstanceRecord) -> InstanceStatus {
        let activity = inner.instance_activity.get(&record.router_ip);
        let active_connections = inner
            .connections
            .values()
            .filter(|c| c.router_ip == record.router_ip)
            .count();
        let peers = inner
            .peers
            .values()
            .filter(|p| p.view.router.as_deref() == Some(record.router_ip.as_str()))
            .count();
        InstanceStatus {
            id: record.id,
            name: record.name,
            router_ip: record.router_ip,
            description: record.description,
            enabled: record.enabled,
            online: active_connections > 0,
            active_connections,
            peers,
            messages: activity.map(|a| a.messages).unwrap_or(0),
            bytes: activity.map(|a| a.bytes).unwrap_or(0),
            last_seen: activity.and_then(|a| a.last_seen),
            created_at: record.created_at,
            updated_at: record.updated_at,
        }
    }

    pub fn overview(&self) -> Overview {
        let now = now_secs();
        let now_sec = now as u64;
        let overview_seq = self.overview_seq.fetch_add(1, AtomicOrdering::Relaxed) + 1;
        let allowlist_enforced = self.admin.enforce_allowlist();

        let mut inner = self.lock_inner();

        let cutoff = now_sec.saturating_sub(600);
        while let Some(entry) = inner.buckets.first_entry() {
            if *entry.key() < cutoff {
                entry.remove();
            } else {
                break;
            }
        }

        let start = now_sec.saturating_sub(119);
        let mut buckets = Vec::with_capacity(120);
        for t in start..=now_sec {
            let b = inner.buckets.get(&t);
            buckets.push(BucketView {
                t,
                messages: b.map(|x| x.messages).unwrap_or(0),
                announcements: b.map(|x| x.announcements).unwrap_or(0),
                withdrawals: b.map(|x| x.withdrawals).unwrap_or(0),
                errors: b.map(|x| x.errors).unwrap_or(0),
            });
        }
        let rate_window_start = now_sec.saturating_sub(9);
        let recent_messages: u64 = (rate_window_start..=now_sec)
            .filter_map(|t| inner.buckets.get(&t))
            .map(|b| b.messages)
            .sum();
        let rate_per_sec = recent_messages as f64 / 10.0;

        let peers_total = inner.peers.len();
        let peers_up = inner
            .peers
            .values()
            .filter(|p| p.view.session == "up")
            .count();

        let mut top_peers_map: HashMap<u32, u64> = HashMap::new();
        let mut all_peers: Vec<PeerView> = Vec::with_capacity(peers_total);
        for peer in inner.peers.values() {
            *top_peers_map.entry(peer.view.asn).or_default() += peer.view.messages;
            all_peers.push(peer.view.clone());
        }
        all_peers.sort_by(|a, b| {
            b.last_seen
                .partial_cmp(&a.last_seen)
                .unwrap_or(Ordering::Equal)
        });

        let mut top_origins: Vec<AsnCount> = inner
            .top_origins
            .iter()
            .map(|(asn, count)| AsnCount {
                asn: *asn,
                count: *count,
            })
            .collect();
        top_origins.sort_by(|a, b| b.count.cmp(&a.count));
        top_origins.truncate(8);

        let mut top_peer_asns: Vec<AsnCount> = top_peers_map
            .into_iter()
            .map(|(asn, count)| AsnCount { asn, count })
            .collect();
        top_peer_asns.sort_by(|a, b| b.count.cmp(&a.count));
        top_peer_asns.truncate(8);

        let active_routes = inner.routes.len();
        let active_connections = inner.connections.len();
        let connections_total = inner.connections_total;
        let totals = inner.totals.clone();
        let last_error = inner.last_error.clone();
        let visible_peers = all_peers.into_iter().take(80).collect();

        drop(inner);

        let instances = self.instance_statuses();
        let demo = self.demo_status();

        Overview {
            started_at: self.started_at,
            overview_seq,
            uptime_seconds: self.started_instant.elapsed().as_secs(),
            bmp_bind: self.config.bmp_bind.to_string(),
            http_bind: self.config.http_bind.to_string(),
            bmp_listening: true,
            demo,
            totals,
            rate_per_sec,
            active_connections,
            connections_total,
            max_msg_size: self.config.max_msg_size,
            idle_timeout_secs: self.config.idle_timeout_secs,
            max_connections: self.config.max_connections,
            peers_total,
            peers_up,
            active_routes,
            allowlist_enforced,
            instances,
            buckets,
            peers: visible_peers,
            top_origins,
            top_peer_asns,
            last_error,
        }
    }
}

fn message_type_matches(m: &MessageView, ty: &str) -> bool {
    match ty {
        "all" | "" => true,
        "peer" => m.msg_type == "peer_up" || m.msg_type == "peer_down",
        "errors" => m.error.is_some(),
        "updates" => m.msg_type == "route_monitoring",
        other => m.msg_type == other,
    }
}

fn message_matches(m: &MessageView, needle: &str) -> bool {
    if let Some(peer) = &m.peer {
        if peer.ip.contains(needle) || peer.asn.to_string().contains(needle) {
            return true;
        }
    }
    if m.router_addr.contains(needle) {
        return true;
    }
    if let Some(name) = &m.router_name {
        if name.to_ascii_lowercase().contains(needle) {
            return true;
        }
    }
    if let Some(err) = &m.error {
        if err.to_ascii_lowercase().contains(needle) {
            return true;
        }
    }
    if let Some(bgp) = &m.bgp_type {
        if bgp.to_ascii_lowercase().contains(needle) {
            return true;
        }
    }
    for route in &m.routes {
        if route.prefix.to_ascii_lowercase().contains(needle)
            || route.as_path_str.to_ascii_lowercase().contains(needle)
            || route
                .origin_asns
                .iter()
                .any(|a| a.to_string().contains(needle))
            || route
                .communities
                .iter()
                .any(|c| c.to_ascii_lowercase().contains(needle))
        {
            return true;
        }
    }
    for stat in &m.stats {
        if stat.name.to_ascii_lowercase().contains(needle) {
            return true;
        }
    }
    false
}

fn route_matches(r: &RouteEntry, needle: &str) -> bool {
    r.prefix.to_ascii_lowercase().contains(needle)
        || r.as_path_str.to_ascii_lowercase().contains(needle)
        || r.peer_ip.contains(needle)
        || r.peer_asn.to_string().contains(needle)
        || r.origin_asns.iter().any(|a| a.to_string().contains(needle))
        || r.origin
            .as_deref()
            .map(|o| o.to_ascii_lowercase().contains(needle))
            .unwrap_or(false)
        || r.communities
            .iter()
            .any(|c| c.to_ascii_lowercase().contains(needle))
        || r.next_hop
            .as_deref()
            .map(|n| n.contains(needle))
            .unwrap_or(false)
}

fn decrement_peer_prefix(inner: &mut Inner, peer_key: &str) {
    if let Some(peer) = inner.peers.get_mut(peer_key) {
        peer.view.prefixes = peer.view.prefixes.saturating_sub(1);
    }
}

fn rebuild_route_order(inner: &mut Inner) {
    inner.route_order.clear();
    if inner.limits.max_routes == 0 {
        return;
    }
    let mut entries: Vec<(f64, (String, String))> = inner
        .routes
        .iter()
        .map(|(key, route)| (route.first_seen, key.clone()))
        .collect();
    entries.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(Ordering::Equal));
    inner
        .route_order
        .extend(entries.into_iter().map(|(_, key)| key));
}

fn trim_state(inner: &mut Inner) {
    let limits = inner.limits;

    if limits.max_messages > 0 {
        while inner.messages.len() > limits.max_messages {
            inner.messages.pop_front();
        }
    }

    if limits.max_routes == 0 {
        inner.route_order.clear();
    } else {
        // Withdrawals leave lazy tombstones in route_order. Compact before the
        // tombstone queue grows beyond a small bounded overhead.
        if inner.route_order.len() > inner.routes.len().saturating_add(1_024) {
            rebuild_route_order(inner);
        }
        if inner.route_order.is_empty() && !inner.routes.is_empty() {
            rebuild_route_order(inner);
        }
        while inner.routes.len() > limits.max_routes {
            let Some(key) = inner.route_order.pop_front() else {
                break;
            };
            if inner.routes.remove(&key).is_some() {
                decrement_peer_prefix(inner, &key.0);
            }
        }
    }

    if limits.max_peers > 0 {
        while inner.peers.len() > limits.max_peers {
            let oldest = inner
                .peers
                .iter()
                .min_by(|a, b| {
                    a.1.view
                        .last_seen
                        .partial_cmp(&b.1.view.last_seen)
                        .unwrap_or(Ordering::Equal)
                })
                .map(|(key, _)| key.clone());
            let Some(peer_key) = oldest else {
                break;
            };
            inner.routes.retain(|(pk, _), _| pk != &peer_key);
            inner.peers.remove(&peer_key);
        }
    }

    // Peer eviction removes routes; enforce the route cap afterwards if both
    // limits are active.
    if limits.max_routes > 0 && inner.routes.len() > limits.max_routes {
        rebuild_route_order(inner);
        while inner.routes.len() > limits.max_routes {
            let Some(key) = inner.route_order.pop_front() else {
                break;
            };
            if inner.routes.remove(&key).is_some() {
                decrement_peer_prefix(inner, &key.0);
            }
        }
    }
}

fn apply_rib_update(inner: &mut Inner, peer_key: &str, elem: &BgpElem, ts: f64) {
    let prefix_key = match elem.prefix.path_id {
        Some(id) => format!("{}#{}", elem.prefix, id),
        None => elem.prefix.to_string(),
    };
    let key = (peer_key.to_string(), prefix_key);
    if elem.elem_type == ElemType::ANNOUNCE {
        let existing = inner.routes.get(&key);
        let first_seen = existing.map(|e| e.first_seen).unwrap_or(ts);
        let updates = existing.map(|e| e.updates + 1).unwrap_or(1);
        let entry = route_entry(elem, peer_key, first_seen, ts, updates);
        let is_new = existing.is_none();
        inner.routes.insert(key.clone(), entry);
        if is_new {
            if inner.limits.max_routes > 0 {
                inner.route_order.push_back(key.clone());
            }
            if let Some(peer) = inner.peers.get_mut(peer_key) {
                peer.view.prefixes += 1;
            }
        }
    } else if inner.routes.remove(&key).is_some() {
        if let Some(peer) = inner.peers.get_mut(peer_key) {
            peer.view.prefixes = peer.view.prefixes.saturating_sub(1);
        }
    }
}

fn remove_peer_routes(inner: &mut Inner, peer_key: &str) {
    inner.routes.retain(|(pk, _), _| pk != peer_key);
    if let Some(peer) = inner.peers.get_mut(peer_key) {
        peer.view.prefixes = 0;
    }
}

fn build_message(
    inner: &mut Inner,
    conn_id: u64,
    router_ip: &str,
    msg: BmpMessage,
    now: f64,
    size: usize,
    raw_hex: String,
    raw_truncated: bool,
) -> MessageView {
    let header = msg.common_header;
    let type_name = msg_type_name(header.msg_type).to_string();
    let type_code = msg_type_code(header.msg_type);
    let router_name = inner
        .connections
        .get(&conn_id)
        .and_then(|c| c.router_name.clone());

    inner.next_message_id += 1;
    let mut view = MessageView {
        id: inner.next_message_id,
        ts: now,
        received_at: now,
        msg_type: type_name,
        msg_type_code: type_code,
        size,
        router_addr: router_ip.to_string(),
        router_name,
        peer: None,
        bgp_type: None,
        announced: 0,
        withdrawn: 0,
        routes_total: 0,
        routes_truncated: 0,
        routes: Vec::new(),
        stats: Vec::new(),
        peer_down_reason: None,
        peer_down_detail: None,
        peer_up: None,
        tlvs: Vec::new(),
        termination_reason: None,
        error: None,
        raw_hex,
        raw_truncated,
    };

    // Per-peer header common to most BMP message types.
    let mut peer_key_val: Option<String> = None;
    if let Some(h) = &msg.per_peer_header {
        let ts = if h.timestamp > 0.0 { h.timestamp } else { now };
        view.ts = ts;
        let key = peer_key(h);
        let peer = inner.peers.entry(key.clone()).or_insert_with(|| PeerState {
            view: peer_view_from_header(h, now),
        });
        peer.view.router = Some(router_ip.to_string());
        peer.view.last_seen = peer.view.last_seen.max(ts);
        peer.view.messages += 1;
        view.peer = Some(peer_ref_from_view(&peer.view));
        peer_key_val = Some(key);
    }

    match msg.message_body {
        BmpMessageBody::RouteMonitoring(rm) => {
            inner.totals.route_monitoring += 1;
            let peer_header = msg.per_peer_header;
            match &rm.bgp_message {
                BgpMessage::Update(update) => {
                    view.bgp_type = Some(if rm.is_end_of_rib() {
                        "UPDATE · End-of-RIB".to_string()
                    } else {
                        "UPDATE".to_string()
                    });
                    if let (Some(h), Some(pk)) = (peer_header, peer_key_val.as_deref()) {
                        let elems = Elementor::bgp_update_to_elems(
                            update.clone(),
                            view.ts,
                            &h.peer_ip,
                            &h.peer_asn,
                        );
                        let mut announced = 0usize;
                        let mut withdrawn = 0usize;
                        view.routes_total = elems.len();
                        for elem in &elems {
                            if elem.elem_type == ElemType::ANNOUNCE {
                                announced += 1;
                                if let Some(origin) =
                                    elem.origin_asns.as_ref().and_then(|v| v.last())
                                {
                                    *inner.top_origins.entry(origin.to_u32()).or_insert(0) += 1;
                                }
                            } else {
                                withdrawn += 1;
                            }
                            if view.routes.len() < MAX_ROUTE_VIEWS {
                                view.routes.push(route_view(elem));
                            }
                            apply_rib_update(inner, pk, elem, view.ts);
                        }
                        view.announced = announced;
                        view.withdrawn = withdrawn;
                        view.routes_truncated = view.routes_total.saturating_sub(view.routes.len());
                        inner.totals.announcements += announced as u64;
                        inner.totals.withdrawals += withdrawn as u64;
                        if let Some(peer) = inner.peers.get_mut(pk) {
                            peer.view.announcements += announced as u64;
                            peer.view.withdrawals += withdrawn as u64;
                        }
                    }
                }
                other => {
                    view.bgp_type = Some(bgp_type_name(other).to_string());
                }
            }
        }
        BmpMessageBody::PeerUpNotification(up) => {
            inner.totals.peer_up += 1;
            if let Some(pk) = peer_key_val.as_deref() {
                if let Some(peer) = inner.peers.get_mut(pk) {
                    peer.view.session = "up".to_string();
                    peer.view.session_since = Some(view.ts);
                    peer.view.local_addr = Some(up.local_addr.to_string());
                    peer.view.local_port = Some(up.local_port);
                    peer.view.remote_port = Some(up.remote_port);
                    peer.view.last_down_reason = None;
                }
            }
            view.peer_up = Some(PeerUpView {
                local_addr: up.local_addr.to_string(),
                local_port: up.local_port,
                remote_port: up.remote_port,
                sent_open: open_info(&up.sent_open),
                received_open: open_info(&up.received_open),
            });
            view.tlvs = peer_up_tlv_views(&up.tlvs);
        }
        BmpMessageBody::PeerDownNotification(down) => {
            inner.totals.peer_down += 1;
            let reason = peer_down_reason_text(down.reason);
            view.peer_down_reason = Some(reason.clone());
            view.peer_down_detail = peer_down_detail(&down);
            if let Some(pk) = peer_key_val.as_deref() {
                remove_peer_routes(inner, pk);
                if let Some(peer) = inner.peers.get_mut(pk) {
                    peer.view.session = "down".to_string();
                    peer.view.session_since = None;
                    peer.view.last_down_reason = Some(reason);
                }
            }
        }
        BmpMessageBody::StatsReport(stats) => {
            inner.totals.statistics_report += 1;
            let views: Vec<StatView> = stats.counters.iter().map(stat_view).collect();
            view.stats = views.clone();
            if let Some(pk) = peer_key_val.as_deref() {
                if let Some(peer) = inner.peers.get_mut(pk) {
                    peer.view.stats = views;
                }
            }
        }
        BmpMessageBody::InitiationMessage(init) => {
            inner.totals.initiation += 1;
            view.tlvs = initiation_tlv_views(&init.tlvs);
            if let Some(name) = router_name_from_initiation(&init.tlvs) {
                if let Some(conn) = inner.connections.get_mut(&conn_id) {
                    conn.router_name = Some(name.clone());
                }
                view.router_name = Some(name);
            }
            // Only relevant for InitiationMessage.
            if let Some(sys_descr) = init
                .tlvs
                .iter()
                .find(|t| matches!(t.info_type, InitiationTlvType::SysDescr))
            {
                tracing::info!(
                    conn_id,
                    router_ip,
                    sys_descr = sys_descr.info.as_str(),
                    "BMP initiation received"
                );
            }
        }
        BmpMessageBody::TerminationMessage(term) => {
            inner.totals.termination += 1;
            let (tlvs, reason) = termination_tlv_views(&term.tlvs);
            view.tlvs = tlvs;
            view.termination_reason = reason;
        }
        BmpMessageBody::RouteMirroring(mirror) => {
            inner.totals.route_mirroring += 1;
            view.bgp_type = Some(format!("{} TLV(s)", mirror.tlvs.len()));
        }
    }

    // Refresh the peer snapshot so PeerDown/PeerUp messages show the new session state.
    if let Some(key) = peer_key_val.as_deref() {
        if let Some(peer) = inner.peers.get(key) {
            view.peer = Some(peer_ref_from_view(&peer.view));
        }
    }
    view.routes_truncated = view.routes_total.saturating_sub(view.routes.len());
    view
}
