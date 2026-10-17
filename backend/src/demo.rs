//! Synthetic BMP feed used for demos and end-to-end verification.
//!
//! The generator does not bypass the collector: it opens a real TCP connection to
//! the configured BMP listener and emits spec-compliant BMP v3 messages, so the
//! exact same framing, parsing and ingestion path is exercised.

use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::str::FromStr;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Weak};
use std::time::Duration;

use bgpkit_parser::models::capabilities::{
    BgpCapabilityType, FourOctetAsCapability, MultiprotocolExtensionsCapability,
};
use bgpkit_parser::models::{
    Afi, AsPath, Asn, AsnLength, AttributeValue, Attributes, BgpMessage, BgpOpenMessage,
    BgpUpdateMessage, Capability, CapabilityValue, Community, LargeCommunity, NetworkPrefix,
    OptParam, Origin, ParamValue, Safi,
};
use tokio::io::AsyncWriteExt;
use tokio::net::TcpStream;
use tokio::sync::watch;
use tracing::{info, warn};

use crate::state::{AppState, now_secs};

const DEMO_SYS_NAME: &str = "bmp-demo-router";
const DEMO_SYS_DESCR: &str = "BMP Status synthetic feed (BIRD/FRR compatible)";

struct DemoPeer {
    asn: u32,
    ip: Ipv4Addr,
    bgp_id: Ipv4Addr,
    local_ip: Ipv4Addr,
    remote_port: u16,
    prefixes: &'static [&'static str],
    upstreams: &'static [&'static [u32]],
}

pub struct DemoSession {
    pub id: u64,
    pub started_at: f64,
    pub sent: Arc<AtomicU64>,
    stop_tx: watch::Sender<bool>,
}

impl DemoSession {
    pub fn stop(&self) {
        let _ = self.stop_tx.send(true);
    }
}

pub fn start(state: &Arc<AppState>) -> Result<u64, String> {
    let mut guard = state.demo.lock().unwrap_or_else(|e| e.into_inner());
    if guard.is_some() {
        return Err("演示数据流已经在运行".to_string());
    }
    let id = state.next_demo_id();
    let (stop_tx, stop_rx) = watch::channel(false);
    let sent = Arc::new(AtomicU64::new(0));
    *guard = Some(DemoSession {
        id,
        started_at: now_secs(),
        sent: sent.clone(),
        stop_tx,
    });
    drop(guard);

    let state2 = state.clone();
    let connect_addr = local_connect_addr(state.config.bmp_bind);
    let interval = Duration::from_millis(state.config.demo_interval_ms.clamp(20, 10_000));
    tokio::spawn(async move {
        run(state2, id, connect_addr, interval, stop_rx, sent).await;
    });
    Ok(id)
}

pub fn stop(state: &Arc<AppState>) -> bool {
    let guard = state.demo.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(session) = guard.as_ref() {
        session.stop();
        true
    } else {
        false
    }
}

fn local_connect_addr(bind: SocketAddr) -> SocketAddr {
    if bind.ip().is_unspecified() {
        match bind.ip() {
            IpAddr::V4(_) => SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), bind.port()),
            IpAddr::V6(_) => {
                SocketAddr::new(IpAddr::V6(std::net::Ipv6Addr::LOCALHOST), bind.port())
            }
        }
    } else {
        bind
    }
}

struct DemoGuard {
    state: Weak<AppState>,
    id: u64,
}

impl Drop for DemoGuard {
    fn drop(&mut self) {
        if let Some(state) = self.state.upgrade() {
            state.clear_demo(self.id);
        }
    }
}

async fn run(
    state: Arc<AppState>,
    id: u64,
    connect_addr: SocketAddr,
    interval: Duration,
    mut stop: watch::Receiver<bool>,
    sent: Arc<AtomicU64>,
) {
    let _guard = DemoGuard {
        state: Arc::downgrade(&state),
        id,
    };
    info!(%connect_addr, "starting synthetic BMP demo feed");

    loop {
        if *stop.borrow() || state.is_shutting_down() {
            break;
        }
        match tokio::time::timeout(Duration::from_secs(5), TcpStream::connect(connect_addr)).await {
            Ok(Ok(stream)) => {
                info!(%connect_addr, "demo feed connected to BMP listener");
                if let Err(e) = feed(stream, &mut stop, interval, &sent).await {
                    warn!("demo feed stopped: {e}");
                }
            }
            Ok(Err(e)) => warn!(%connect_addr, "demo feed connection failed: {e}"),
            Err(_) => warn!(%connect_addr, "demo feed connection timed out"),
        }
        tokio::select! {
            _ = stop.changed() => break,
            _ = tokio::time::sleep(Duration::from_secs(2)) => {}
        }
    }
    info!("synthetic BMP demo feed stopped");
}

