//! In-memory runtime state, message processing and aggregation.

use std::cmp::Ordering;
use std::collections::{BTreeMap, HashMap, VecDeque};

use ahash::{AHashMap, AHashSet};
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
    route_peer: Arc<RoutePeer>,
}

#[derive(Debug, Hash, PartialEq, Eq)]
struct RoutePeer {
    key: Arc<str>,
    ip: Arc<str>,
    asn: u32,
}

#[derive(Debug, Clone, Hash, PartialEq, Eq)]
struct RouteKey {
    peer: Arc<RoutePeer>,
    prefix: Arc<str>,
    path_id: Option<u32>,
}

struct RouteRecord {
    next_hop: Option<Arc<str>>,
    as_path: Arc<[u32]>,
    as_path_str: Arc<str>,
    origin_asns: Arc<[u32]>,
    origin: Option<Arc<str>>,
    med: Option<u32>,
    local_pref: Option<u32>,
    communities: Arc<[Arc<str>]>,
    first_seen: f64,
    updated_at: f64,
    updates: u64,
    first_seq: u64,
    order_key: (u64, u64),
}

#[derive(Default)]
struct StringInterner {
    values: AHashSet<Arc<str>>,
}

impl StringInterner {
    fn intern(&mut self, value: &str) -> Arc<str> {
        // The interner is an optimization cache, not route storage. Bound it
        // so pathological churn cannot retain an unbounded set of values.
        if self.values.len() >= 262_144 {
            self.values.clear();
        }
        if let Some(existing) = self.values.get(value) {
            return existing.clone();
        }
        let interned: Arc<str> = Arc::from(value);
        self.values.insert(interned.clone());
        interned
    }
}

#[derive(Default)]
struct U32Interner {
    values: AHashSet<Arc<[u32]>>,
}

impl U32Interner {
    fn intern(&mut self, values: &[u32]) -> Arc<[u32]> {
        if self.values.len() >= 262_144 {
            self.values.clear();
        }
        if let Some(existing) = self.values.get(values) {
            return existing.clone();
        }
        let interned: Arc<[u32]> = Arc::from(values);
        self.values.insert(interned.clone());
        interned
    }
}

#[derive(Default)]
struct CommunityInterner {
    values: AHashSet<Arc<[Arc<str>]>>,
}

impl CommunityInterner {
    fn intern(&mut self, values: Vec<Arc<str>>) -> Arc<[Arc<str>]> {
        if self.values.len() >= 65_536 {
            self.values.clear();
        }
        if let Some(existing) = self.values.get(values.as_slice()) {
            return existing.clone();
        }
        let interned: Arc<[Arc<str>]> = Arc::from(values);
        self.values.insert(interned.clone());
        interned
    }
}

