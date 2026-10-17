"use client";

import { useEffect, useState } from "react";
import type { InstanceStatus, MessageView, OpenInfo } from "@/lib/types";
import {
  formatBytes,
  formatDateTime,
  formatNumber,
  msgMeta,
} from "@/lib/format";
import { CheckIcon, CloseIcon, CopyIcon } from "./icons";

function sessionLabel(session: string): string {
  if (session === "up") return "UP";
  if (session === "down") return "DOWN";
  return "UNKNOWN";
}

async function copyText(text: string): Promise<boolean> {
  try {
    await navigator.clipboard.writeText(text);
    return true;
  } catch {
    try {
      const textarea = document.createElement("textarea");
      textarea.value = text;
      textarea.style.position = "fixed";
      textarea.style.opacity = "0";
      document.body.appendChild(textarea);
      textarea.select();
      const ok = document.execCommand("copy");
      document.body.removeChild(textarea);
      return ok;
    } catch {
      return false;
    }
  }
}

export function MessageDetail({
  message,
  instances,
  onClose,
}: {
  message: MessageView;
  instances: InstanceStatus[];
  onClose: () => void;
}) {
  const [copied, setCopied] = useState(false);
  const meta = msgMeta(message.msgType);
  const instanceName = instances.find((item) => item.routerIp === message.routerAddr)?.name;

  useEffect(() => {
    const handler = (event: KeyboardEvent) => {
      if (event.key === "Escape") onClose();
    };
    window.addEventListener("keydown", handler);
    const previous = document.body.style.overflow;
    document.body.style.overflow = "hidden";
    return () => {
      window.removeEventListener("keydown", handler);
      document.body.style.overflow = previous;
    };
  }, [onClose]);

  const handleCopy = async () => {
    if (await copyText(message.rawHex)) {
      setCopied(true);
      setTimeout(() => setCopied(false), 1600);
    }
  };

  return (
    <div className="drawer-overlay" onMouseDown={onClose} role="presentation">
      <aside
        className="drawer"
        role="dialog"
        aria-modal="true"
        aria-label="BMP 消息详情"
        onMouseDown={(event) => event.stopPropagation()}
      >
        <header className="drawer-head">
          <div className="drawer-title">
            <span
              className={`type-badge ${meta.className}`}
              style={{ "--type-color": meta.color } as React.CSSProperties}
            >
              {meta.short}
            </span>
            <div>
              <h2>{meta.label}</h2>
              <p>
                #{message.id} · {formatDateTime(message.ts)} · {formatBytes(message.size)}
              </p>
            </div>
          </div>
          <button className="icon-button" type="button" onClick={onClose} aria-label="关闭详情">
            <CloseIcon size={18} />
          </button>
        </header>

        <div className="drawer-body">
          <section className="detail-grid">
            <div>
              <span>BMP 类型码</span>
              <b>{message.msgTypeCode}</b>
            </div>
            <div>
              <span>接收时间</span>
              <b>{formatDateTime(message.receivedAt)}</b>
            </div>
            <div>
              <span>来源路由器</span>
              <b>{instanceName ?? message.routerName ?? message.routerAddr}</b>
              <small>{message.routerAddr}</small>
            </div>
            <div>
              <span>消息大小</span>
              <b>{formatNumber(message.size)} B</b>
            </div>
            {message.bgpType && (
              <div>
                <span>内嵌 BGP</span>
                <b>{message.bgpType}</b>
              </div>
            )}
            <div>
              <span>路由变更</span>
              <b>
                ↑{message.announced} · ↓{message.withdrawn}
              </b>
            </div>
          </section>

          {message.peer && (
            <section className="detail-section">
              <div className="section-title">
                对等体
                <span className={`session-pill ${message.peer.session}`}>
                  {sessionLabel(message.peer.session)}
                </span>
              </div>
              <div className="detail-grid compact">
                <div>
                  <span>地址</span>
                  <b className="mono">{message.peer.ip}</b>
                </div>
                <div>
                  <span>ASN</span>
                  <b>AS{message.peer.asn}</b>
                </div>
                <div>
                  <span>BGP ID</span>
                  <b className="mono">{message.peer.bgpId}</b>
                </div>
                <div>
                  <span>Peer Key</span>
                  <b className="mono small">{message.peer.key}</b>
                </div>
              </div>
              <div className="badge-row">
                {message.peer.postPolicy && <span className="badge violet">post-policy</span>}
                {message.peer.adjRibOut && <span className="badge amber">adj-rib-out</span>}
                {message.peer.ipv6 && <span className="badge subtle">IPv6</span>}
                {message.peer.asn16bit && <span className="badge subtle">2-byte ASN</span>}
                {!message.peer.postPolicy && !message.peer.adjRibOut && (
                  <span className="badge subtle">pre-policy · Adj-RIB-In</span>
                )}
              </div>
            </section>
          )}

          {message.error && (
            <section className="detail-section error-box">
              <div className="section-title">解析错误</div>
              <p className="mono">{message.error}</p>
            </section>
          )}

          {message.routes.length > 0 && (
            <section className="detail-section">
              <div className="section-title">
                路由明细
                <span className="count-pill">
                  {message.routesTotal} 条{message.routesTruncated > 0 ? ` · 仅展示 ${message.routes.length}` : ""}
                </span>
              </div>
              <div className="detail-routes">
                <div className="detail-routes-head">
                  <span>前缀</span>
                  <span>动作</span>
                  <span>下一跳</span>
                  <span>AS Path</span>
                  <span>Origin</span>
                </div>
                {message.routes.map((route, index) => (
                  <div className="detail-routes-row" key={`${route.prefix}-${route.pathId ?? 0}-${index}`}>
                    <span className="mono">{route.prefix}</span>
                    <span>
                      <span className={`route-action ${route.action}`}>
                        {route.action === "announce" ? "通告" : "撤销"}
                      </span>
                    </span>
                    <span className="mono">{route.nextHop ?? "—"}</span>
                    <span className="mono as-path" title={route.asPathStr}>
                      {route.asPathStr || "—"}
                    </span>
                    <span>{route.origin ?? "—"}</span>
                  </div>
                ))}
              </div>
              {message.routes[0]?.communities.length ? (
                <div className="chips">
                  {message.routes[0].communities.map((community) => (
                    <span className="chip mono" key={community}>
                      {community}
                    </span>
                  ))}
                </div>
              ) : null}
            </section>
          )}

          {message.stats.length > 0 && (
            <section className="detail-section">
              <div className="section-title">统计计数器</div>
              <div className="stat-list">
                {message.stats.map((stat) => (
                  <div className="stat-row" key={`${stat.code}-${stat.name}`}>
                    <span className="stat-code">{stat.code}</span>
                    <span className="stat-name">{stat.name}</span>
                    <span className={`stat-kind ${stat.kind}`}>{stat.kind}</span>
                    <span className="stat-value mono">{stat.value}</span>
                  </div>
                ))}
              </div>
            </section>
          )}

          {message.peerUp && (
            <section className="detail-section">
              <div className="section-title">会话建立信息</div>
              <div className="detail-grid compact">
                <div>
                  <span>本地地址</span>
                  <b className="mono">
                    {message.peerUp.localAddr}:{message.peerUp.localPort}
                  </b>
                </div>
                <div>
                  <span>远端端口</span>
                  <b className="mono">{message.peerUp.remotePort}</b>
                </div>
              </div>
              <div className="open-grid">
                {(
                  [
                    ["发送 OPEN", message.peerUp.sentOpen],
                    ["接收 OPEN", message.peerUp.receivedOpen],
                  ] as Array<[string, OpenInfo | null]>
                ).map(([label, open]) => (
                    <div className="open-card" key={label}>
                      <div className="mini-title">{label}</div>
                      {open ? (
                        <>
                          <div className="open-line">
                            AS{open.asn} · hold {open.holdTime}s · ID {open.bgpId}
                          </div>
                          <div className="chips">
                            {open.capabilities.length > 0 ? (
                              open.capabilities.map((capability) => (
                                <span className="chip" key={capability}>
                                  {capability}
                                </span>
                              ))
                            ) : (
                              <span className="muted">无 capability</span>
                            )}
                          </div>
                        </>
                      ) : (
                        <span className="muted">非 OPEN 消息</span>
                      )}
                    </div>
                  ))}
              </div>
            </section>
          )}

          {message.tlvs.length > 0 && (
            <section className="detail-section">
              <div className="section-title">TLV</div>
              <div className="tlv-list">
                {message.tlvs.map((tlv, index) => (
                  <div className="tlv-row" key={`${tlv.ty}-${index}`}>
                    <span className="tlv-type">
                      {tlv.ty} · {tlv.name}
                    </span>
                    <span className="mono">{tlv.value}</span>
                  </div>
                ))}
              </div>
            </section>
          )}

          {(message.peerDownReason || message.terminationReason) && (
            <section className="detail-section">
              <div className="section-title">原因</div>
              <p>{message.peerDownReason ?? message.terminationReason}</p>
              {message.peerDownDetail && <p className="mono muted">{message.peerDownDetail}</p>}
            </section>
          )}

          <section className="detail-section">
            <div className="section-title">
              原始消息（hex）
              <button className="text-button" type="button" onClick={handleCopy}>
                {copied ? <CheckIcon size={13} /> : <CopyIcon size={13} />}
                {copied ? "已复制" : "复制"}
              </button>
            </div>
            <pre className="raw-hex">{message.rawHex || "—"}</pre>
            {message.rawTruncated && (
              <p className="muted small">为控制内存占用，仅保留消息的前 1024 字节。</p>
            )}
          </section>
        </div>
      </aside>
    </div>
  );
}
