//! Serializable view models and conversions from `bgpkit-parser` types.

use bgpkit_parser::models::{AsPath, AsnLength, BgpElem, BgpMessage, ElemType, ParamValue};
use bgpkit_parser::parser::bmp::messages::{
    BmpMsgType, BmpPeerType, BmpPerPeerHeader, InitiationTlv, InitiationTlvType,
    PeerDownNotification, PeerDownReason, PeerUpNotificationTlv, PerPeerFlags, StatCounter,
    StatType, StatsData, TerminationReason, TerminationTlvValue,
};
use serde::Serialize;

/// A peer as referenced by a single BMP message.
#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct PeerRef {
    pub key: String,
    pub ip: String,
    pub asn: u32,
    pub bgp_id: String,
    pub session: String,
    pub post_policy: bool,
    pub adj_rib_out: bool,
    pub ipv6: bool,
    pub asn_16bit: bool,
}

/// Accumulated state for a monitored BGP peer.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PeerView {
    pub key: String,
    pub ip: String,
    pub asn: u32,
    pub bgp_id: String,
    pub peer_type: String,
    pub distinguisher: u64,
    pub post_policy: bool,
    pub adj_rib_out: bool,
    pub ipv6: bool,
    pub asn_16bit: bool,
    /// `up`, `down` or `unknown`
    pub session: String,
    pub session_since: Option<f64>,
    pub first_seen: f64,
    pub last_seen: f64,
    /// Address of the BMP router that most recently reported this peer.
    pub router: Option<String>,
    pub local_addr: Option<String>,
    pub local_port: Option<u16>,
    pub remote_port: Option<u16>,
    pub messages: u64,
    pub announcements: u64,
    pub withdrawals: u64,
    /// Number of routes currently present in the locally reconstructed Adj-RIB-In.
    pub prefixes: u64,
    pub last_down_reason: Option<String>,
    pub stats: Vec<StatView>,
}

/// A single route carried by a BMP route-monitoring message.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RouteView {
    pub prefix: String,
    pub action: String,
    pub next_hop: Option<String>,
    pub as_path: Vec<u32>,
    pub as_path_str: String,
    pub origin_asns: Vec<u32>,
    pub origin: Option<String>,
    pub med: Option<u32>,
    pub local_pref: Option<u32>,
    pub communities: Vec<String>,
    pub path_id: Option<u32>,
}

