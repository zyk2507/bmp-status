import type { AsnCount, Totals } from "@/lib/types";
import { formatCompact, formatNumber } from "@/lib/format";

interface Segment {
  label: string;
  value: number;
  color: string;
}

const R = 50;
const CIRC = 2 * Math.PI * R;

function AsnRank({ title, data }: { title: string; data: AsnCount[] }) {
  const max = Math.max(1, ...data.map((item) => item.count));
  return (
    <div className="asn-rank">
      <div className="mini-title">{title}</div>
      {data.length === 0 ? (
        <div className="mini-empty">暂无数据</div>
      ) : (
        data.slice(0, 5).map((item) => (
          <div className="asn-row" key={item.asn}>
            <span className="asn-name">AS{item.asn}</span>
            <span className="asn-bar">
              <i style={{ width: `${Math.max(4, (item.count / max) * 100)}%` }} />
            </span>
            <span className="asn-count">{formatNumber(item.count)}</span>
          </div>
        ))
      )}
    </div>
  );
}

export function MessageMix({
  totals,
  topOrigins,
  topPeerAsns,
}: {
  totals: Totals;
  topOrigins: AsnCount[];
  topPeerAsns: AsnCount[];
}) {
  const segments: Segment[] = [
    { label: "路由监控", value: totals.routeMonitoring, color: "#22d3ee" },
    { label: "统计报告", value: totals.statisticsReport, color: "#818cf8" },
    { label: "Peer Up", value: totals.peerUp, color: "#34d399" },
    { label: "Peer Down", value: totals.peerDown, color: "#fb7185" },
    { label: "初始化", value: totals.initiation, color: "#fbbf24" },
    { label: "终止", value: totals.termination, color: "#94a3b8" },
    { label: "路由镜像", value: totals.routeMirroring, color: "#c084fc" },
    { label: "解析错误", value: totals.parseErrors, color: "#f87171" },
  ].filter((segment) => segment.value > 0);

  const total = segments.reduce((acc, segment) => acc + segment.value, 0);
  const totalLabel = formatCompact(total);
  const totalFontSize = totalLabel.length <= 5 ? 20 : totalLabel.length <= 7 ? 16 : 12;
  let offset = 0;

  return (
    <div className="mix-wrap">
      <div className="mix-body">
        <div className="donut-wrap">
          <svg
            viewBox="0 0 140 140"
            className="donut"
            role="img"
            aria-label={`BMP 消息类型构成，共 ${formatNumber(total)} 条消息`}
          >
            <circle cx="70" cy="70" r={R} fill="none" stroke="rgba(148,163,184,0.1)" strokeWidth="16" />
            {segments.map((segment) => {
              const length = (segment.value / total) * CIRC;
              const circle = (
                <circle
                  key={segment.label}
                  cx="70"
                  cy="70"
                  r={R}
                  fill="none"
                  stroke={segment.color}
                  strokeWidth="16"
                  strokeDasharray={`${Math.max(length - 1.6, 0)} ${CIRC - Math.max(length - 1.6, 0)}`}
                  strokeDashoffset={-offset}
                  transform="rotate(-90 70 70)"
                >
                  <title>{`${segment.label}: ${formatNumber(segment.value)}`}</title>
                </circle>
              );
              offset += length;
              return circle;
            })}
            <text
              x="70"
              y="68"
              textAnchor="middle"
              className="donut-value"
              style={{ fontSize: totalFontSize }}
            >
              {totalLabel}
            </text>
            <text x="70" y="85" textAnchor="middle" className="donut-label">
              条消息
            </text>
          </svg>
        </div>

        <div className="mix-legend">
          {segments.length === 0 && <div className="mini-empty">尚无消息</div>}
          {segments.map((segment) => (
            <div className="mix-row" key={segment.label}>
              <span className="legend-dot" style={{ background: segment.color }} />
              <span className="mix-name">{segment.label}</span>
              <span className="mix-count">{formatNumber(segment.value)}</span>
              <span className="mix-pct">
                {total > 0 ? `${((segment.value / total) * 100).toFixed(1)}%` : "0%"}
              </span>
            </div>
          ))}
        </div>
      </div>

      <div className="asn-columns">
        <AsnRank title="Top Origin AS（通告）" data={topOrigins} />
        <AsnRank title="Top Peer AS（消息量）" data={topPeerAsns} />
      </div>
    </div>
  );
}
