"use client";

import Link from "next/link";
import { useEffect, useMemo, useState } from "react";
import type { MessageView, Overview } from "@/lib/types";
import { useLive } from "@/lib/useLive";
import { formatNumber, formatUptime, timeAgo } from "@/lib/format";
import { ClockIcon, GearIcon, LogoutIcon, RadioIcon, RefreshIcon, WifiOffIcon } from "./icons";
import { LoginScreen } from "./LoginScreen";
import { useAuth } from "@/lib/useAuth";
import { StatCards } from "./StatCards";
import { InstanceStrip } from "./InstanceStrip";
import { TrafficChart } from "./TrafficChart";
import { MessageMix } from "./MessageMix";
import { PeersPanel } from "./PeersPanel";
import { MessageFeed } from "./MessageFeed";
import { RouteTable } from "./RouteTable";
import { MessageDetail } from "./MessageDetail";

function Header({
  overview,
  status,
  onRefresh,
  onLogout,
}: {
  overview: Overview | null;
  status: "connecting" | "live" | "offline";
  onRefresh: () => void;
  onLogout: () => Promise<void>;
}) {
  const [clock, setClock] = useState("--:--:--");

  useEffect(() => {
    const tick = () =>
      setClock(
        new Date().toLocaleTimeString("zh-CN", {
          hour12: false,
          hour: "2-digit",
          minute: "2-digit",
          second: "2-digit",
        }),
      );
    tick();
    const timer = setInterval(tick, 1000);
    return () => clearInterval(timer);
  }, []);

  const label =
    status === "live" ? "LIVE" : status === "connecting" ? "CONNECTING" : "OFFLINE";

  return (
    <header className="topbar">
      <div className="brand">
        <div className="brand-mark">
          <RadioIcon size={22} />
        </div>
        <div className="brand-text">
          <h1>
            BMP Status <span className="version">v1.0</span>
          </h1>
          <p>BGP Monitoring Protocol · 实时路由观测</p>
        </div>
      </div>

      <div className="topbar-right">
        {overview?.demo.running && (
          <span className="demo-badge" title="管理员启动的合成 BMP 数据流">
            DEMO · {formatNumber(overview.demo.sent)}
          </span>
        )}
        <span className="endpoint-chip mono" title="BMP TCP 监听地址">
          tcp://{overview?.bmpBind ?? "—"}
        </span>
        <span className={`status-pill ${status}`}>
          <i />
          {label}
        </span>
        <span className="clock mono">
          <ClockIcon size={14} />
          {clock}
        </span>
        <button className="icon-button" type="button" onClick={onRefresh} title="刷新数据" aria-label="刷新数据">
          <RefreshIcon size={16} />
        </button>
        <Link className="icon-button" href="/admin/" title="管理员面板" aria-label="管理员面板">
          <GearIcon size={17} />
        </Link>
        <button
          className="icon-button"
          type="button"
          onClick={() => void onLogout()}
          title="退出登录"
          aria-label="退出登录"
        >
          <LogoutIcon size={16} />
        </button>
      </div>
    </header>
  );
}

function PanelHead({
  title,
  subtitle,
  right,
}: {
  title: string;
  subtitle?: string;
  right?: React.ReactNode;
}) {
  return (
    <div className="panel-head">
      <div>
        <h2>{title}</h2>
        {subtitle && <p>{subtitle}</p>}
      </div>
      {right}
    </div>
  );
}

function EmptyState({ overview }: { overview: Overview }) {
  return (
    <section className="panel empty-state">
      <div className="empty-orbit">
        <span className="orbit-ring" />
        <RadioIcon size={32} />
      </div>
      <h2>等待 BMP 数据接入</h2>
      <p className="empty-lead">
        将 BIRD、FRR 或交换机的 BMP station 指向本服务，消息会实时解析并展示在仪表盘上。
      </p>
      <div className="empty-endpoint">
        <span>BMP 监听地址</span>
        <code>tcp://{overview.bmpBind}</code>
      </div>

      <div className="empty-config">
        <div>
          <h3>FRR</h3>
          <pre>{`bmp targets bmp-status
 bmp connect <server-ip> port ${overview.bmpBind.split(":").pop()}
 bmp monitor ipv4 unicast pre-policy`}</pre>
        </div>
        <div>
          <h3>BIRD</h3>
          <pre>{`# BIRD 2.16+ 的 bmp 协议段
protocol bmp {
  station address ip <server-ip> port ${overview.bmpBind.split(":").pop()};
  monitoring rib in pre_policy;
}`}</pre>
        </div>
      </div>

      <div className="empty-actions">
        <Link className="btn btn-primary" href="/admin/">
          登记 BIRD 实例
        </Link>
        <span className="muted">
          收到 Peer Up / Route Monitoring 后，对等体和前缀会自动出现。
        </span>
      </div>
    </section>
  );
}

function LoadingPanel({ status }: { status: string }) {
  return (
    <div className="panel loading-panel">
      <div className="spinner" />
      <h2>正在连接 BMP 收集器…</h2>
      <p>WebSocket 状态：{status}</p>
    </div>
  );
}

