"use client";

import { useCallback, useEffect, useRef, useState } from "react";
import type { RoutesResponse } from "@/lib/types";
import { fetchJson } from "@/lib/api";
import { formatNumber, timeAgo } from "@/lib/format";
import {
  ChevronLeftIcon,
  ChevronRightIcon,
  CloseIcon,
  SearchIcon,
} from "./icons";

const PAGE_SIZE = 50;
const SEARCH_DEBOUNCE_MS = 300;

export function RouteTable({
  totalActive,
  peerFilter,
  instanceFilter,
  peerFilterLabel,
  instanceFilterLabel,
  onClearFilters,
}: {
  totalActive: number;
  peerFilter: string | null;
  instanceFilter: string | null;
  peerFilterLabel: string | null;
  instanceFilterLabel: string | null;
  onClearFilters: () => void;
}) {
  const [query, setQuery] = useState("");
  const [debouncedQuery, setDebouncedQuery] = useState("");
  const [page, setPage] = useState(1);
  const [data, setData] = useState<RoutesResponse | null>(null);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const requestSeq = useRef(0);

  // Debounce typing so a search does not hit the API on every keypress.
  useEffect(() => {
    const timer = setTimeout(() => {
      setDebouncedQuery(query.trim());
      setPage(1);
    }, SEARCH_DEBOUNCE_MS);
    return () => clearTimeout(timer);
  }, [query]);

  useEffect(() => {
    setPage(1);
  }, [peerFilter, instanceFilter]);

  const loadPage = useCallback(
    async (targetPage: number) => {
      const requestId = ++requestSeq.current;
      const params = new URLSearchParams({
        page: String(targetPage),
        limit: String(PAGE_SIZE),
      });
      if (debouncedQuery) params.set("q", debouncedQuery);
      if (peerFilter) params.set("peer", peerFilter);
      if (instanceFilter) params.set("router", instanceFilter);

      try {
        const next = await fetchJson<RoutesResponse>(`/api/routes?${params.toString()}`);
        if (requestId !== requestSeq.current) return;
        setData(next);
        setPage(next.page);
        setError(null);
      } catch (err) {
        if (requestId !== requestSeq.current) return;
        setError(err instanceof Error ? err.message : String(err));
      } finally {
        if (requestId === requestSeq.current) setLoading(false);
      }
    },
    [debouncedQuery, peerFilter, instanceFilter],
  );

  useEffect(() => {
    void loadPage(page);
  }, [loadPage, page]);

  // Keep the current page fresh; the server does the sorting and slicing.
  useEffect(() => {
    const timer = setInterval(() => {
      if (!document.hidden) void loadPage(page);
    }, 5_000);
    return () => clearInterval(timer);
  }, [loadPage, page]);

  const rows = data?.routes ?? [];
  const pageSize = data?.pageSize ?? PAGE_SIZE;
  const displayPage = data?.page ?? page;
  const totalPages = data?.totalPages ?? 1;
  const filteredTotal = data?.total ?? 0;
  const firstRow = filteredTotal > 0 ? (displayPage - 1) * pageSize + 1 : 0;
  const lastRow = filteredTotal > 0 ? firstRow + rows.length - 1 : 0;
  const hasFilter = Boolean(peerFilterLabel || instanceFilterLabel || debouncedQuery);

  return (
    <div className="route-panel" aria-busy={loading}>
      <div className="route-toolbar">
        <div className="route-summary">
          <span>
            活跃前缀 <b>{formatNumber(totalActive)}</b>
          </span>
          {hasFilter && (
            <span className="muted">筛选结果 {formatNumber(filteredTotal)} 条</span>
          )}
          {instanceFilterLabel && <span className="filter-chip">实例：{instanceFilterLabel}</span>}
          {peerFilterLabel && <span className="filter-chip">对等体：{peerFilterLabel}</span>}
          {(peerFilterLabel || instanceFilterLabel) && (
            <button type="button" className="text-button" onClick={onClearFilters}>
              清除筛选
            </button>
          )}
        </div>

        <label className="search">
          <SearchIcon size={15} />
          <input
            value={query}
            onChange={(event) => setQuery(event.target.value)}
            placeholder="搜索前缀 / AS Path / 社区 / 下一跳"
            aria-label="搜索路由"
          />
          {query && (
            <button type="button" className="clear-input" onClick={() => setQuery("")} aria-label="清空搜索">
              <CloseIcon size={12} />
            </button>
          )}
        </label>
      </div>

      {error && <div className="alert alert-error route-error">加载路由失败：{error}</div>}

      <div className="route-scroll">
        <table className="route-table">
          <thead>
            <tr>
              <th>前缀</th>
              <th>来源</th>
              <th>下一跳</th>
              <th>AS Path</th>
              <th>Origin</th>
              <th className="right">MED / LP</th>
              <th>Community</th>
              <th className="right">更新</th>
            </tr>
          </thead>
          <tbody>
            {rows.map((route) => (
              <tr key={`${route.peerKey}-${route.prefix}-${route.pathId ?? 0}`}>
                <td>
                  <span className="prefix mono">{route.prefix}</span>
                  {route.pathId !== null && <span className="path-id">#{route.pathId}</span>}
                </td>
                <td>
                  <span className="mono">{route.peerIp}</span>
                  <span className="peer-asn">AS{route.peerAsn}</span>
                </td>
                <td className="mono">{route.nextHop ?? "—"}</td>
                <td>
                  <span className="path-chips">
                    {route.asPath.slice(0, 5).map((asn, index) => (
                      <span className="asn-chip" key={`${asn}-${index}`}>
                        {asn}
                      </span>
                    ))}
                    {route.asPath.length > 5 && (
                      <span className="asn-chip more" title={route.asPathStr}>
                        +{route.asPath.length - 5}
                      </span>
                    )}
                    {route.asPath.length === 0 && <span className="muted">—</span>}
                  </span>
                </td>
                <td>
                  {route.origin ? (
                    <span className={`origin-badge origin-${route.origin}`}>{route.origin}</span>
                  ) : (
                    "—"
                  )}
                </td>
                <td className="right mono">
                  {route.med ?? "—"} / {route.localPref ?? "—"}
                </td>
                <td>
                  <span className="community-list">
                    {route.communities.slice(0, 2).map((community) => (
                      <span className="community" key={community}>
                        {community}
                      </span>
                    ))}
                    {route.communities.length > 2 && (
                      <span className="community more" title={route.communities.join(" ")}>
                        +{route.communities.length - 2}
                      </span>
                    )}
                    {route.communities.length === 0 && <span className="muted">—</span>}
                  </span>
                </td>
                <td className="right">
                  <span title={new Date(route.updatedAt * 1000).toLocaleString("zh-CN")}>
                    {timeAgo(route.updatedAt)}
                  </span>
                  <span className="updates">{route.updates}×</span>
                </td>
              </tr>
            ))}
            {rows.length === 0 && (
              <tr>
                <td colSpan={8}>
                  <div className="panel-empty">
                    {loading
                      ? "正在加载路由…"
                      : filteredTotal === 0
                        ? hasFilter
                          ? "没有匹配当前筛选条件的前缀。"
                          : "当前还没有重建出任何前缀。收到 Route Monitoring 通告后会显示在这里。"
                        : "当前页没有数据。"}
                  </div>
                </td>
              </tr>
            )}
          </tbody>
        </table>
      </div>

      <div className="route-footer">
        <span>
          {filteredTotal > 0
            ? `显示 ${formatNumber(firstRow)}–${formatNumber(lastRow)} / ${formatNumber(filteredTotal)}`
            : "0 条"}
          {" · 每页 "}
          {pageSize}
          {" 条 · 服务端分页"}
          {loading && " · 刷新中…"}
        </span>
        <div className="pagination">
          <button
            type="button"
            className="icon-button"
            disabled={displayPage <= 1}
            onClick={() => setPage(displayPage - 1)}
            aria-label="上一页"
          >
            <ChevronLeftIcon size={16} />
          </button>
          <span>
            {displayPage} / {totalPages}
          </span>
          <button
            type="button"
            className="icon-button"
            disabled={displayPage >= totalPages}
            onClick={() => setPage(displayPage + 1)}
            aria-label="下一页"
          >
            <ChevronRightIcon size={16} />
          </button>
        </div>
      </div>
    </div>
  );
}