/// A route in the reconstructed active RIB.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RouteEntry {
    pub prefix: String,
    pub path_id: Option<u32>,
    pub peer_key: String,
    pub peer_ip: String,
    pub peer_asn: u32,
    pub next_hop: Option<String>,
    pub as_path: Vec<u32>,
    pub as_path_str: String,
    pub origin_asns: Vec<u32>,
    pub origin: Option<String>,
    pub med: Option<u32>,
    pub local_pref: Option<u32>,
    pub communities: Vec<String>,
    pub first_seen: f64,
    pub updated_at: f64,
    pub updates: u64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StatView {
    pub code: u16,
    pub name: String,
    pub value: String,
    pub kind: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TlvView {
    pub ty: u16,
    pub name: String,
    pub value: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OpenInfo {
    pub version: u8,
    pub asn: u32,
    pub hold_time: u16,
    pub bgp_id: String,
    pub capabilities: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PeerUpView {
    pub local_addr: String,
    pub local_port: u16,
    pub remote_port: u16,
    pub sent_open: Option<OpenInfo>,
    pub received_open: Option<OpenInfo>,
}

/// One BMP message, ready for JSON and the live feed.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MessageView {
    pub id: u64,
    /// BMP per-peer timestamp when available, otherwise receive time (epoch seconds).
    pub ts: f64,
    pub received_at: f64,
    pub msg_type: String,
    pub msg_type_code: u8,
    pub size: usize,
    pub router_addr: String,
    pub router_name: Option<String>,
    pub peer: Option<PeerRef>,
    pub bgp_type: Option<String>,
    pub announced: usize,
    pub withdrawn: usize,
    pub routes_total: usize,
    pub routes_truncated: usize,
    pub routes: Vec<RouteView>,
    pub stats: Vec<StatView>,
    pub peer_down_reason: Option<String>,
    pub peer_down_detail: Option<String>,
    pub peer_up: Option<PeerUpView>,
    pub tlvs: Vec<TlvView>,
    pub termination_reason: Option<String>,
    pub error: Option<String>,
    pub raw_hex: String,
    pub raw_truncated: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConnectionView {
    pub id: u64,
    pub peer: String,
    /// IP part of the TCP peer address, used to match configured instances.
    pub router_ip: String,
    pub router_name: Option<String>,
    pub since: f64,
    pub last_message_at: f64,
    pub messages: u64,
    pub bytes: u64,
    pub state: String,
    pub close_reason: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BucketView {
    pub t: u64,
    pub messages: u64,
    pub announcements: u64,
    pub withdrawals: u64,
    pub errors: u64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AsnCount {
    pub asn: u32,
    pub count: u64,
}

/// Public status of a configured BIRD / BMP instance.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InstanceStatus {
    pub id: String,
    pub name: String,
    pub router_ip: String,
    pub description: String,
    pub enabled: bool,
    pub online: bool,
    pub active_connections: usize,
    pub peers: usize,
    pub messages: u64,
    pub bytes: u64,
    pub last_seen: Option<f64>,
    pub created_at: f64,
    pub updated_at: f64,
}

#[derive(Debug, Clone, Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct DemoStatus {
    pub running: bool,
    pub started_at: Option<f64>,
    pub sent: u64,
}

pub fn msg_type_name(t: BmpMsgType) -> &'static str {
    match t {
        BmpMsgType::RouteMonitoring => "route_monitoring",
        BmpMsgType::StatisticsReport => "statistics_report",
        BmpMsgType::PeerDownNotification => "peer_down",
        BmpMsgType::PeerUpNotification => "peer_up",
        BmpMsgType::InitiationMessage => "initiation",
        BmpMsgType::TerminationMessage => "termination",
        BmpMsgType::RouteMirroringMessage => "route_mirroring",
    }
}

pub fn msg_type_code(t: BmpMsgType) -> u8 {
    u8::from(t)
}

pub fn peer_type_name(t: BmpPeerType) -> &'static str {
    match t {
        BmpPeerType::Global => "global",
        BmpPeerType::RD => "rd",
        BmpPeerType::Local => "local",
        BmpPeerType::LocalRib => "local_rib",
    }
}

pub fn bgp_type_name(msg: &BgpMessage) -> &'static str {
    match msg {
        BgpMessage::Open(_) => "OPEN",
        BgpMessage::Update(_) => "UPDATE",
        BgpMessage::Notification(_) => "NOTIFICATION",
        BgpMessage::KeepAlive => "KEEPALIVE",
        BgpMessage::RouteRefresh(_) => "ROUTE_REFRESH",
    }
}

pub fn peer_key(h: &BmpPerPeerHeader) -> String {
    format!(
        "{}|{}|{}",
        h.peer_ip,
        h.peer_asn.to_u32(),
        h.peer_distinguisher
    )
}

pub fn peer_ref_from_view(p: &PeerView) -> PeerRef {
    PeerRef {
        key: p.key.clone(),
        ip: p.ip.clone(),
        asn: p.asn,
        bgp_id: p.bgp_id.clone(),
        session: p.session.clone(),
        post_policy: p.post_policy,
        adj_rib_out: p.adj_rib_out,
        ipv6: p.ipv6,
        asn_16bit: p.asn_16bit,
    }
}

