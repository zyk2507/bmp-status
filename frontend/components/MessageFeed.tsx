"use client";

import { useMemo, useState } from "react";
import type { InstanceStatus, MessageView } from "@/lib/types";
import { formatClockMs, formatNumber, msgMeta } from "@/lib/format";
import { CloseIcon, PauseIcon, PlayIcon, SearchIcon } from "./icons";

const TABS = [
  { id: "all", label: "全部" },
  { id: "updates", label: "路由更新" },
  { id: "statistics_report", label: "统计" },
  { id: "peer", label: "对等体" },
  { id: "errors", label: "错误" },
];

function tabMatches(message: MessageView, tab: string): boolean {
  if (tab === "all") return true;
  if (tab === "updates") return message.msgType === "route_monitoring";
  if (tab === "peer") return message.msgType === "peer_up" || message.msgType === "peer_down";
  if (tab === "errors") return Boolean(message.error);
  return message.msgType === tab;
}

function searchMatches(message: MessageView, query: string): boolean {
  if (!query) return true;
  const needle = query.toLowerCase();
  const haystack = [
    message.peer?.ip,
    message.peer ? `AS${message.peer.asn}` : null,
    message.routerAddr,
    message.routerName,
    message.bgpType,
    message.error,
    message.peerDownReason,
    message.terminationReason,
    ...message.routes.flatMap((route) => [
      route.prefix,
      route.asPathStr,
      route.nextHop,
      route.origin,
      ...route.communities,
    ]),
    ...message.stats.map((stat) => stat.name),
    ...message.tlvs.map((tlv) => tlv.value),
  ]
    .filter(Boolean)
    .join(" ")
    .toLowerCase();
  return haystack.includes(needle);
}

interface Summary {
  main: string;
  sub: string;
}

function summarize(message: MessageView): Summary {
  switch (message.msgType) {
    case "route_monitoring": {
      const parts: string[] = [];
      if (message.announced > 0) parts.push(`↑ ${message.announced} 条通告`);
      if (message.withdrawn > 0) parts.push(`↓ ${message.withdrawn} 条撤销`);
      if (parts.length === 0) parts.push(message.bgpType ?? "空更新");
      const preview = message.routes
        .slice(0, 3)
        .map((route) => `${route.action === "withdraw" ? "−" : "+"}${route.prefix}`)
        .join("  ");
      return {
        main: parts.join("  "),
        sub: preview || (message.bgpType?.includes("End-of-RIB") ? "End-of-RIB 标记" : "无 NLRI"),
      };
    }
    case "statistics_report": {
      const first = message.stats[0];
      return {
        main: first ? `${first.name}` : "统计报告",
        sub: first
          ? `${first.value}${message.stats.length > 1 ? ` · 共 ${message.stats.length} 项` : ""}`
          : `${message.stats.length} 项计数器`,
      };
    }
    case "peer_up":
      return {
        main: `会话建立 · 本地 ${message.peerUp?.localAddr ?? "—"}:${message.peerUp?.localPort ?? "—"}`,
        sub: `hold ${message.peerUp?.sentOpen?.holdTime ?? "—"}s · OPEN AS${message.peerUp?.sentOpen?.asn ?? "—"}`,
      };
    case "peer_down":
      return {
        main: message.peerDownReason ?? "对等体会话断开",
        sub: message.peerDownDetail ?? "路由器报告 Peer Down",
      };
    case "initiation":
      return {
        main:
          message.tlvs.find((tlv) => tlv.name === "SysName")?.value ??
          "BMP 初始化消息",
        sub:
          message.tlvs.find((tlv) => tlv.name === "SysDescr")?.value ??
          message.routerAddr,
      };
    case "termination":
      return {
        main: message.terminationReason ?? "BMP 终止消息",
        sub: "路由器主动关闭 BMP 会话",
      };
    case "route_mirroring":
      return {
        main: "路由镜像消息",
        sub: message.bgpType ?? "TLV 数据",
      };
    case "parse_error":
    case "protocol_error":
      return {
        main: message.error ?? "消息解析失败",
        sub: `类型码 ${message.msgTypeCode} · ${message.size} 字节`,
      };
    default:
      return { main: message.bgpType ?? message.msgType, sub: "—" };
  }
}

