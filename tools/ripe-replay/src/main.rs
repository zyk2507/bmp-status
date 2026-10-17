//! Fetch/cached RIPE RIS updates with bgpkit-parser, then install the announced
//! routes into a RustyBGP Loc-RIB through the GoBGP-compatible gRPC API.
//!
//! This tool never touches the kernel routing table: it only talks to the
//! RustyBGP management API. The test topology deliberately runs RustyBGP
//! without the `zebra` section, which also disables kernel FIB writes and
//! next-hop tracking (NHT).

use std::collections::{HashMap, HashSet};
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};
use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{Context, Result, bail};
use bgpkit_parser::models::{AsPathSegment, Community, ElemType, MetaCommunity, Origin};
use bgpkit_parser::BgpkitParser;
use clap::Parser;
use serde::Serialize;
use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;
use tracing::{info, warn};

mod api {
    #![allow(dead_code)]
    tonic::include_proto!("api");
}

use api::attribute::Attr;
use api::go_bgp_service_client::GoBgpServiceClient;
use api::nlri::Nlri;

/// RRC26 is a small RIS collector, so its 5-minute updates file is only a few
/// hundred KB instead of hundreds of MB for the full RRC00 feed.
const DEFAULT_URL: &str = "https://data.ris.ripe.net/rrc26/2025.01/updates.20250101.0000.gz";
const AFI_IPV4: i32 = 1;
const AFI_IPV6: i32 = 2;
const SAFI_UNICAST: i32 = 1;

#[derive(Debug, Parser)]
#[command(
    name = "ripe-bgp-replay",
    about = "Replay cached RIPE RIS routes into RustyBGP (gRPC, no kernel writes)"
)]
struct Args {
    /// RIPE RIS MRT file. Cached locally by file name unless --force-download is used.
    #[arg(long, default_value = DEFAULT_URL)]
    url: String,

    /// Cache directory for downloaded RIPE files.
    #[arg(long, default_value = "data/ripe-cache")]
    cache_dir: PathBuf,

    /// Ignore the cached file and download again.
    #[arg(long, default_value_t = false)]
    force_download: bool,

    /// Maximum number of unique IPv4 prefixes to inject (0 disables IPv4).
    #[arg(long, default_value_t = 1_000)]
    limit: usize,

    /// Maximum number of unique IPv6 prefixes to inject (0 disables IPv6).
    #[arg(long, default_value_t = 0)]
    limit_v6: usize,

    /// RustyBGP gRPC endpoint (GoBGP-compatible API).
    #[arg(long, default_value = "http://127.0.0.1:50052")]
    api: String,

    /// Number of paths per AddPathStream batch.
    #[arg(long, default_value_t = 100)]
    batch_size: usize,

    /// Only inject routes whose origin ASN matches.
    #[arg(long)]
    origin_asn: Option<u32>,

    /// Skip routes whose AS path contains one of these ASNs (e.g. the test ASNs).
    #[arg(long = "exclude-asn", default_value = "65001,65002", value_delimiter = ',')]
    exclude_asn: Vec<u32>,

    /// Use this IPv4 next hop when the RIPE path has no IPv4 next hop.
    #[arg(long, default_value = "192.0.2.1")]
    fallback_next_hop: Ipv4Addr,

    /// Use this IPv6 next hop when the RIPE path has no IPv6 next hop.
    #[arg(long, default_value = "2001:db8::1")]
    fallback_next_hop_v6: Ipv6Addr,

    /// Parse/cache only; do not connect to RustyBGP.
    #[arg(long, default_value_t = false)]
    dry_run: bool,

    /// Write cache/injection summary JSON.
    #[arg(long)]
    summary_file: Option<PathBuf>,
}

#[derive(Debug, Serialize)]
struct SampleRoute {
    family: &'static str,
    prefix: String,
    as_path: String,
    next_hop: Option<String>,
    origin_asn: Option<u32>,
}

#[derive(Debug, Serialize)]
struct Summary {
    url: String,
    cache_path: String,
    cache_hit: bool,
    downloaded: bool,
    announcements_seen: usize,
    announcements_v4: usize,
    announcements_v6: usize,
    unique_prefixes: usize,
    unique_prefixes_v4: usize,
    unique_prefixes_v6: usize,
    injected: usize,
    injected_v4: usize,
    injected_v6: usize,
    top_origins: Vec<(u32, usize)>,
    sample: Vec<SampleRoute>,
}