pub fn peer_view_from_header(h: &BmpPerPeerHeader, now: f64) -> PeerView {
    let (post_policy, adj_rib_out) = match &h.peer_flags {
        PerPeerFlags::PeerFlags(f) => (f.is_post_policy(), f.is_adj_rib_out()),
        PerPeerFlags::LocalRibPeerFlags(_) => (false, false),
    };
    PeerView {
        key: peer_key(h),
        ip: h.peer_ip.to_string(),
        asn: h.peer_asn.to_u32(),
        bgp_id: h.peer_bgp_id.to_string(),
        peer_type: peer_type_name(h.peer_type).to_string(),
        distinguisher: h.peer_distinguisher,
        post_policy,
        adj_rib_out,
        ipv6: h.peer_ip.is_ipv6(),
        asn_16bit: matches!(h.asn_length(), AsnLength::Bits16),
        session: "unknown".to_string(),
        session_since: None,
        first_seen: now,
        last_seen: now,
        router: None,
        local_addr: None,
        local_port: None,
        remote_port: None,
        messages: 0,
        announcements: 0,
        withdrawals: 0,
        prefixes: 0,
        last_down_reason: None,
        stats: Vec::new(),
    }
}

pub fn flatten_as_path(path: &AsPath) -> Vec<u32> {
    path.iter_segments()
        .flat_map(|seg| seg.iter().map(|asn| asn.to_u32()))
        .collect()
}

pub fn route_view(elem: &BgpElem) -> RouteView {
    let as_path = elem.as_path.as_ref();
    RouteView {
        prefix: elem.prefix.to_string(),
        action: if elem.elem_type == ElemType::ANNOUNCE {
            "announce".to_string()
        } else {
            "withdraw".to_string()
        },
        next_hop: elem.next_hop.map(|ip| ip.to_string()),
        as_path: as_path.map(flatten_as_path).unwrap_or_default(),
        as_path_str: as_path.map(|p| p.to_string()).unwrap_or_default(),
        origin_asns: elem
            .origin_asns
            .as_ref()
            .map(|asns| asns.iter().map(|a| a.to_u32()).collect())
            .unwrap_or_default(),
        origin: elem
            .origin
            .as_ref()
            .map(|origin| origin.to_string().to_ascii_lowercase()),
        med: elem.med,
        local_pref: elem.local_pref,
        communities: elem
            .communities
            .as_ref()
            .map(|cs| cs.iter().map(|c| c.to_string()).collect())
            .unwrap_or_default(),
        path_id: elem.prefix.path_id,
    }
}

pub fn route_entry(
    elem: &BgpElem,
    peer_key: &str,
    first_seen: f64,
    updated_at: f64,
    updates: u64,
) -> RouteEntry {
    let as_path = elem.as_path.as_ref();
    RouteEntry {
        prefix: elem.prefix.to_string(),
        path_id: elem.prefix.path_id,
        peer_key: peer_key.to_string(),
        peer_ip: elem.peer_ip.to_string(),
        peer_asn: elem.peer_asn.to_u32(),
        next_hop: elem.next_hop.map(|ip| ip.to_string()),
        as_path: as_path.map(flatten_as_path).unwrap_or_default(),
        as_path_str: as_path.map(|p| p.to_string()).unwrap_or_default(),
        origin_asns: elem
            .origin_asns
            .as_ref()
            .map(|asns| asns.iter().map(|a| a.to_u32()).collect())
            .unwrap_or_default(),
        origin: elem
            .origin
            .as_ref()
            .map(|origin| origin.to_string().to_ascii_lowercase()),
        med: elem.med,
        local_pref: elem.local_pref,
        communities: elem
            .communities
            .as_ref()
            .map(|cs| cs.iter().map(|c| c.to_string()).collect())
            .unwrap_or_default(),
        first_seen,
        updated_at,
        updates,
    }
}

pub fn open_info(msg: &BgpMessage) -> Option<OpenInfo> {
    let BgpMessage::Open(open) = msg else {
        return None;
    };
    let mut capabilities = Vec::new();
    for param in &open.opt_params {
        match &param.param_value {
            ParamValue::Capacities(caps) => {
                for cap in caps {
                    if capabilities.len() < 16 {
                        capabilities.push(format!("{:?}", cap.ty));
                    }
                }
            }
            ParamValue::Raw(bytes) => capabilities.push(format!("raw({}B)", bytes.len())),
        }
    }
    Some(OpenInfo {
        version: open.version,
        asn: open.asn.to_u32(),
        hold_time: open.hold_time,
        bgp_id: open.bgp_identifier.to_string(),
        capabilities,
    })
}