export function MessageFeed({
  messages,
  instances,
  paused,
  onTogglePause,
  onSelect,
  peerFilterLabel,
  instanceFilterLabel,
  onClearFilters,
}: {
  messages: MessageView[];
  instances: InstanceStatus[];
  paused: boolean;
  onTogglePause: () => void;
  onSelect: (message: MessageView) => void;
  peerFilterLabel: string | null;
  instanceFilterLabel: string | null;
  onClearFilters: () => void;
}) {
  const [tab, setTab] = useState("all");
  const [query, setQuery] = useState("");
  const instanceByIp = useMemo(
    () => Object.fromEntries(instances.map((instance) => [instance.routerIp, instance.name])),
    [instances],
  );

  const filtered = useMemo(
    () =>
      messages
        .filter((message) => tabMatches(message, tab) && searchMatches(message, query))
        .slice(0, 200),
    [messages, tab, query],
  );

  return (
    <div className="feed">
      <div className="feed-toolbar">
        <div className="tabs" role="tablist" aria-label="消息类型筛选">
          {TABS.map((item) => (
            <button
              key={item.id}
              type="button"
              role="tab"
              aria-selected={tab === item.id}
              className={`tab${tab === item.id ? " active" : ""}`}
              onClick={() => setTab(item.id)}
            >
              {item.label}
            </button>
          ))}
        </div>
        <div className="feed-actions">
          <label className="search search-small">
            <SearchIcon size={14} />
            <input
              value={query}
              onChange={(event) => setQuery(event.target.value)}
              placeholder="搜索前缀 / AS / 对等体"
              aria-label="搜索消息"
            />
            {query && (
              <button type="button" className="clear-input" onClick={() => setQuery("")} aria-label="清空搜索">
                <CloseIcon size={12} />
              </button>
            )}
          </label>
          <button
            type="button"
            className={`btn btn-sm ${paused ? "btn-primary" : "btn-ghost"}`}
            onClick={onTogglePause}
            title={paused ? "恢复实时消息流" : "暂停实时消息流"}
          >
            {paused ? <PlayIcon size={13} /> : <PauseIcon size={13} />}
            {paused ? "已暂停" : "暂停"}
          </button>
        </div>
      </div>

      {(peerFilterLabel || instanceFilterLabel) && (
        <div className="filter-bar">
          {instanceFilterLabel && <span className="filter-chip">实例：{instanceFilterLabel}</span>}
          {peerFilterLabel && <span className="filter-chip">对等体：{peerFilterLabel}</span>}
          <button type="button" className="text-button" onClick={onClearFilters}>
            清除筛选
          </button>
        </div>
      )}

      <div className="feed-table">
        <div className="feed-header">
          <span>时间</span>
          <span>类型</span>
          <span>对等体 / 来源</span>
          <span>摘要</span>
          <span className="right">前缀</span>
        </div>
        <div className="feed-scroll">
          {filtered.map((message) => {
            const meta = msgMeta(message.msgType);
            const summary = summarize(message);
            const instanceName = instanceByIp[message.routerAddr];
            return (
              <button
                key={message.id}
                type="button"
                className="feed-row"
                onClick={() => onSelect(message)}
              >
                <span className="feed-time mono" title={new Date(message.ts * 1000).toLocaleString("zh-CN")}>
                  {formatClockMs(message.ts)}
                </span>
                <span
                  className={`type-badge ${meta.className}`}
                  style={{ "--type-color": meta.color } as React.CSSProperties}
                  title={meta.label}
                >
                  {meta.short}
                </span>
                <span className="feed-peer">
                  {message.peer ? (
                    <>
                      <span className="feed-peer-main mono">{message.peer.ip}</span>
                      <span className="feed-peer-sub">
                        AS{message.peer.asn} · {instanceName ?? message.routerName ?? message.routerAddr}
                      </span>
                    </>
                  ) : (
                    <>
                      <span className="feed-peer-main">{message.routerName ?? "BMP 会话"}</span>
                      <span className="feed-peer-sub">
                        {instanceName ?? message.routerAddr}
                        {message.tlvs.length > 0 ? ` · ${message.tlvs.length} TLV` : ""}
                      </span>
                    </>
                  )}
                </span>
                <span className={`feed-summary${message.error ? " has-error" : ""}`}>
                  <span className="feed-main" title={summary.main}>{summary.main}</span>
                  <span className="feed-sub" title={summary.sub}>{summary.sub}</span>
                </span>
                <span className="feed-count">
                  {message.routesTotal > 0 ? (
                    <>
                      <b>{formatNumber(message.routesTotal)}</b>
                      <em>{message.withdrawn > 0 && message.announced === 0 ? "撤销" : "更新"}</em>
                    </>
                  ) : (
                    <span className="muted">—</span>
                  )}
                </span>
              </button>
            );
          })}

          {filtered.length === 0 && (
            <div className="panel-empty">
              {messages.length === 0
                ? paused
                  ? "实时消息流已暂停。点击“已暂停”恢复。"
                  : "等待第一条 BMP 消息…"
                : "没有匹配当前筛选条件的消息。"}
            </div>
          )}
        </div>
      </div>

      <div className="feed-footer">
        <span>
          显示 {filtered.length} 条 · 共缓存 {messages.length} 条 · 点击任意行查看原始消息
        </span>
        <span className={`feed-live ${paused ? "paused" : ""}`}>
          <i /> {paused ? "PAUSED" : "LIVE"}
        </span>
      </div>
    </div>
  );
}