#[derive(Default)]
struct CollectedRoutes {
    v4: Vec<bgpkit_parser::BgpElem>,
    v6: Vec<bgpkit_parser::BgpElem>,
    announcements_v4: usize,
    announcements_v6: usize,
    top_origins: Vec<(u32, usize)>,
}

fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "ripe_bgp_replay=info".into()),
        )
        .with_target(false)
        .init();

    let args = Args::parse();
    if args.limit == 0 && args.limit_v6 == 0 {
        bail!("at least one of --limit / --limit-v6 must be > 0");
    }
    if args.batch_size == 0 {
        bail!("--batch-size must be > 0");
    }

    let (cache_path, cache_hit, downloaded) = ensure_cached(&args)?;
    info!(
        path = %cache_path.display(),
        cache_hit, downloaded, "RIPE RIS data ready"
    );

    let collected = collect_routes(&args, &cache_path)?;
    if collected.v4.is_empty() && collected.v6.is_empty() {
        bail!(
            "no usable RIPE announcements found in {} (v4 seen {}, v6 seen {})",
            cache_path.display(),
            collected.announcements_v4,
            collected.announcements_v6
        );
    }
    if args.limit > 0 && collected.v4.is_empty() {
        bail!("no usable IPv4 announcements found (v4 seen {})", collected.announcements_v4);
    }
    if args.limit_v6 > 0 && collected.v6.is_empty() {
        bail!("no usable IPv6 announcements found (v6 seen {})", collected.announcements_v6);
    }

    let announcements_seen = collected.announcements_v4 + collected.announcements_v6;
    let unique_prefixes = collected.v4.len() + collected.v6.len();
    info!(
        announcements_seen,
        unique_prefixes,
        unique_prefixes_v4 = collected.v4.len(),
        unique_prefixes_v6 = collected.v6.len(),
        "parsed RIPE announcements"
    );
    info!(
        "top origin ASNs: {}",
        collected
            .top_origins
            .iter()
            .take(5)
            .map(|(asn, count)| format!("AS{asn} ({count})"))
            .collect::<Vec<_>>()
            .join(", ")
    );
    // Keep output bounded: at most two samples per address family.
    for (family, elems) in [("ipv4", &collected.v4), ("ipv6", &collected.v6)] {
        for elem in elems.iter().take(2) {
            info!(
                family,
                prefix = %elem.prefix,
                as_path = %elem
                    .as_path
                    .as_ref()
                    .map(|p| p.to_string())
                    .unwrap_or_default(),
                next_hop = ?elem.next_hop,
                "sample route"
            );
        }
    }

    let mut paths = Vec::with_capacity(unique_prefixes);
    let mut valid_v4 = 0usize;
    let mut valid_v6 = 0usize;
    for elem in &collected.v4 {
        match build_path(elem, args.fallback_next_hop, args.fallback_next_hop_v6) {
            Ok(path) => {
                paths.push(path);
                valid_v4 += 1;
            }
            Err(e) => warn!(prefix = %elem.prefix, "skipping IPv4 route: {e}"),
        }
    }
    for elem in &collected.v6 {
        match build_path(elem, args.fallback_next_hop, args.fallback_next_hop_v6) {
            Ok(path) => {
                paths.push(path);
                valid_v6 += 1;
            }
            Err(e) => warn!(prefix = %elem.prefix, "skipping IPv6 route: {e}"),
        }
    }

    let (injected, injected_v4, injected_v6) = if args.dry_run {
        (0, 0, 0)
    } else {
        (paths.len(), valid_v4, valid_v6)
    };
    if !args.dry_run {
        if paths.is_empty() {
            bail!("no valid paths to inject");
        }
        // Blocking I/O (oneio/reqwest and parsing) is kept outside the runtime.
        // Build the runtime only for the gRPC injection phase.
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .context("failed to create Tokio runtime")?;
        runtime.block_on(inject(&args.api, paths, args.batch_size))?;
        info!(
            injected,
            injected_v4, injected_v6, "routes added to RustyBGP Loc-RIB via AddPathStream"
        );
    } else {
        info!(
            parsed_v4 = valid_v4,
            parsed_v6 = valid_v6, "dry run: not connecting to RustyBGP"
        );
    }

    let mut sample = Vec::new();
    for (family, elems) in [("ipv4", &collected.v4), ("ipv6", &collected.v6)] {
        sample.extend(elems.iter().take(4).map(|elem| SampleRoute {
            family,
            prefix: elem.prefix.to_string(),
            as_path: elem
                .as_path
                .as_ref()
                .map(|p| p.to_string())
                .unwrap_or_default(),
            next_hop: elem.next_hop.map(|ip| ip.to_string()),
            origin_asn: elem
                .origin_asns
                .as_ref()
                .and_then(|asns| asns.last())
                .map(|asn| asn.to_u32()),
        }));
    }

    let summary = Summary {
        url: args.url.clone(),
        cache_path: cache_path.display().to_string(),
        cache_hit,
        downloaded,
        announcements_seen,
        announcements_v4: collected.announcements_v4,
        announcements_v6: collected.announcements_v6,
        unique_prefixes,
        unique_prefixes_v4: collected.v4.len(),
        unique_prefixes_v6: collected.v6.len(),
        injected,
        injected_v4,
        injected_v6,
        top_origins: collected.top_origins.into_iter().take(10).collect(),
        sample,
    };
    if let Some(path) = &args.summary_file {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(path, serde_json::to_vec_pretty(&summary)?)?;
        info!(path = %path.display(), "wrote summary");
    }
    Ok(())
}