async fn feed(
    mut stream: TcpStream,
    stop: &mut watch::Receiver<bool>,
    interval: Duration,
    sent: &AtomicU64,
) -> std::io::Result<()> {
    stream.set_nodelay(true)?;
    let peers = demo_peers();
    let mut rng = Rng::new(seed());
    let mut down = vec![false; peers.len()];

    send(&mut stream, initiation_message(), sent).await?;
    for peer in &peers {
        send(&mut stream, peer_up_message(peer, now_secs()), sent).await?;
    }
    // Seed the RIB so the dashboard is immediately useful.
    for peer in &peers {
        let prefixes = parse_prefixes(peer, peer.prefixes.len());
        let update = announce_update(peer, prefixes, &mut rng);
        send(&mut stream, route_message(peer, &update, now_secs()), sent).await?;
    }

    let mut iteration: u64 = 0;
    loop {
        if *stop.borrow() {
            break;
        }
        let jitter =
            interval + Duration::from_millis(rng.below(interval.as_millis() as usize / 2) as u64);
        tokio::select! {
            _ = stop.changed() => break,
            _ = tokio::time::sleep(jitter) => {}
        }
        iteration += 1;

        // Occasionally take a peer down, and later bring it back.
        if iteration % 53 == 0 {
            if let Some(idx) = random_up_peer(&peers, &down, &mut rng) {
                down[idx] = true;
                send(
                    &mut stream,
                    peer_down_message(&peers[idx], now_secs()),
                    sent,
                )
                .await?;
            }
        } else if let Some(idx) = down.iter().position(|d| *d) {
            if rng.chance(40) {
                down[idx] = false;
                send(&mut stream, peer_up_message(&peers[idx], now_secs()), sent).await?;
                let update = announce_update(
                    &peers[idx],
                    parse_prefixes(&peers[idx], peers[idx].prefixes.len()),
                    &mut rng,
                );
                send(
                    &mut stream,
                    route_message(&peers[idx], &update, now_secs()),
                    sent,
                )
                .await?;
            }
        }

        let Some(idx) = random_up_peer(&peers, &down, &mut rng) else {
            continue;
        };
        let peer = &peers[idx];

        let roll = rng.below(100);
        if roll < 8 {
            // Statistics report
            send(&mut stream, stats_message(peer, &mut rng, now_secs()), sent).await?;
        } else if roll < 18 {
            // Withdrawal
            let prefix = parse_prefix(peer.prefixes[rng.below(peer.prefixes.len())]);
            let update = withdraw_update(prefix);
            send(&mut stream, route_message(peer, &update, now_secs()), sent).await?;
        } else if roll < 38 {
            // Multi-prefix announcement burst
            let count = 2 + rng.below(4).min(peer.prefixes.len().saturating_sub(1));
            let prefixes = parse_prefixes(peer, count);
            let update = announce_update(peer, prefixes, &mut rng);
            send(&mut stream, route_message(peer, &update, now_secs()), sent).await?;
        } else {
            // Single-prefix announcement
            let prefix = parse_prefix(peer.prefixes[rng.below(peer.prefixes.len())]);
            let update = announce_update(peer, vec![prefix], &mut rng);
            send(&mut stream, route_message(peer, &update, now_secs()), sent).await?;
        }

        if iteration % 137 == 0 {
            send(&mut stream, stats_message(peer, &mut rng, now_secs()), sent).await?;
        }
    }

    let _ = send(&mut stream, termination_message(), sent).await;
    let _ = stream.shutdown().await;
    Ok(())
}

async fn send(stream: &mut TcpStream, bytes: Vec<u8>, sent: &AtomicU64) -> std::io::Result<()> {
    tokio::time::timeout(Duration::from_secs(5), stream.write_all(&bytes))
        .await
        .map_err(|_| std::io::Error::new(std::io::ErrorKind::TimedOut, "demo write timed out"))??;
    sent.fetch_add(1, Ordering::Relaxed);
    Ok(())
}

