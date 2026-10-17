import Link from "next/link";
import type { InstanceStatus } from "@/lib/types";
import { formatNumber, timeAgo } from "@/lib/format";
import { GearIcon, RadioIcon, ServerIcon } from "./icons";

export function InstanceStrip({
  instances,
  selectedIp,
  onSelect,
  bmpBind,
  allowlistEnforced,
}: {
  instances: InstanceStatus[];
  selectedIp: string | null;
  onSelect: (routerIp: string | null) => void;
  bmpBind: string;
  allowlistEnforced: boolean;
}) {
  return (
    <section className="instance-strip" aria-label="BMP 接入实例">
      <div className="instance-strip-head">
        <div className="eyebrow">
          <ServerIcon size={14} /> BMP 接入实例
          <span className="endpoint">tcp://{bmpBind}</span>
        </div>
        <div className="instance-strip-meta">
          <span>{instances.length} 个已登记</span>
          <span className={`allowlist-pill ${allowlistEnforced ? "on" : ""}`}>
            {allowlistEnforced ? "来源白名单已启用" : "白名单未启用"}
          </span>
          <Link className="inline-link" href="/admin/">
            <GearIcon size={13} /> 管理
          </Link>
        </div>
      </div>

      <div className="instance-cards">
        {selectedIp && (
          <button className="instance-card instance-clear" onClick={() => onSelect(null)} type="button">
            <span className="instance-name">全部实例</span>
            <span className="instance-sub">清除筛选</span>
          </button>
        )}
        {instances.map((instance) => (
          <button
            key={instance.id}
            type="button"
            className={`instance-card${selectedIp === instance.routerIp ? " selected" : ""}${
              !instance.enabled ? " disabled" : ""
            }`}
            onClick={() => onSelect(selectedIp === instance.routerIp ? null : instance.routerIp)}
            title={instance.description || instance.name}
          >
            <span className={`status-dot ${instance.online ? "up" : instance.enabled ? "down" : "idle"}`} />
            <span className="instance-main">
              <span className="instance-name">{instance.name}</span>
              <span className="instance-sub mono">{instance.routerIp}</span>
            </span>
            <span className="instance-metrics">
              <span>
                <RadioIcon size={12} /> {instance.peers} peers
              </span>
              <span>{formatNumber(instance.messages)} msgs</span>
              <span>{instance.online ? "在线" : instance.lastSeen ? timeAgo(instance.lastSeen) : "未连接"}</span>
            </span>
          </button>
        ))}

        {instances.length === 0 && (
          <div className="instance-empty">
            <span>尚未登记 BIRD / BMP 实例，所有来源当前均可接入。</span>
            <Link className="inline-link" href="/admin/">
              前往管理员面板登记 <GearIcon size={13} />
            </Link>
          </div>
        )}
      </div>
    </section>
  );
}