fn ensure_cached(args: &Args) -> Result<(PathBuf, bool, bool)> {
    let file_name = args
        .url
        .rsplit('/')
        .next()
        .filter(|name| !name.is_empty())
        .unwrap_or("ripe-updates.mrt.gz");
    let path = args.cache_dir.join(file_name);
    if path.exists() && !args.force_download {
        let size = std::fs::metadata(&path)?.len();
        if size > 1024 {
            info!(
                path = %path.display(),
                size = %human_size(size),
                "using cached RIPE RIS file (no download)"
            );
            return Ok((path, true, false));
        }
        warn!(path = %path.display(), size, "cached file looks incomplete; downloading again");
    }

    std::fs::create_dir_all(&args.cache_dir)
        .with_context(|| format!("failed to create {}", args.cache_dir.display()))?;
    let tmp = path.with_extension("part");
    info!(url = %args.url, dest = %path.display(), "downloading RIPE RIS file");
    oneio::download(&args.url, tmp.to_str().context("cache path is not UTF-8")?)
        .with_context(|| format!("failed to download {}", args.url))?;
    let size = std::fs::metadata(&tmp)?.len();
    if size < 1024 {
        bail!("downloaded file is unexpectedly small ({size} bytes): {}", args.url);
    }
    std::fs::rename(&tmp, &path)?;
    info!(path = %path.display(), size = %human_size(size), "downloaded and cached RIPE RIS file");
    Ok((path, false, true))
}

fn collect_routes(args: &Args, cache_path: &Path) -> Result<CollectedRoutes> {
    let parser = BgpkitParser::new(cache_path.to_str().context("cache path is not UTF-8")?)
        .with_context(|| format!("failed to open {}", cache_path.display()))?;

    let excluded: HashSet<u32> = args.exclude_asn.iter().copied().collect();
    let mut seen_v4 = HashSet::new();
    let mut seen_v6 = HashSet::new();
    let mut announcements_v4 = 0usize;
    let mut announcements_v6 = 0usize;
    let mut origins: HashMap<u32, usize> = HashMap::new();
    let mut v4 = Vec::new();
    let mut v6 = Vec::new();

    for elem in parser.into_elem_iter() {
        if elem.elem_type != ElemType::ANNOUNCE {
            continue;
        }
        let is_v6 = elem.prefix.prefix.addr().is_ipv6();
        if is_v6 && args.limit_v6 == 0 {
            continue;
        }
        if !is_v6 && args.limit == 0 {
            continue;
        }
        let as_path = match elem.as_path.as_ref() {
            Some(path) if !path.is_empty() => path,
            _ => continue,
        };
        if as_path
            .iter_segments()
            .flat_map(|segment| segment.iter())
            .any(|asn| excluded.contains(&asn.to_u32()))
        {
            continue;
        }
        if let Some(origin_filter) = args.origin_asn {
            let matches = elem
                .origin_asns
                .as_ref()
                .map(|asns| asns.iter().any(|asn| asn.to_u32() == origin_filter))
                .unwrap_or(false);
            if !matches {
                continue;
            }
        }

        if is_v6 {
            announcements_v6 += 1;
        } else {
            announcements_v4 += 1;
        }
        for asn in elem.origin_asns.as_ref().into_iter().flatten() {
            *origins.entry(asn.to_u32()).or_default() += 1;
        }

        if is_v6 && v6.len() >= args.limit_v6 {
            continue;
        }
        if !is_v6 && v4.len() >= args.limit {
            continue;
        }
        let seen = if is_v6 { &mut seen_v6 } else { &mut seen_v4 };
        if !seen.insert(elem.prefix.to_string()) {
            continue;
        }
        if is_v6 {
            v6.push(elem);
        } else {
            v4.push(elem);
        }

        let v4_done = args.limit == 0 || v4.len() >= args.limit;
        let v6_done = args.limit_v6 == 0 || v6.len() >= args.limit_v6;
        if v4_done && v6_done {
            break;
        }
    }

    let mut top_origins: Vec<(u32, usize)> = origins.into_iter().collect();
    top_origins.sort_by(|a, b| b.1.cmp(&a.1));
    Ok(CollectedRoutes {
        v4,
        v6,
        announcements_v4,
        announcements_v6,
        top_origins,
    })
}