fn demo_peers() -> Vec<DemoPeer> {
    vec![
        DemoPeer {
            asn: 13335,
            ip: Ipv4Addr::new(192, 0, 2, 1),
            bgp_id: Ipv4Addr::new(192, 0, 2, 1),
            local_ip: Ipv4Addr::new(192, 0, 2, 254),
            remote_port: 179,
            prefixes: &[
                "104.16.0.0/13",
                "104.24.0.0/14",
                "1.1.1.0/24",
                "162.158.0.0/15",
                "172.64.0.0/13",
                "188.114.96.0/20",
            ],
            upstreams: &[&[174, 3356], &[6939], &[15169, 1299]],
        },
        DemoPeer {
            asn: 15169,
            ip: Ipv4Addr::new(198, 51, 100, 1),
            bgp_id: Ipv4Addr::new(198, 51, 100, 1),
            local_ip: Ipv4Addr::new(198, 51, 100, 254),
            remote_port: 179,
            prefixes: &[
                "8.8.8.0/24",
                "8.8.4.0/24",
                "142.250.0.0/15",
                "172.217.0.0/16",
                "216.58.192.0/19",
                "34.64.0.0/10",
            ],
            upstreams: &[&[1299], &[3356, 174], &[6939]],
        },
        DemoPeer {
            asn: 6939,
            ip: Ipv4Addr::new(203, 0, 113, 1),
            bgp_id: Ipv4Addr::new(203, 0, 113, 1),
            local_ip: Ipv4Addr::new(203, 0, 113, 254),
            remote_port: 179,
            prefixes: &[
                "74.82.0.0/16",
                "216.218.128.0/17",
                "184.105.64.0/18",
                "198.51.44.0/24",
            ],
            upstreams: &[&[13335], &[3356], &[1299]],
        },
        DemoPeer {
            asn: 9808,
            ip: Ipv4Addr::new(192, 0, 2, 65),
            bgp_id: Ipv4Addr::new(192, 0, 2, 65),
            local_ip: Ipv4Addr::new(192, 0, 2, 126),
            remote_port: 179,
            prefixes: &[
                "221.176.0.0/13",
                "211.136.0.0/13",
                "183.232.0.0/16",
                "117.136.0.0/13",
                "111.0.0.0/10",
            ],
            upstreams: &[&[58453, 6939], &[4134, 1299], &[58453]],
        },
    ]
}

fn random_up_peer(peers: &[DemoPeer], down: &[bool], rng: &mut Rng) -> Option<usize> {
    let up: Vec<usize> = (0..peers.len()).filter(|i| !down[*i]).collect();
    if up.is_empty() {
        None
    } else {
        Some(up[rng.below(up.len())])
    }
}

fn parse_prefix(s: &str) -> NetworkPrefix {
    NetworkPrefix::from_str(s).expect("static demo prefix must parse")
}

fn parse_prefixes(peer: &DemoPeer, count: usize) -> Vec<NetworkPrefix> {
    let count = count.clamp(1, peer.prefixes.len());
    let mut rng = Rng::new(seed());
    let mut indexes: Vec<usize> = (0..peer.prefixes.len()).collect();
    // Shuffle a few entries for variety.
    for i in (1..indexes.len()).rev() {
        let j = rng.below(i + 1);
        indexes.swap(i, j);
    }
    indexes
        .into_iter()
        .take(count)
        .map(|i| parse_prefix(peer.prefixes[i]))
        .collect()
}

fn announce_update(
    peer: &DemoPeer,
    prefixes: Vec<NetworkPrefix>,
    rng: &mut Rng,
) -> BgpUpdateMessage {
    let upstream = peer.upstreams[rng.below(peer.upstreams.len())];
    let mut path = Vec::with_capacity(upstream.len() + 1);
    path.push(peer.asn);
    path.extend_from_slice(upstream);

    let mut attrs = Attributes::default();
    attrs.add_attr(AttributeValue::Origin(Origin::IGP).into());
    attrs.add_attr(AttributeValue::AsPath(AsPath::from_sequence(&path)).into());
    attrs.add_attr(AttributeValue::NextHop(IpAddr::V4(peer.ip)).into());
    attrs.add_attr(AttributeValue::MultiExitDiscriminator((rng.below(20) as u32) * 10).into());
    attrs.add_attr(AttributeValue::LocalPreference(100 + (rng.below(3) as u32) * 10).into());
    attrs.add_attr(
        AttributeValue::Communities(vec![
            Community::Custom(Asn::new_32bit(peer.asn), 100 + rng.below(3) as u16),
            Community::Custom(Asn::new_32bit(peer.asn), 200 + rng.below(2) as u16),
        ])
        .into(),
    );
    attrs.add_attr(
        AttributeValue::LargeCommunities(vec![LargeCommunity::new(
            peer.asn,
            [100, rng.below(8) as u32],
        )])
        .into(),
    );

    BgpUpdateMessage {
        withdrawn_prefixes: Vec::new(),
        attributes: attrs,
        announced_prefixes: prefixes,
    }
}

