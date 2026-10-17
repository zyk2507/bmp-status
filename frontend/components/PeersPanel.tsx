import type { PeerView } from "@/lib/types";
import { formatNumber, timeAgo } from "@/lib/format";

function sessionLabel(session: string): string {
  if (session === "up") return "ESTABLISHED";
  if (session === "down") return "DOWN";
  return "UNKNOWN";
}

export function PeersPanel({
  peers,
  instanceByIp,
  selectedKey,
  onSelect,
}: {
  peers: PeerView[];
  instanceByIp: Record<string, string>;
  selectedKey: string | null;
  onSelect: (key: string | null) => void;
}) {
  return (
    <div className="peer-panel">
      <div className="peer-panel-summary">
        <span>
          <b>{peers.filter((peer) => peer.session === "up").length}</b> 在线
        </span>
        <span>
          <b>{peers.length}</b> 已发现
        </span>
        <span>
          <b>{formatNumber(peers.reduce((acc, peer) => acc + peer.prefixes, 0))}</b> 活跃前缀
        </span>
        {selectedKey && (
          <button className="text-button" type="button" onClick={() => onSelect(null)}>
            清除选择
          </button>
        )}
      </div>

      <div className="peer-list">
        {peers.length === 0 && (
          <div className="panel-empty">
            尚未发现 BGP 对等体。收到 Peer Up 或 Route Monitoring 后会自动出现在这里。
          </div>
        )}
        {peers.map((peer) => {
          const selected = peer.key === selectedKey;
          const instanceName = peer.router ? instanceByIp[peer.router] : undefined;
          return (
            <button
              key={peer.key}
              type="button"
              className={`peer-row${selected ? " selected" : ""}`}
              onClick={() => onSelect(selected ? null : peer.key)}
              title={peer.lastDownReason ?? undefined}
            >
              <span className={`status-dot ${peer.session}`} />

              <span className="peer-identity">
                <span className="peer-line">
                  <span className="peer-ip mono">{peer.ip}</span>
                  <span className="badge asn">AS{peer.asn}</span>
                  {peer.ipv6 && <span className="badge subtle">IPv6</span>}
                  {peer.postPolicy && <span className="badge violet">post-policy</span>}
                  {peer.adjRibOut && <span className="badge amber">adj-rib-out</span>}
                  {peer.asn16bit && <span className="badge subtle">AS2</span>}
                </span>
                <span className="peer-sub">
                  ID {peer.bgpId} · {instanceName ?? peer.router ?? "未知来源"}
                  {peer.localAddr ? ` · local ${peer.localAddr}:${peer.localPort ?? "—"}` : ""}
                </span>
              </span>

              <span className="peer-metrics">
                <span className="metric">
                  <b>{formatNumber(peer.prefixes)}</b>
                  <em>前缀</em>
                </span>
                <span className="metric">
                  <b>{formatNumber(peer.messages)}</b>
                  <em>消息</em>
                </span>
                <span className="metric up">
                  <b>↑{formatNumber(peer.announcements)}</b>
                  <em>通告</em>
                </span>
                <span className="metric down">
                  <b>↓{formatNumber(peer.withdrawals)}</b>
                  <em>撤销</em>
                </span>
              </span>

              <span className="peer-time">
                <span className={`session-label ${peer.session}`}>{sessionLabel(peer.session)}</span>
                <span>{timeAgo(peer.lastSeen)}</span>
              </span>
            </button>
          );
        })}
      </div>
    </div>
  );
}