pub fn peer_down_reason_text(reason: PeerDownReason) -> String {
    let text = match reason {
        PeerDownReason::Reserved => "保留原因",
        PeerDownReason::LocalSystemClosedNotificationPduFollows => {
            "本地系统关闭会话（随后附带 NOTIFICATION）"
        }
        PeerDownReason::LocalSystemClosedFsmEvenFollows => "本地系统关闭会话（FSM 事件）",
        PeerDownReason::RemoteSystemClosedNotificationPduFollows => {
            "远端系统关闭会话（附带 NOTIFICATION）"
        }
        PeerDownReason::RemoteSystemsClosedNoData => "远端系统关闭会话（无数据）",
        PeerDownReason::PeerDeConfigured => "对等体被删除配置",
        PeerDownReason::LocalSystemClosedTlvDataFollows => "本地系统关闭会话（TLV 数据）",
    };
    format!("{text} · code {}", u8::from(reason))
}

pub fn stat_view(counter: &StatCounter) -> StatView {
    let code = u16::from(counter.stat_type);
    let (value, kind) = match &counter.stat_data {
        StatsData::Counter(v) => (v.to_string(), "counter"),
        StatsData::Gauge(v) => (v.to_string(), "gauge"),
        StatsData::AfiSafiGauge(afi, safi, v) => {
            (format!("AFI {afi} / SAFI {safi} · {v}"), "gauge")
        }
        StatsData::Unknown(bytes) => {
            let preview: String = bytes
                .iter()
                .take(24)
                .map(|b| format!("{b:02x}"))
                .collect::<Vec<_>>()
                .join("");
            (format!("0x{preview}"), "unknown")
        }
    };
    StatView {
        code,
        name: stat_name(counter.stat_type),
        value,
        kind: kind.to_string(),
    }
}

fn stat_name(t: StatType) -> String {
    let name = match t {
        StatType::PrefixesRejectedByInboundPolicy => "入站策略拒绝的前缀数",
        StatType::DuplicatePrefixAdvertisements => "重复的通告前缀",
        StatType::DuplicateWithdrawnPrefixes => "重复的撤销前缀",
        StatType::UpdatesInvalidatedDueToClusterListLoop => "因 Cluster List 环路丢弃的更新",
        StatType::UpdatesInvalidatedDueToASPathLoop => "因 AS_PATH 环路丢弃的更新",
        StatType::UpdatesInvalidatedDueToOriginatorId => "因 Originator ID 丢弃的更新",
        StatType::UpdatesInvalidatedDueToASConfedLoop => "因 AS 联盟环路丢弃的更新",
        StatType::RoutesInAdjRibsIn => "Adj-RIB-In 路由数",
        StatType::RoutesInLocRib => "Loc-RIB 路由数",
        StatType::RoutesInPerAfiSafiAdjRibIn => "按 AFI/SAFI 的 Adj-RIB-In 路由数",
        StatType::RoutesInPerAfiSafiLocRib => "按 AFI/SAFI 的 Loc-RIB 路由数",
        StatType::UpdatesSubjectedToTreatAsWithdraw => "按撤销处理的更新数",
        StatType::PrefixesSubjectedToTreatAsWithdraw => "按撤销处理的前缀数",
        StatType::DuplicateUpdateMessagesReceived => "重复的 UPDATE 消息",
        StatType::RoutesInPrePolicyAdjRibOut => "Pre-Policy Adj-RIB-Out 路由数",
        StatType::RoutesInPostPolicyAdjRibOut => "Post-Policy Adj-RIB-Out 路由数",
        StatType::RoutesInPrePolicyAdjRibIn => "Pre-Policy Adj-RIB-In 路由数",
        StatType::RoutesInPostPolicyAdjRibIn => "Post-Policy Adj-RIB-In 路由数",
        StatType::RoutesInPerAfiSafiPrePolicyAdjRibOut => {
            "按 AFI/SAFI 的 Pre-Policy Adj-RIB-Out 路由数"
        }
        StatType::RoutesInPerAfiSafiPostPolicyAdjRibOut => {
            "按 AFI/SAFI 的 Post-Policy Adj-RIB-Out 路由数"
        }
        StatType::RoutesInPerAfiSafiPrePolicyAdjRibIn => {
            "按 AFI/SAFI 的 Pre-Policy Adj-RIB-In 路由数"
        }
        StatType::RoutesInPerAfiSafiPostPolicyAdjRibIn => {
            "按 AFI/SAFI 的 Post-Policy Adj-RIB-In 路由数"
        }
        StatType::RoutesInPerAfiSafiPrePolicyAdjRibInRejectedByPolicy => {
            "按 AFI/SAFI 被策略拒绝的 Pre-Policy Adj-RIB-In 路由数"
        }
        StatType::RoutesInPerAfiSafiPostPolicyAdjRibInAcceptedByPolicy => {
            "按 AFI/SAFI 被策略接受的 Post-Policy Adj-RIB-In 路由数"
        }
        other => return format!("统计类型 {}", u16::from(other)),
    };
    name.to_string()
}