fn withdraw_update(prefix: NetworkPrefix) -> BgpUpdateMessage {
    BgpUpdateMessage {
        withdrawn_prefixes: vec![prefix],
        attributes: Attributes::default(),
        announced_prefixes: Vec::new(),
    }
}

fn route_message(peer: &DemoPeer, update: &BgpUpdateMessage, ts: f64) -> Vec<u8> {
    let body = BgpMessage::Update(update.clone())
        .encode(AsnLength::Bits32)
        .expect("BGP UPDATE encoding cannot fail for demo data");
    bmp_message(0, Some(peer), ts, &body)
}

fn initiation_message() -> Vec<u8> {
    let mut body = Vec::new();
    push_tlv(&mut body, 1, DEMO_SYS_DESCR.as_bytes());
    push_tlv(&mut body, 2, DEMO_SYS_NAME.as_bytes());
    bmp_message(4, None, now_secs(), &body)
}

fn termination_message() -> Vec<u8> {
    let mut body = Vec::new();
    push_tlv(&mut body, 1, &0u16.to_be_bytes());
    bmp_message(5, None, now_secs(), &body)
}

fn peer_up_message(peer: &DemoPeer, ts: f64) -> Vec<u8> {
    let mut body = Vec::with_capacity(64 + 2 * 64);
    body.extend_from_slice(&[0u8; 12]);
    body.extend_from_slice(&peer.local_ip.octets());
    body.extend_from_slice(&179u16.to_be_bytes());
    body.extend_from_slice(&peer.remote_port.to_be_bytes());
    body.extend_from_slice(&open_message(peer));
    body.extend_from_slice(&open_message(peer));
    push_tlv(&mut body, 0, b"bmp-status-demo");
    bmp_message(3, Some(peer), ts, &body)
}

fn open_message(peer: &DemoPeer) -> Vec<u8> {
    let capabilities = vec![
        Capability {
            ty: BgpCapabilityType::MULTIPROTOCOL_EXTENSIONS_FOR_BGP_4,
            value: CapabilityValue::MultiprotocolExtensions(
                MultiprotocolExtensionsCapability::new(Afi::Ipv4, Safi::Unicast),
            ),
        },
        Capability {
            ty: BgpCapabilityType::SUPPORT_FOR_4_OCTET_AS_NUMBER_CAPABILITY,
            value: CapabilityValue::FourOctetAs(FourOctetAsCapability::new(peer.asn)),
        },
    ];
    let msg = BgpMessage::Open(BgpOpenMessage {
        version: 4,
        asn: Asn::new_32bit(peer.asn),
        hold_time: 180,
        bgp_identifier: peer.bgp_id,
        extended_length: false,
        opt_params: vec![OptParam {
            param_type: 2,
            param_value: ParamValue::Capacities(capabilities),
        }],
    });
    msg.encode(AsnLength::Bits32)
        .expect("BGP OPEN encoding cannot fail for demo data")
        .to_vec()
}

fn peer_down_message(peer: &DemoPeer, ts: f64) -> Vec<u8> {
    // Reason 4: remote system closed the session without data.
    bmp_message(2, Some(peer), ts, &[4])
}

