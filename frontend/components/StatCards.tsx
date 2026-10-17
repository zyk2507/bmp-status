import type { Overview } from "@/lib/types";
import type { RoutePulse } from "@/lib/useLive";
import { formatBytes, formatNumber, formatRate, truncateMiddle } from "@/lib/format";
import {
  ActivityIcon,
  AlertIcon,
  DatabaseIcon,
  LinkIcon,
  RouteIcon,
  UsersIcon,
} from "./icons";

interface CardProps {
  label: string;
  value: string;
  sub: string;
  icon: React.ReactNode;
  tone: string;
  danger?: boolean;
  pulse?: "up" | "down";
}

function KpiCard({ label, value, sub, icon, tone, danger, pulse }: CardProps) {
  return (
    <div className={`kpi-card${danger ? " kpi-danger" : ""}${pulse ? ` pulse-${pulse}` : ""}`}>
      <div className="kpi-top">
        <span className="kpi-label">{label}</span>
        <span className={`kpi-icon ${tone}`}>{icon}</span>
      </div>
      <div className="kpi-value">{value}</div>
      <div className="kpi-sub" title={sub}>
        {sub}
      </div>
    </div>
  );
}

export function StatCards({ overview, routePulse }: { overview: Overview; routePulse: RoutePulse | null }) {
  const { totals } = overview;
  return (
    <section className="kpi-grid" aria-label="关键指标">
      <KpiCard
        label="消息总数"
        value={formatNumber(totals.messages)}
        sub={`${formatBytes(totals.bytes)} · ↑${formatNumber(totals.announcements)} / ↓${formatNumber(totals.withdrawals)}`}
        icon={<ActivityIcon size={18} />}
        tone="tone-cyan"
      />
      <KpiCard
        label="实时速率"
        value={`${formatRate(overview.ratePerSec)}/s`}
        sub="最近 10 秒平均消息速率"
        icon={<ActivityIcon size={18} />}
        tone="tone-indigo"
      />
      <KpiCard
        key={`active-routes-${routePulse?.seq ?? 0}`}
        pulse={routePulse?.kind}
        label="活跃前缀"
        value={formatNumber(overview.activeRoutes)}
        sub="本地重建的 Adj-RIB-In"
        icon={<RouteIcon size={18} />}
        tone="tone-teal"
      />
      <KpiCard
        label="BGP 对等体"
        value={`${overview.peersUp}/${overview.peersTotal}`}
        sub="在线会话 / 已发现对等体"
        icon={<UsersIcon size={18} />}
        tone="tone-emerald"
      />
      <KpiCard
        label="BMP 连接"
        value={formatNumber(overview.activeConnections)}
        sub={`累计 ${formatNumber(overview.connectionsTotal)}${
          totals.rejectedConnections > 0
            ? ` · 拒绝 ${formatNumber(totals.rejectedConnections)}`
            : ""
        }`}
        icon={<LinkIcon size={18} />}
        tone="tone-blue"
      />
      <KpiCard
        label="解析错误"
        value={formatNumber(totals.parseErrors)}
        sub={
          overview.lastError
            ? truncateMiddle(overview.lastError, 46)
            : "bgpkit-parser 运行正常"
        }
        icon={totals.parseErrors > 0 ? <AlertIcon size={18} /> : <DatabaseIcon size={18} />}
        tone="tone-rose"
        danger={totals.parseErrors > 0}
      />
    </section>
  );
}