function DashboardView({ onLogout }: { onLogout: () => Promise<void> }) {
  const {
    overview,
    messages,
    routePulse,
    status,
    paused,
    setPaused,
    lastEventAt,
    error,
    refresh,
  } = useLive();

  const [selected, setSelected] = useState<MessageView | null>(null);
  const [peerFilter, setPeerFilter] = useState<string | null>(null);
  const [instanceFilter, setInstanceFilter] = useState<string | null>(null);

  const instances = overview?.instances ?? [];
  const instanceByIp = useMemo(
    () => Object.fromEntries(instances.map((instance) => [instance.routerIp, instance.name])),
    [instances],
  );
  const visiblePeers = useMemo(
    () => (overview?.peers ?? []).filter((peer) => !instanceFilter || peer.router === instanceFilter),
    [overview?.peers, instanceFilter],
  );

  const visibleMessages = useMemo(
    () =>
      messages.filter(
        (message) =>
          (!peerFilter || message.peer?.key === peerFilter) &&
          (!instanceFilter || message.routerAddr === instanceFilter),
      ),
    [messages, peerFilter, instanceFilter],
  );

  const peerLabel = peerFilter
    ? overview?.peers.find((peer) => peer.key === peerFilter)?.ip ?? peerFilter
    : null;
  const instanceLabel = instanceFilter
    ? instanceByIp[instanceFilter] ?? instanceFilter
    : null;

  const clearFilters = () => {
    setPeerFilter(null);
    setInstanceFilter(null);
  };

  return (
    <main className="shell">
      <Header overview={overview} status={status} onRefresh={refresh} onLogout={onLogout} />

      {error && (
        <div className="alert alert-error">
          <WifiOffIcon size={16} />
          <span>无法连接后端：{error}。请确认 bmp-status 正在运行。</span>
        </div>
      )}

      {!overview ? (
        <LoadingPanel status={status} />
      ) : (
        <>
          <StatCards overview={overview} routePulse={routePulse} />

          <InstanceStrip
            instances={instances}
            selectedIp={instanceFilter}
            onSelect={(routerIp) => {
              setInstanceFilter(routerIp);
              setPeerFilter(null);
            }}
            bmpBind={overview.bmpBind}
            allowlistEnforced={overview.allowlistEnforced}
          />

          {overview.totals.messages === 0 ? (
            <EmptyState overview={overview} />
          ) : (
            <>
              <div className="panel-grid chart-grid">
                <section className="panel">
                  <PanelHead
                    title="流量与更新速率"
                    subtitle="最近 120 秒 · 每秒聚合"
                    right={
                      <span className={`live-badge${status === "live" ? "" : " offline"}`}>
                        <i /> {status === "live" ? "实时" : "轮询"}
                      </span>
                    }
                  />
                  <div className="panel-body no-pad-top">
                    <TrafficChart buckets={overview.buckets} />
                  </div>
                </section>

                <section className="panel">
                  <PanelHead title="消息构成" subtitle="按 BMP 消息类型累计" />
                  <div className="panel-body">
                    <MessageMix
                      totals={overview.totals}
                      topOrigins={overview.topOrigins}
                      topPeerAsns={overview.topPeerAsns}
                    />
                  </div>
                </section>
              </div>

              <div className="panel-grid main-grid">
                <section className="panel">
                  <PanelHead
                    title="BGP 对等体"
                    subtitle="会话状态与本地重建的 Adj-RIB-In"
                    right={
                      <span className="panel-badge">
                        {overview.peersUp} UP / {overview.peersTotal} 总计
                      </span>
                    }
                  />
                  <PeersPanel
                    peers={visiblePeers}
                    instanceByIp={instanceByIp}
                    selectedKey={peerFilter}
                    onSelect={(key) => setPeerFilter(key)}
                  />
                </section>

                <section className="panel">
                  <PanelHead
                    title="实时消息流"
                    subtitle={
                      lastEventAt
                        ? `最近事件 ${timeAgo(lastEventAt / 1000)}`
                        : "等待第一条 BMP 消息"
                    }
                    right={
                      <span className={`live-badge${paused ? " paused" : ""}`}>
                        <i /> {paused ? "已暂停" : "LIVE"}
                      </span>
                    }
                  />
                  <MessageFeed
                    messages={visibleMessages}
                    instances={instances}
                    paused={paused}
                    onTogglePause={() => setPaused(!paused)}
                    onSelect={setSelected}
                    peerFilterLabel={peerLabel}
                    instanceFilterLabel={instanceLabel}
                    onClearFilters={clearFilters}
                  />
                </section>
              </div>

              <section className="panel">
                <PanelHead
                  title="前缀表"
                  subtitle="由 Route Monitoring 通告与撤销消息实时重建"
                  right={
                    <span className="panel-badge">
                      {formatNumber(overview.activeRoutes)} 条活跃路由
                    </span>
                  }
                />
                <RouteTable
                  totalActive={overview.activeRoutes}
                  peerFilter={peerFilter}
                  instanceFilter={instanceFilter}
                  peerFilterLabel={peerLabel}
                  instanceFilterLabel={instanceLabel}
                  onClearFilters={clearFilters}
                />
              </section>
            </>
          )}
        </>
      )}

      <footer className="footer">
        <div>
          <span className="footer-brand">BMP Status</span>
          <span>bgpkit-parser 0.22 · axum · Next.js</span>
        </div>
        <div>
          {overview && <span>运行 {formatUptime(overview.uptimeSeconds)}</span>}
          <Link href="/admin/">管理员面板</Link>
          <span className="muted">BMP tcp://{overview?.bmpBind ?? "—"}</span>
        </div>
      </footer>

      {selected && (
        <MessageDetail
          message={selected}
          instances={instances}
          onClose={() => setSelected(null)}
        />
      )}
    </main>
  );
}


function SessionLoading() {
  return (
    <div className="login-wrap">
      <div className="login-card">
        <div className="spinner" style={{ margin: "0 auto 14px" }} />
        <p style={{ textAlign: "center", marginBottom: 0 }}>正在验证页面会话…</p>
      </div>
    </div>
  );
}

export default function Dashboard() {
  const auth = useAuth();

  if (auth.status === "loading") return <SessionLoading />;
  if (auth.status === "unauthenticated") return <LoginScreen onLogin={auth.login} />;
  return <DashboardView onLogout={auth.logout} />;
}