fn stats_message(peer: &DemoPeer, rng: &mut Rng, ts: f64) -> Vec<u8> {
    let route_count = 800 + rng.below(4200) as u64;
    let counters: Vec<u8> = {
        let mut c = Vec::new();
        c.extend_from_slice(&3u32.to_be_bytes());
        // type 7: routes in Adj-RIB-In (gauge, 8 bytes)
        c.extend_from_slice(&7u16.to_be_bytes());
        c.extend_from_slice(&8u16.to_be_bytes());
        c.extend_from_slice(&route_count.to_be_bytes());
        // type 0: prefixes rejected by inbound policy (counter, 4 bytes)
        c.extend_from_slice(&0u16.to_be_bytes());
        c.extend_from_slice(&4u16.to_be_bytes());
        c.extend_from_slice(&(rng.below(100) as u32).to_be_bytes());
        // type 13: duplicate update messages (counter, 4 bytes)
        c.extend_from_slice(&13u16.to_be_bytes());
        c.extend_from_slice(&4u16.to_be_bytes());
        c.extend_from_slice(&(rng.below(20) as u32).to_be_bytes());
        c
    };
    bmp_message(1, Some(peer), ts, &counters)
}

fn push_tlv(buf: &mut Vec<u8>, ty: u16, value: &[u8]) {
    buf.extend_from_slice(&ty.to_be_bytes());
    buf.extend_from_slice(&(value.len() as u16).to_be_bytes());
    buf.extend_from_slice(value);
}

fn bmp_message(msg_type: u8, peer: Option<&DemoPeer>, ts: f64, body: &[u8]) -> Vec<u8> {
    let peer_len = if peer.is_some() { 42 } else { 0 };
    let total = 6 + peer_len + body.len();
    let mut buf = Vec::with_capacity(total);
    buf.push(3);
    buf.extend_from_slice(&(total as u32).to_be_bytes());
    buf.push(msg_type);
    if let Some(peer) = peer {
        buf.extend_from_slice(&per_peer_header(peer, ts));
    }
    buf.extend_from_slice(body);
    buf
}

fn per_peer_header(peer: &DemoPeer, ts: f64) -> Vec<u8> {
    let mut buf = Vec::with_capacity(42);
    buf.push(0); // peer type: global instance
    buf.push(0); // flags: IPv4, pre-policy, 4-byte ASN
    buf.extend_from_slice(&0u64.to_be_bytes()); // distinguisher
    buf.extend_from_slice(&[0u8; 12]);
    buf.extend_from_slice(&peer.ip.octets());
    buf.extend_from_slice(&peer.asn.to_be_bytes());
    buf.extend_from_slice(&peer.bgp_id.octets());
    let secs = ts.floor() as u64;
    let usec = ((ts.fract()) * 1_000_000.0) as u32;
    buf.extend_from_slice(&(secs as u32).to_be_bytes());
    buf.extend_from_slice(&usec.to_be_bytes());
    buf
}

fn seed() -> u64 {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos() as u64;
    nanos | 1
}

struct Rng(u64);

impl Rng {
    fn new(seed: u64) -> Self {
        Self(seed | 1)
    }

    fn next(&mut self) -> u64 {
        // xorshift64
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }

    fn below(&mut self, n: usize) -> usize {
        if n == 0 {
            0
        } else {
            (self.next() % n as u64) as usize
        }
    }

    fn chance(&mut self, percent: usize) -> bool {
        self.below(100) < percent
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bgpkit_parser::parse_bmp_msg;
    use bytes::Bytes;

    #[test]
    fn demo_messages_round_trip_through_bgpkit_parser() {
        let peers = demo_peers();
        let mut rng = Rng::new(42);
        let updates = vec![
            initiation_message(),
            peer_up_message(&peers[0], 1_700_000_000.0),
            peer_down_message(&peers[1], 1_700_000_001.0),
            stats_message(&peers[2], &mut rng, 1_700_000_002.0),
            termination_message(),
        ];
        for (idx, bytes) in updates.into_iter().enumerate() {
            let mut data = Bytes::from(bytes);
            let msg = parse_bmp_msg(&mut data).unwrap_or_else(|e| panic!("msg {idx} failed: {e}"));
            assert_eq!(msg.common_header.version, 3);
        }

        let update = announce_update(&peers[0], vec![parse_prefix("104.16.0.0/13")], &mut rng);
        let bytes = route_message(&peers[0], &update, 1_700_000_003.0);
        let mut data = Bytes::from(bytes);
        let msg = parse_bmp_msg(&mut data).expect("route monitoring parses");
        assert!(matches!(
            msg.message_body,
            bgpkit_parser::parser::bmp::messages::BmpMessageBody::RouteMonitoring(_)
        ));
        let peer = msg.per_peer_header.expect("per-peer header");
        assert_eq!(peer.peer_asn.to_u32(), 13335);
        assert_eq!(peer.peer_ip.to_string(), "192.0.2.1");
    }
}