fn build_path(
    elem: &bgpkit_parser::BgpElem,
    fallback_next_hop: Ipv4Addr,
    fallback_next_hop_v6: Ipv6Addr,
) -> Result<api::Path> {
    let is_v6 = elem.prefix.prefix.addr().is_ipv6();
    let prefix_len = elem.prefix.prefix.prefix_len() as u32;
    let prefix_addr = elem.prefix.prefix.addr().to_string();
    let nlri = api::Nlri {
        nlri: Some(Nlri::Prefix(api::IpAddressPrefix {
            prefix_len,
            prefix: prefix_addr,
        })),
    };

    let mut attrs = Vec::new();
    let origin = match elem.origin.as_ref() {
        Some(Origin::IGP) => 0,
        Some(Origin::EGP) => 1,
        _ => 2,
    };
    attrs.push(api::Attribute {
        attr: Some(Attr::Origin(api::OriginAttribute { origin })),
    });

    if let Some(path) = elem.as_path.as_ref() {
        let mut segments = Vec::new();
        for segment in path.iter_segments() {
            let (kind, numbers): (i32, Vec<u32>) = match segment {
                AsPathSegment::AsSequence(values) => (
                    2,
                    values.iter().map(|asn| asn.to_u32()).collect(),
                ),
                AsPathSegment::AsSet(values) => {
                    (1, values.iter().map(|asn| asn.to_u32()).collect())
                }
                AsPathSegment::ConfedSequence(values) => {
                    (3, values.iter().map(|asn| asn.to_u32()).collect())
                }
                AsPathSegment::ConfedSet(values) => {
                    (4, values.iter().map(|asn| asn.to_u32()).collect())
                }
            };
            if numbers.is_empty() {
                continue;
            }
            for chunk in numbers.chunks(u8::MAX as usize) {
                segments.push(api::AsSegment {
                    r#type: kind,
                    numbers: chunk.to_vec(),
                });
            }
        }
        if !segments.is_empty() {
            attrs.push(api::Attribute {
                attr: Some(Attr::AsPath(api::AsPathAttribute { segments })),
            });
        }
    }

    if is_v6 {
        let next_hop = match elem.next_hop {
            Some(IpAddr::V6(v6)) => v6,
            _ => fallback_next_hop_v6,
        };
        attrs.push(api::Attribute {
            attr: Some(Attr::MpReach(api::MpReachNlriAttribute {
                family: Some(api::Family {
                    afi: AFI_IPV6,
                    safi: SAFI_UNICAST,
                }),
                next_hops: vec![next_hop.to_string()],
                nlris: Vec::new(),
            })),
        });
    } else {
        let next_hop = match elem.next_hop {
            Some(IpAddr::V4(v4)) => v4,
            _ => fallback_next_hop,
        };
        attrs.push(api::Attribute {
            attr: Some(Attr::NextHop(api::NextHopAttribute {
                next_hop: next_hop.to_string(),
            })),
        });
    }
    if let Some(med) = elem.med {
        attrs.push(api::Attribute {
            attr: Some(Attr::MultiExitDisc(api::MultiExitDiscAttribute { med })),
        });
    }
    if let Some(local_pref) = elem.local_pref {
        attrs.push(api::Attribute {
            attr: Some(Attr::LocalPref(api::LocalPrefAttribute { local_pref })),
        });
    }

    let mut communities = Vec::new();
    let mut large_communities = Vec::new();
    for community in elem.communities.as_ref().into_iter().flatten() {
        match community {
            MetaCommunity::Plain(community) => communities.push(community_to_u32(*community)),
            MetaCommunity::Large(community) => {
                large_communities.push(api::LargeCommunity {
                    global_admin: community.global_admin,
                    local_data1: community.local_data[0],
                    local_data2: community.local_data[1],
                });
            }
            _ => {}
        }
    }
    if !communities.is_empty() {
        attrs.push(api::Attribute {
            attr: Some(Attr::Communities(api::CommunitiesAttribute {
                communities,
            })),
        });
    }
    if !large_communities.is_empty() {
        attrs.push(api::Attribute {
            attr: Some(Attr::LargeCommunities(api::LargeCommunitiesAttribute {
                communities: large_communities,
            })),
        });
    }
    if elem.atomic {
        attrs.push(api::Attribute {
            attr: Some(Attr::AtomicAggregate(api::AtomicAggregateAttribute {})),
        });
    }

    Ok(api::Path {
        nlri: Some(nlri),
        pattrs: attrs,
        family: Some(api::Family {
            afi: if is_v6 { AFI_IPV6 } else { AFI_IPV4 },
            safi: SAFI_UNICAST,
        }),
        ..Default::default()
    })
}

