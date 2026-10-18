export interface MsgMeta {
  label: string;
  short: string;
  color: string;
  className: string;
}

export const MSG_META: Record<string, MsgMeta> = {
  route_monitoring: {
    label: "路由监控",
    short: "UPDATE",
    color: "#22d3ee",
    className: "type-route",
  },
  statistics_report: {
    label: "统计报告",
    short: "STATS",
    color: "#818cf8",
    className: "type-stats",
  },
  peer_up: { label: "对等体 UP", short: "PEER UP", color: "#34d399", className: "type-up" },
  peer_down: { label: "对等体 DOWN", short: "PEER DOWN", color: "#fb7185", className: "type-down" },
  initiation: { label: "初始化", short: "INIT", color: "#fbbf24", className: "type-init" },
  termination: { label: "终止", short: "TERM", color: "#94a3b8", className: "type-term" },
  route_mirroring: { label: "路由镜像", short: "MIRROR", color: "#c084fc", className: "type-mirror" },
  parse_error: { label: "解析错误", short: "ERROR", color: "#f87171", className: "type-error" },
  protocol_error: { label: "协议错误", short: "ERROR", color: "#f87171", className: "type-error" },
};

export function msgMeta(type: string): MsgMeta {
  return (
    MSG_META[type] ?? {
      label: type,
      short: type.toUpperCase(),
      color: "#94a3b8",
      className: "type-other",
    }
  );
}

const grouped = new Intl.NumberFormat("zh-CN");
const compact = new Intl.NumberFormat("zh-CN", {
  notation: "compact",
  maximumFractionDigits: 1,
});

export function formatNumber(value: number): string {
  if (!Number.isFinite(value)) return "0";
  return grouped.format(Math.round(value));
}

export function formatCompact(value: number): string {
  if (!Number.isFinite(value)) return "0";
  return compact.format(value);
}

export function formatRate(value: number): string {
  if (!Number.isFinite(value)) return "0.0";
  if (value >= 100) return value.toFixed(0);
  if (value >= 10) return value.toFixed(1);
  return value.toFixed(2);
}

export function formatBytes(bytes: number): string {
  if (!Number.isFinite(bytes) || bytes <= 0) return "0 B";
  const units = ["B", "KB", "MB", "GB", "TB"];
  let value = bytes;
  let unit = 0;
  while (value >= 1024 && unit < units.length - 1) {
    value /= 1024;
    unit += 1;
  }
  return `${value >= 100 ? value.toFixed(0) : value.toFixed(1)} ${units[unit]}`;
}

export function formatClock(ts: number): string {
  if (!ts) return "--:--:--";
  return new Date(ts * 1000).toLocaleTimeString("zh-CN", {
    hour12: false,
    hour: "2-digit",
    minute: "2-digit",
    second: "2-digit",
  });
}

export function formatClockMs(ts: number): string {
  if (!ts) return "--:--:--.---";
  const date = new Date(ts * 1000);
  const ms = date.getMilliseconds().toString().padStart(3, "0");
  return `${formatClock(ts)}.${ms}`;
}

export function formatDateTime(ts: number): string {
  if (!ts) return "—";
  return new Date(ts * 1000).toLocaleString("zh-CN", { hour12: false });
}

export function timeAgo(ts: number | null | undefined): string {
  if (!ts) return "—";
  const diff = Math.max(0, Date.now() / 1000 - ts);
  if (diff < 2) return "刚刚";
  if (diff < 60) return `${Math.floor(diff)} 秒前`;
  if (diff < 3600) return `${Math.floor(diff / 60)} 分钟前`;
  if (diff < 86400) return `${Math.floor(diff / 3600)} 小时前`;
  return `${Math.floor(diff / 86400)} 天前`;
}

export function formatUptime(seconds: number): string {
  if (!Number.isFinite(seconds) || seconds < 0) return "0s";
  const d = Math.floor(seconds / 86400);
  const h = Math.floor((seconds % 86400) / 3600);
  const m = Math.floor((seconds % 3600) / 60);
  if (d > 0) return `${d} 天 ${h} 小时`;
  if (h > 0) return `${h} 小时 ${m} 分`;
  if (m > 0) return `${m} 分 ${Math.floor(seconds % 60)} 秒`;
  return `${Math.floor(seconds)} 秒`;
}

export function truncateMiddle(value: string, max = 42): string {
  if (value.length <= max) return value;
  const head = Math.ceil((max - 1) / 2);
  const tail = Math.floor((max - 1) / 2);
  return `${value.slice(0, head)}…${value.slice(-tail)}`;
}

export function formatAsn(asn: number): string {
  return `AS${asn}`;
}