pub fn initiation_tlv_views(tlvs: &[InitiationTlv]) -> Vec<TlvView> {
    tlvs.iter()
        .map(|tlv| TlvView {
            ty: u16::from(tlv.info_type),
            name: initiation_tlv_name(tlv.info_type).to_string(),
            value: tlv.info.clone(),
        })
        .collect()
}

pub fn initiation_tlv_name(t: InitiationTlvType) -> &'static str {
    match t {
        InitiationTlvType::String => "String",
        InitiationTlvType::SysDescr => "SysDescr",
        InitiationTlvType::SysName => "SysName",
        InitiationTlvType::VrTableName => "VrTableName",
        InitiationTlvType::AdminLabel => "AdminLabel",
        InitiationTlvType::Unknown(_) => "Unknown",
    }
}

pub fn peer_up_tlv_views(tlvs: &[PeerUpNotificationTlv]) -> Vec<TlvView> {
    tlvs.iter()
        .map(|tlv| TlvView {
            ty: u16::from(tlv.info_type),
            name: format!("{:?}", tlv.info_type),
            value: tlv.info_value.clone(),
        })
        .collect()
}

pub fn termination_tlv_views(
    tlvs: &[bgpkit_parser::parser::bmp::messages::TerminationTlv],
) -> (Vec<TlvView>, Option<String>) {
    let mut views = Vec::new();
    let mut reason = None;
    for tlv in tlvs {
        let (name, value) = match &tlv.info_value {
            TerminationTlvValue::String(s) => ("String".to_string(), s.clone()),
            TerminationTlvValue::Reason(r) => {
                let text = termination_reason_text(*r);
                reason = Some(text.clone());
                ("Reason".to_string(), text)
            }
        };
        views.push(TlvView {
            ty: u16::from(tlv.info_type),
            name,
            value,
        });
    }
    (views, reason)
}

pub fn termination_reason_text(reason: TerminationReason) -> String {
    let text = match reason {
        TerminationReason::AdministrativelyClosed => "管理性关闭",
        TerminationReason::UnspecifiedReason => "未指定原因",
        TerminationReason::OutOfResources => "资源耗尽",
        TerminationReason::RedundantConnection => "冗余连接",
        TerminationReason::PermanentlyAdministrativelyClosed => "永久管理性关闭",
    };
    format!("{text} · code {}", u16::from(reason))
}

pub fn peer_down_detail(msg: &PeerDownNotification) -> Option<String> {
    msg.data.as_ref().map(|data| {
        let preview: String = data
            .iter()
            .take(32)
            .map(|b| format!("{b:02x}"))
            .collect::<Vec<_>>()
            .join("");
        format!("{} byte(s): 0x{preview}", data.len())
    })
}

/// Extract a human-readable router name from a BMP initiation message.
pub fn router_name_from_initiation(tlvs: &[InitiationTlv]) -> Option<String> {
    tlvs.iter()
        .find(|t| matches!(t.info_type, InitiationTlvType::SysName))
        .or_else(|| {
            tlvs.iter()
                .find(|t| matches!(t.info_type, InitiationTlvType::String))
        })
        .map(|t| t.info.trim().to_string())
        .filter(|s| !s.is_empty())
}