fn community_to_u32(community: Community) -> u32 {
    match community {
        Community::NoExport => 0xFFFF_FF01,
        Community::NoAdvertise => 0xFFFF_FF02,
        Community::NoExportSubConfed => 0xFFFF_FF03,
        Community::Custom(asn, value) => (asn.to_u32() << 16) | value as u32,
    }
}

async fn inject(api_url: &str, paths: Vec<api::Path>, batch_size: usize) -> Result<()> {
    let mut client = connect_with_retry(api_url).await?;
    let total = paths.len();
    let (tx, rx) = mpsc::channel::<api::AddPathStreamRequest>(8);

    let batch_count = paths.len().div_ceil(batch_size);
    let log_every = (batch_count / 10).max(1);
    let sender = tokio::spawn(async move {
        for (index, chunk) in paths.chunks(batch_size).enumerate() {
            let request = api::AddPathStreamRequest {
                table_type: api::TableType::Global as i32,
                vrf_id: String::new(),
                paths: chunk.to_vec(),
            };
            if tx.send(request).await.is_err() {
                bail!("RustyBGP closed the AddPathStream before all batches were sent");
            }
            let done = ((index + 1) * batch_size).min(total);
            // Keep terminal/AI output bounded even for very large RIBs.
            if index == 0 || index + 1 == batch_count || (index + 1) % log_every == 0 {
                info!(done, total, "injecting RIB batches");
            }
        }
        Ok::<_, anyhow::Error>(())
    });

    let response = client
        .add_path_stream(ReceiverStream::new(rx))
        .await
        .context("AddPathStream RPC failed")?;
    let _ = response.into_inner();
    sender.await.context("batch sender task panicked")??;
    Ok(())
}

async fn connect_with_retry(api_url: &str) -> Result<GoBgpServiceClient<tonic::transport::Channel>> {
    let mut last_error = None;
    for attempt in 1..=30 {
        match GoBgpServiceClient::connect(api_url.to_string()).await {
            Ok(client) => {
                info!(api = %api_url, "connected to RustyBGP gRPC API");
                return Ok(client);
            }
            Err(error) => {
                last_error = Some(error);
                if attempt == 1 || attempt % 5 == 0 {
                    warn!(attempt, api = %api_url, "waiting for RustyBGP gRPC API");
                }
                tokio::time::sleep(Duration::from_secs(1)).await;
            }
        }
    }
    Err(anyhow::anyhow!(
        "failed to connect to RustyBGP gRPC API at {api_url}: {}",
        last_error
            .map(|e| e.to_string())
            .unwrap_or_else(|| "unknown error".to_string())
    ))
}

fn human_size(bytes: u64) -> String {
    if bytes >= 1024 * 1024 {
        format!("{:.1} MB", bytes as f64 / 1024.0 / 1024.0)
    } else if bytes >= 1024 {
        format!("{:.1} KB", bytes as f64 / 1024.0)
    } else {
        format!("{bytes} B")
    }
}