#[derive(Default)]
struct Inner {
    next_message_id: u64,
    next_conn_id: u64,
    connections_total: u64,
    totals: Totals,
    messages: VecDeque<Arc<MessageView>>,
    peers: HashMap<String, PeerState>,
    routes: AHashMap<RouteKey, RouteRecord>,
    /// Newest-update order. Values are active route keys only.
    route_order: BTreeMap<(u64, u64), RouteKey>,
    /// Creation order used only when max_routes is configured.
    cap_order: BTreeMap<u64, RouteKey>,
    next_route_seq: u64,
    limits: RuntimeConfig,
    string_interner: StringInterner,
    u32_interner: U32Interner,
    community_interner: CommunityInterner,
    message_reset_at: f64,
    buckets: BTreeMap<u64, BucketCounts>,
    top_origins: AHashMap<u32, u64>,
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
        let started_at = now_secs();
        let message_reset_at = if limits.message_reset_secs > 0 {
            started_at + limits.message_reset_secs as f64
        } else {
            0.0
        };
        Self {
            config,
            admin,
            runtime,
            demo: Mutex::new(None),
            demo_seq: AtomicU64::new(0),
            started_at,
            started_instant: Instant::now(),
            overview_seq: AtomicU64::new(0),
            inner: Mutex::new(Inner {
                limits,
                message_reset_at,
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
        let now = now_secs();
        let mut inner = self.lock_inner();
        let previous = inner.limits;
        inner.limits = next;
        if next.message_reset_secs == 0 {
            inner.message_reset_at = 0.0;
        } else if previous.message_reset_secs != next.message_reset_secs
            || inner.message_reset_at <= 0.0
        {
            inner.message_reset_at = now + next.message_reset_secs as f64;
        }
        if next.max_routes == 0 {
            inner.cap_order.clear();
        } else if previous.max_routes == 0 || inner.cap_order.is_empty() {
            rebuild_cap_order(&mut inner);
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
        maybe_reset_message_totals(&mut inner, now);

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
        maybe_reset_message_totals(&mut inner, now);
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
        let peer_filter = peer.map(|s| s.trim()).filter(|s| !s.is_empty());
        let peer_asn_filter = peer_filter.and_then(|value| value.parse::<u32>().ok());
        let router_filter = router.map(|s| s.trim()).filter(|s| !s.is_empty());
        let limit = limit.max(1);
        let requested_page = page.max(1);
        let start = (requested_page - 1) * limit;
        let inner = self.lock_inner();

        let matches = |key: &RouteKey, record: &RouteRecord| {
            route_matches_filters(
                &inner,
                key,
                record,
                needle.as_deref(),
                peer_filter,
                peer_asn_filter,
                router_filter,
            )
        };

        let no_filter = needle.is_none() && peer_filter.is_none() && router_filter.is_none();
        let mut total = if no_filter { inner.routes.len() } else { 0 };
        let mut routes: Vec<RouteEntry> = Vec::with_capacity(limit);

        if no_filter {
            for key in inner.route_order.values().rev().skip(start).take(limit) {
                if let Some(record) = inner.routes.get(key) {
                    routes.push(to_route_entry(key, record));
                }
            }
        } else {
            // A filtered/search query must visit all active records to produce
            // the correct total count. It does not allocate or sort the whole
            // RIB; only the requested page is materialized.
            for key in inner.route_order.values().rev() {
                let Some(record) = inner.routes.get(key) else {
                    continue;
                };
                if matches(key, record) {
                    if total >= start && routes.len() < limit {
                        routes.push(to_route_entry(key, record));
                    }
                    total += 1;
                }
            }
        }

        let total_pages = total.div_ceil(limit).max(1);
        let page = requested_page.min(total_pages);

        // If the requested page was beyond the end, re-scan for the clamped
        // last page. This path is rare and avoids retaining all matches.
        if page != requested_page {
            let page_start = (page - 1) * limit;
            routes.clear();
            if no_filter {
                for key in inner
                    .route_order
                    .values()
                    .rev()
                    .skip(page_start)
                    .take(limit)
                {
                    if let Some(record) = inner.routes.get(key) {
                        routes.push(to_route_entry(key, record));
                    }
                }
            } else {
                let mut seen = 0usize;
                for key in inner.route_order.values().rev() {
                    let Some(record) = inner.routes.get(key) else {
                        continue;
                    };
                    if matches(key, record) {
                        if seen >= page_start && routes.len() < limit {
                            routes.push(to_route_entry(key, record));
                        }
                        seen += 1;
                    }
                }
            }
        }

        let returned = routes.len();
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
        maybe_reset_message_totals(&mut inner, now);

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

fn route_matches_text(key: &RouteKey, record: &RouteRecord, needle: &str) -> bool {
    key.prefix.contains(needle)
        || record.as_path_str.contains(needle)
        || key.peer.ip.contains(needle)
        || key.peer.asn.to_string().contains(needle)
        || record
            .origin_asns
            .iter()
            .any(|a| a.to_string().contains(needle))
        || record
            .origin
            .as_deref()
            .map(|o| o.contains(needle))
            .unwrap_or(false)
        || record
            .communities
            .iter()
            .any(|c| c.to_ascii_lowercase().contains(needle))
        || record
            .next_hop
            .as_deref()
            .map(|n| n.contains(needle))
            .unwrap_or(false)
}

fn route_matches_filters(
    inner: &Inner,
    key: &RouteKey,
    record: &RouteRecord,
    needle: Option<&str>,
    peer: Option<&str>,
    peer_asn: Option<u32>,
    router: Option<&str>,
) -> bool {
    if let Some(peer_filter) = peer {
        if key.peer.ip.as_ref() != peer_filter
            && key.peer.key.as_ref() != peer_filter
            && peer_asn != Some(key.peer.asn)
        {
            return false;
        }
    }
    if let Some(router_filter) = router {
        let matches_router = inner
            .peers
            .get(key.peer.key.as_ref())
            .and_then(|p| p.view.router.as_deref())
            == Some(router_filter);
        if !matches_router {
            return false;
        }
    }
    if let Some(needle) = needle {
        return route_matches_text(key, record, needle);
    }
    true
}

fn to_route_entry(key: &RouteKey, record: &RouteRecord) -> RouteEntry {
    RouteEntry {
        prefix: key.prefix.to_string(),
        path_id: key.path_id,
        peer_key: key.peer.key.to_string(),
        peer_ip: key.peer.ip.to_string(),
        peer_asn: key.peer.asn,
        next_hop: record.next_hop.as_deref().map(str::to_string),
        as_path: record.as_path.to_vec(),
        as_path_str: record.as_path_str.to_string(),
        origin_asns: record.origin_asns.to_vec(),
        origin: record.origin.as_deref().map(str::to_string),
        med: record.med,
        local_pref: record.local_pref,
        communities: record
            .communities
            .iter()
            .map(|community| community.to_string())
            .collect(),
        first_seen: record.first_seen,
        updated_at: record.updated_at,
        updates: record.updates,
    }
}

fn route_order_key(updated_at: f64, seq: u64) -> (u64, u64) {
    let micros = if updated_at.is_finite() && updated_at > 0.0 {
        (updated_at * 1_000_000.0).min(u64::MAX as f64) as u64
    } else {
        0
    };
    (micros, seq)
}

fn decrement_peer_prefix(inner: &mut Inner, peer_key: &str) {
    if let Some(peer) = inner.peers.get_mut(peer_key) {
        peer.view.prefixes = peer.view.prefixes.saturating_sub(1);
    }
}

fn remove_route(inner: &mut Inner, key: &RouteKey) -> bool {
    let Some(record) = inner.routes.remove(key) else {
        return false;
    };
    inner.route_order.remove(&record.order_key);
    inner.cap_order.remove(&record.first_seq);
    decrement_peer_prefix(inner, key.peer.key.as_ref());
    true
}

fn remove_peer_routes(inner: &mut Inner, peer_key: &str) {
    let keys: Vec<RouteKey> = inner
        .routes
        .keys()
        .filter(|key| key.peer.key.as_ref() == peer_key)
        .cloned()
        .collect();
    for key in keys {
        remove_route(inner, &key);
    }
    if let Some(peer) = inner.peers.get_mut(peer_key) {
        peer.view.prefixes = 0;
    }
}

fn make_route_record(
    inner: &mut Inner,
    elem: &BgpElem,
    first_seen: f64,
    updated_at: f64,
    updates: u64,
) -> RouteRecord {
    let as_path_vec = elem
        .as_path
        .as_ref()
        .map(flatten_as_path)
        .unwrap_or_default();
    let as_path: Arc<[u32]> = inner.u32_interner.intern(&as_path_vec);

    let as_path_str: Arc<str> = elem
        .as_path
        .as_ref()
        .map(|path| inner.string_interner.intern(&path.to_string()))
        .unwrap_or_else(|| inner.string_interner.intern(""));

    let origin_asns_vec: Vec<u32> = elem
        .origin_asns
        .as_ref()
        .map(|asns| asns.iter().map(|a| a.to_u32()).collect())
        .unwrap_or_default();
    let origin_asns: Arc<[u32]> = inner.u32_interner.intern(&origin_asns_vec);

    let origin = elem.origin.as_ref().map(|origin| {
        inner
            .string_interner
            .intern(&origin.to_string().to_ascii_lowercase())
    });

    let next_hop = elem
        .next_hop
        .map(|ip| inner.string_interner.intern(&ip.to_string()));

    let mut community_list: Vec<Arc<str>> = Vec::new();
    if let Some(communities) = &elem.communities {
        community_list.reserve(communities.len());
        for community in communities {
            let value = community.to_string();
            community_list.push(inner.string_interner.intern(&value));
        }
    }
    let communities = inner.community_interner.intern(community_list);

    RouteRecord {
        next_hop,
        as_path,
        as_path_str,
        origin_asns,
        origin,
        med: elem.med,
        local_pref: elem.local_pref,
        communities,
        first_seen,
        updated_at,
        updates,
        first_seq: 0,
        order_key: (0, 0),
    }
}

fn apply_rib_update(inner: &mut Inner, peer: Arc<RoutePeer>, elem: &BgpElem, ts: f64) {
    let prefix: Arc<str> = Arc::from(elem.prefix.to_string());
    let key = RouteKey {
        peer,
        prefix,
        path_id: elem.prefix.path_id,
    };

    if elem.elem_type == ElemType::ANNOUNCE {
        let existing = inner.routes.get(&key);
        let first_seen = existing.map(|e| e.first_seen).unwrap_or(ts);
        let updates = existing.map(|e| e.updates + 1).unwrap_or(1);
        let old_order = existing.map(|e| e.order_key);
        let existing_first_seq = existing.map(|e| e.first_seq);
        let is_new = existing.is_none();

        if let Some(order) = old_order {
            inner.route_order.remove(&order);
        }

        inner.next_route_seq = inner.next_route_seq.wrapping_add(1);
        let seq = inner.next_route_seq;
        let order_key = route_order_key(ts, seq);
        let first_seq = existing_first_seq.unwrap_or(seq);
        let mut record = make_route_record(inner, elem, first_seen, ts, updates);
        record.order_key = order_key;
        record.first_seq = first_seq;

        inner.routes.insert(key.clone(), record);
        inner.route_order.insert(order_key, key.clone());
        if is_new {
            if inner.limits.max_routes > 0 {
                inner.cap_order.insert(first_seq, key.clone());
            }
            if let Some(peer) = inner.peers.get_mut(key.peer.key.as_ref()) {
                peer.view.prefixes += 1;
            }
        }
    } else if let Some(record) = inner.routes.remove(&key) {
        inner.route_order.remove(&record.order_key);
        decrement_peer_prefix(inner, key.peer.key.as_ref());
    }
}

fn rebuild_cap_order(inner: &mut Inner) {
    inner.cap_order.clear();
    if inner.limits.max_routes == 0 {
        return;
    }
    inner.cap_order.extend(
        inner
            .routes
            .iter()
            .map(|(key, record)| (record.first_seq, key.clone())),
    );
}

fn reset_message_totals(inner: &mut Inner) {
    // Only reset the global message counters. Rate buckets, active routes,
    // peer sessions, instance state and connection totals are intentionally
    // left untouched.
    inner.totals.messages = 0;
    inner.totals.route_monitoring = 0;
    inner.totals.statistics_report = 0;
    inner.totals.peer_up = 0;
    inner.totals.peer_down = 0;
    inner.totals.initiation = 0;
    inner.totals.termination = 0;
    inner.totals.route_mirroring = 0;
    inner.totals.announcements = 0;
    inner.totals.withdrawals = 0;
    inner.totals.parse_errors = 0;
    inner.totals.bytes = 0;
}

fn maybe_reset_message_totals(inner: &mut Inner, now: f64) {
    let reset_secs = inner.limits.message_reset_secs;
    if reset_secs == 0 {
        inner.message_reset_at = 0.0;
        return;
    }
    if inner.message_reset_at <= 0.0 {
        inner.message_reset_at = now + reset_secs as f64;
        return;
    }
    if now < inner.message_reset_at {
        return;
    }

    reset_message_totals(inner);
    // Skip missed periods so a suspended/slow process does not spend time
    // repeatedly resetting in a loop.
    let elapsed = (now - inner.message_reset_at).max(0.0);
    let periods = (elapsed / reset_secs as f64).floor() as u64 + 1;
    inner.message_reset_at += periods as f64 * reset_secs as f64;
}

fn trim_state(inner: &mut Inner) {
    let limits = inner.limits;

    if limits.max_messages > 0 {
        while inner.messages.len() > limits.max_messages {
            inner.messages.pop_front();
        }
    }

    if limits.max_routes > 0 {
        if inner.cap_order.is_empty() && !inner.routes.is_empty() {
            rebuild_cap_order(inner);
        }
        while inner.routes.len() > limits.max_routes {
            let Some((_, key)) = inner.cap_order.pop_first() else {
                break;
            };
            remove_route(inner, &key);
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
            remove_peer_routes(inner, &peer_key);
            inner.peers.remove(&peer_key);
        }
    }

    // HashMap keeps its high-water capacity after mass withdrawals/evictions.
    // Release that memory when the active RIB has shrunk substantially.
    if inner.routes.capacity() > 8_192
        && inner.routes.len().saturating_mul(4) < inner.routes.capacity()
    {
        inner.routes.shrink_to_fit();
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
    let mut peer_meta_val: Option<Arc<RoutePeer>> = None;
    if let Some(h) = &msg.per_peer_header {
        let ts = if h.timestamp > 0.0 { h.timestamp } else { now };
        view.ts = ts;
        let key = peer_key(h);
        let peer = inner.peers.entry(key.clone()).or_insert_with(|| {
            let route_peer = Arc::new(RoutePeer {
                key: Arc::from(key.as_str()),
                ip: Arc::from(h.peer_ip.to_string()),
                asn: h.peer_asn.to_u32(),
            });
            PeerState {
                view: peer_view_from_header(h, now),
                route_peer,
            }
        });
        peer.view.router = Some(router_ip.to_string());
        peer.view.last_seen = peer.view.last_seen.max(ts);
        peer.view.messages += 1;
        view.peer = Some(peer_ref_from_view(&peer.view));
        peer_meta_val = Some(peer.route_peer.clone());
        peer_key_val = Some(key);
    }

    match msg.message_body {
        BmpMessageBody::RouteMonitoring(rm) => {
            inner.totals.route_monitoring += 1;
            let peer_header = msg.per_peer_header;
            let end_of_rib = rm.is_end_of_rib();
            // Consume the UPDATE instead of cloning it. A large BMP UPDATE may
            // carry thousands of prefixes, so this avoids a second deep copy
            // of the BGP payload on the ingest hot path.
            match rm.bgp_message {
                BgpMessage::Update(update) => {
                    view.bgp_type = Some(if end_of_rib {
                        "UPDATE · End-of-RIB".to_string()
                    } else {
                        "UPDATE".to_string()
                    });
                    if let (Some(h), Some(peer_meta)) = (peer_header, peer_meta_val.as_ref()) {
                        let elems = Elementor::bgp_update_to_elems(
                            update,
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
                            apply_rib_update(inner, peer_meta.clone(), elem, view.ts);
                        }
                        view.announced = announced;
                        view.withdrawn = withdrawn;
                        view.routes_truncated = view.routes_total.saturating_sub(view.routes.len());
                        inner.totals.announcements += announced as u64;
                        inner.totals.withdrawals += withdrawn as u64;
                        if let Some(peer) = inner.peers.get_mut(peer_meta.key.as_ref()) {
                            peer.view.announcements += announced as u64;
                            peer.view.withdrawals += withdrawn as u64;
                        }
                    }
                }
                other => {
                    view.bgp_type = Some(bgp_type_name(&other).to_string());
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
