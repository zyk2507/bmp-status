"use client";

import Link from "next/link";
import { useCallback, useEffect, useRef, useState } from "react";
import { useAuth } from "@/lib/useAuth";
import {
  ApiError,
  deleteJson,
  fetchJson,
  postJson,
  putJson,
} from "@/lib/api";
import {
  formatBytes,
  formatDateTime,
  formatNumber,
  formatUptime,
  timeAgo,
} from "@/lib/format";
import type {
  AdminInstancesResponse,
  ConnectionView,
  ConnectionsResponse,
  InstanceInput,
  InstanceStatus,
  Overview,
  RuntimeConfig,
} from "@/lib/types";
import {
  CheckIcon,
  ChevronLeftIcon,
  LogoutIcon,
  PencilIcon,
  PlusIcon,
  RefreshIcon,
  ShieldIcon,
  TrashIcon,
} from "@/components/icons";

const EMPTY_FORM: InstanceInput = {
  name: "",
  routerIp: "",
  description: "",
  enabled: true,
};

function errorMessage(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}

function AdminPanelHead({
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

function instanceForConnection(
  connection: ConnectionView,
  instances: InstanceStatus[],
): InstanceStatus | undefined {
  return instances.find((instance) => instance.routerIp === connection.routerIp);
}

function ConnectionRows({
  connections,
  instances,
}: {
  connections: ConnectionView[];
  instances: InstanceStatus[];
}) {
  if (connections.length === 0) {
    return (
      <tr>
        <td colSpan={7}>
          <div className="panel-empty">暂无连接记录。</div>
        </td>
      </tr>
    );
  }
  return (
    <>
      {connections.map((connection) => {
        const instance = instanceForConnection(connection, instances);
        const state =
          connection.state === "active"
            ? "online"
            : connection.state === "rejected"
              ? "rejected"
              : "offline";
        return (
          <tr key={`${connection.id}-${connection.state}-${connection.since}`}>
            <td>
              <b className="mono">{connection.routerIp}</b>
              <div className="muted small mono">{connection.peer}</div>
            </td>
            <td>
              {instance ? (
                instance.name
              ) : (
                <span className="badge subtle">未登记</span>
              )}
            </td>
            <td className="mono">{connection.routerName ?? "—"}</td>
            <td>
              <span className={`state-badge ${state}`}>
                {connection.state === "active"
                  ? "在线"
                  : connection.state === "rejected"
                    ? "已拒绝"
                    : "已关闭"}
              </span>
            </td>
            <td>
              {formatNumber(connection.messages)} 条
              <div className="muted small">{formatBytes(connection.bytes)}</div>
            </td>
            <td title={formatDateTime(connection.lastMessageAt)}>
              {timeAgo(connection.lastMessageAt)}
            </td>
            <td className="muted">{connection.closeReason ?? "—"}</td>
          </tr>
        );
      })}
    </>
  );
}

function AdminAuthLoading({ text }: { text: string }) {
  return (
    <div className="login-wrap">
      <div className="login-card">
        <div className="spinner" style={{ margin: "0 auto 14px" }} />
        <p style={{ textAlign: "center", marginBottom: 0 }}>{text}</p>
      </div>
    </div>
  );
}

function AdminView({ onLogout }: { onLogout: () => Promise<void> }) {
  const [adminData, setAdminData] = useState<AdminInstancesResponse | null>(null);
  const [connections, setConnections] = useState<ConnectionsResponse | null>(null);
  const [overview, setOverview] = useState<Overview | null>(null);

  const [form, setForm] = useState<InstanceInput>({ ...EMPTY_FORM });
  const [editingId, setEditingId] = useState<string | null>(null);
  const [formBusy, setFormBusy] = useState(false);
  const [settingsBusy, setSettingsBusy] = useState(false);
  const [runtimeForm, setRuntimeForm] = useState<RuntimeConfig>({
    maxMessages: 500,
    maxRoutes: 0,
    maxPeers: 0,
  });
  const [runtimeBusy, setRuntimeBusy] = useState(false);
  const runtimeInitialized = useRef(false);

  const [currentPassword, setCurrentPassword] = useState("");
  const [newPassword, setNewPassword] = useState("");
  const [confirmPassword, setConfirmPassword] = useState("");
  const [passwordBusy, setPasswordBusy] = useState(false);

  const [toast, setToast] = useState<{ kind: "error" | "success"; text: string } | null>(null);

  const notify = useCallback((kind: "error" | "success", text: string) => {
    setToast({ kind, text });
  }, []);

  useEffect(() => {
    if (!toast) return;
    const timer = setTimeout(() => setToast(null), 4200);
    return () => clearTimeout(timer);
  }, [toast]);

  const handleUnauthorized = useCallback(
    (error: unknown): boolean => error instanceof ApiError && error.status === 401,
    [],
  );

  const loadAdmin = useCallback(
    async (silent = false) => {
      try {
        const [instanceData, connectionData, overviewData] = await Promise.all([
          fetchJson<AdminInstancesResponse>("/api/admin/instances"),
          fetchJson<ConnectionsResponse>("/api/admin/connections"),
          fetchJson<Overview>("/api/overview"),
        ]);
        setAdminData(instanceData);
        setConnections(connectionData);
        setOverview(overviewData);
      } catch (error) {
        if (handleUnauthorized(error)) return;
        if (!silent) notify("error", errorMessage(error));
      }
    },
    [handleUnauthorized, notify],
  );

  useEffect(() => {
    void loadAdmin();
    const timer = setInterval(() => {
      if (!document.hidden) void loadAdmin(true);
    }, 5000);
    return () => clearInterval(timer);
  }, [loadAdmin]);

  useEffect(() => {
    if (adminData && !runtimeInitialized.current) {
      setRuntimeForm(adminData.runtime);
      runtimeInitialized.current = true;
    }
  }, [adminData]);

  const handleLogout = async () => {
    await onLogout();
  };

  const resetForm = () => {
    setForm({ ...EMPTY_FORM });
    setEditingId(null);
  };

  const handleInstanceSubmit = async (event: React.FormEvent) => {
    event.preventDefault();
    if (!form.name.trim() || !form.routerIp.trim()) {
      notify("error", "请填写实例名称和 BMP 源 IP");
      return;
    }
    setFormBusy(true);
    try {
      if (editingId) {
        await putJson(`/api/admin/instances/${editingId}`, form);
        notify("success", "实例已更新");
      } else {
        await postJson("/api/admin/instances", form);
        notify("success", "实例已添加");
      }
      resetForm();
      await loadAdmin(true);
    } catch (error) {
      if (!handleUnauthorized(error)) notify("error", errorMessage(error));
    } finally {
      setFormBusy(false);
    }
  };

  const startEdit = (instance: InstanceStatus) => {
    setEditingId(instance.id);
    setForm({
      name: instance.name,
      routerIp: instance.routerIp,
      description: instance.description,
      enabled: instance.enabled,
    });
    document.getElementById("instance-form")?.scrollIntoView({ behavior: "smooth", block: "center" });
  };

  const toggleInstance = async (instance: InstanceStatus) => {
    try {
      await putJson(`/api/admin/instances/${instance.id}`, {
        name: instance.name,
        routerIp: instance.routerIp,
        description: instance.description,
        enabled: !instance.enabled,
      });
      await loadAdmin(true);
      notify("success", `${instance.name} 已${instance.enabled ? "禁用" : "启用"}`);
    } catch (error) {
      if (!handleUnauthorized(error)) notify("error", errorMessage(error));
    }
  };

  const deleteInstance = async (instance: InstanceStatus) => {
    if (!window.confirm(`确定删除实例“${instance.name}”（${instance.routerIp}）？`)) return;
    try {
      await deleteJson(`/api/admin/instances/${instance.id}`);
      if (editingId === instance.id) resetForm();
      await loadAdmin(true);
      notify("success", "实例已删除");
    } catch (error) {
      if (!handleUnauthorized(error)) notify("error", errorMessage(error));
    }
  };

  const toggleAllowlist = async () => {
    if (!adminData) return;
    setSettingsBusy(true);
    try {
      await putJson("/api/admin/settings", {
        enforceAllowlist: !adminData.enforceAllowlist,
      });
      await loadAdmin(true);
      notify(
        "success",
        adminData.enforceAllowlist ? "来源白名单已关闭" : "来源白名单已启用",
      );
    } catch (error) {
      if (!handleUnauthorized(error)) notify("error", errorMessage(error));
    } finally {
      setSettingsBusy(false);
    }
  };

  const handleRuntimeSubmit = async (event: React.FormEvent) => {
    event.preventDefault();
    setRuntimeBusy(true);
    try {
      const result = await putJson<{ runtime: RuntimeConfig }>("/api/admin/runtime", runtimeForm);
      setRuntimeForm(result.runtime);
      await loadAdmin(true);
      notify("success", "运行数据上限已更新");
    } catch (error) {
      if (!handleUnauthorized(error)) notify("error", errorMessage(error));
    } finally {
      setRuntimeBusy(false);
    }
  };

  const handlePasswordChange = async (event: React.FormEvent) => {
    event.preventDefault();
    if (newPassword.length < 8) {
      notify("error", "新密码至少需要 8 个字符");
      return;
    }
    if (newPassword !== confirmPassword) {
      notify("error", "两次输入的新密码不一致");
      return;
    }
    setPasswordBusy(true);
    try {
      await postJson("/api/admin/password", {
        currentPassword,
        newPassword,
      });
      setCurrentPassword("");
      setNewPassword("");
      setConfirmPassword("");
      notify("success", "页面密码已更新，其他会话已失效");
    } catch (error) {
      if (!handleUnauthorized(error)) notify("error", errorMessage(error));
    } finally {
      setPasswordBusy(false);
    }
  };

  if (!adminData || !connections) {
    return (
      <div className="admin-shell">
        <div className="panel loading-panel">
          <div className="spinner" />
          <h2>正在加载管理员数据…</h2>
        </div>
      </div>
    );
  }

  const instances = adminData.instances;
  const recentConnections = connections.recent.slice(0, 20);

  return (
    <main className="admin-shell">
      <header className="admin-header">
        <div className="admin-header-left">
          <div className="brand-mark">
            <ShieldIcon size={22} />
          </div>
          <div>
            <h1 style={{ margin: 0, fontSize: 20, display: "flex", alignItems: "center", gap: 9 }}>
              管理员控制台 <span className="admin-badge">ADMIN</span>
            </h1>
            <p style={{ margin: "4px 0 0", fontSize: 10.5, color: "var(--muted)", letterSpacing: "0.08em" }}>
              实例登记 · 来源白名单 · 运行上限 · 安全设置
            </p>
          </div>
        </div>
        <div className="admin-header-right">
          <Link className="btn btn-ghost btn-sm" href="/">
            <ChevronLeftIcon size={14} /> 返回仪表盘
          </Link>
          <button className="btn btn-sm" type="button" onClick={() => void loadAdmin()} title="刷新管理员数据">
            <RefreshIcon size={14} /> 刷新
          </button>
          <button className="btn btn-sm" type="button" onClick={handleLogout}>
            <LogoutIcon size={14} /> 退出登录
          </button>
        </div>
      </header>

      <div className="admin-grid">
        <div className="admin-stack">
          <section className="admin-panel">
            <AdminPanelHead
              title="BMP 实例"
              subtitle="登记 BIRD / FRR 实例的 BMP TCP 源地址，用于标识和访问控制"
              right={<span className="panel-badge">{instances.length} 个实例</span>}
            />
            <div className="panel-body">
              <form id="instance-form" onSubmit={handleInstanceSubmit}>
                <div className="form-grid">
                  <label className="field">
                    <span>实例名称 *</span>
                    <input
                      className="input"
                      value={form.name}
                      onChange={(event) => setForm({ ...form, name: event.target.value })}
                      placeholder="例如：edge-rtr-1 / 上海 BIRD"
                      maxLength={64}
                    />
                  </label>
                  <label className="field">
                    <span>BMP 源 IP *</span>
                    <input
                      className="input mono"
                      value={form.routerIp}
                      onChange={(event) => setForm({ ...form, routerIp: event.target.value })}
                      placeholder="10.0.0.1"
                    />
                    <small>BMP TCP 连接使用的源地址；若经过 NAT，请填写 NAT 后的地址。</small>
                  </label>
                  <label className="field full">
                    <span>描述</span>
                    <input
                      className="input"
                      value={form.description}
                      onChange={(event) => setForm({ ...form, description: event.target.value })}
                      placeholder="位置、角色或其他备注（可选）"
                      maxLength={280}
                    />
                  </label>
                  <label className="checkbox-row full">
                    <input
                      type="checkbox"
                      checked={form.enabled}
                      onChange={(event) => setForm({ ...form, enabled: event.target.checked })}
                    />
                    启用该实例（白名单只放行已启用的实例）
                  </label>
                </div>
                <div className="form-actions">
                  <button className="btn btn-primary btn-sm" type="submit" disabled={formBusy}>
                    {editingId ? <CheckIcon size={13} /> : <PlusIcon size={13} />}
                    {formBusy ? "保存中…" : editingId ? "保存修改" : "添加实例"}
                  </button>
                  {editingId && (
                    <button className="btn btn-ghost btn-sm" type="button" onClick={resetForm}>
                      取消编辑
                    </button>
                  )}
                  <span className="muted small">
                    多个 BIRD 实例可以分别登记；仪表盘会按源 IP 关联实例名称。
                  </span>
                </div>
              </form>

              <div className="divider" />

              <div className="table-wrap">
                <table className="admin-table">
                  <thead>
                    <tr>
                      <th>实例</th>
                      <th>状态</th>
                      <th>对等体</th>
                      <th>消息 / 流量</th>
                      <th>最近上报</th>
                      <th className="right">操作</th>
                    </tr>
                  </thead>
                  <tbody>
                    {instances.map((instance) => (
                      <tr key={instance.id}>
                        <td>
                          <div className="table-name">
                            <span
                              className={`status-dot ${
                                instance.online ? "up" : instance.enabled ? "down" : "idle"
                              }`}
                            />
                            <div>
                              <b>{instance.name}</b>
                              <span>{instance.routerIp}</span>
                              {instance.description && (
                                <span className="muted small" style={{ fontFamily: "inherit", marginTop: 2 }}>
                                  {instance.description}
                                </span>
                              )}
                            </div>
                          </div>
                        </td>
                        <td>
                          <span className={`state-badge ${instance.online ? "online" : "offline"}`}>
                            {instance.online
                              ? "在线"
                              : instance.enabled
                                ? instance.lastSeen
                                  ? "等待重连"
                                  : "等待连接"
                                : "已禁用"}
                          </span>
                        </td>
                        <td>{formatNumber(instance.peers)}</td>
                        <td>
                          {formatNumber(instance.messages)}
                          <div className="muted small">{formatBytes(instance.bytes)}</div>
                        </td>
                        <td title={instance.lastSeen ? formatDateTime(instance.lastSeen) : undefined}>
                          {instance.lastSeen ? timeAgo(instance.lastSeen) : "—"}
                        </td>
                        <td>
                          <div className="table-actions">
                            <button
                              className={`switch ${instance.enabled ? "on" : ""}`}
                              type="button"
                              onClick={() => void toggleInstance(instance)}
                              title={instance.enabled ? "禁用实例" : "启用实例"}
                              aria-label={instance.enabled ? "禁用实例" : "启用实例"}
                              aria-checked={instance.enabled}
                              role="switch"
                            />
                            <button
                              className="icon-button"
                              type="button"
                              onClick={() => startEdit(instance)}
                              title="编辑实例"
                              aria-label="编辑实例"
                            >
                              <PencilIcon size={13} />
                            </button>
                            <button
                              className="icon-button"
                              type="button"
                              onClick={() => void deleteInstance(instance)}
                              title="删除实例"
                              aria-label="删除实例"
                            >
                              <TrashIcon size={13} />
                            </button>
                          </div>
                        </td>
                      </tr>
                    ))}
                    {instances.length === 0 && (
                      <tr>
                        <td colSpan={6}>
                          <div className="panel-empty">
                            尚未登记任何实例。添加后可在仪表盘按实例筛选，并可启用来源白名单。
                          </div>
                        </td>
                      </tr>
                    )}
                  </tbody>
                </table>
              </div>
            </div>
          </section>

          <section className="admin-panel">
            <AdminPanelHead
              title="BMP 连接"
              subtitle="当前活动连接与最近关闭 / 拒绝记录"
              right={
                <span className="panel-badge">
                  {connections.active.length} 活动
                </span>
              }
            />
            <div className="panel-body">
              <div className="table-wrap">
                <table className="admin-table">
                  <thead>
                    <tr>
                      <th>来源</th>
                      <th>实例</th>
                      <th>路由器名</th>
                      <th>状态</th>
                      <th>消息 / 流量</th>
                      <th>最近消息</th>
                      <th>关闭原因</th>
                    </tr>
                  </thead>
                  <tbody>
                    <ConnectionRows
                      connections={[...connections.active, ...recentConnections]}
                      instances={instances}
                    />
                  </tbody>
                </table>
              </div>
              <p className="muted small" style={{ margin: "9px 2px 0" }}>
                白名单只影响新连接；关闭白名单不会主动断开已有连接。
              </p>
            </div>
          </section>
        </div>

        <div className="admin-stack">
          <section className="admin-panel">
            <AdminPanelHead title="访问控制" subtitle="限制哪些 BMP 源可以接入" />
            <div className="panel-body">
              <div className="switch-row">
                <div className="switch-copy">
                  <h3>仅允许已登记实例接入</h3>
                  <p>
                    启用后，源 IP 不在“已启用”实例列表中的 BMP TCP 连接会被立即拒绝。
                    回环地址（127.0.0.1 / ::1）始终允许，便于本地演示与本地路由进程。
                  </p>
                </div>
                <button
                  className={`switch ${adminData.enforceAllowlist ? "on" : ""}`}
                  type="button"
                  role="switch"
                  aria-checked={adminData.enforceAllowlist}
                  aria-label="切换来源白名单"
                  disabled={settingsBusy}
                  onClick={() => void toggleAllowlist()}
                />
              </div>
              <div className="setting-row">
                <span>当前状态</span>
                <b>{adminData.enforceAllowlist ? "白名单已启用" : "接受所有来源"}</b>
              </div>
              <div className="setting-row">
                <span>已登记实例</span>
                <b>{instances.length} 个（{instances.filter((item) => item.enabled).length} 启用）</b>
              </div>
            </div>
          </section>

          <section className="admin-panel">
            <AdminPanelHead
              title="运行数据上限"
              subtitle="0 表示不限制；配置保存在 data/runtime.sqlite3"
            />
            <div className="panel-body">
              <form className="form-grid" onSubmit={handleRuntimeSubmit}>
                <label className="field">
                  <span>消息缓存条数</span>
                  <input
                    className="input mono"
                    type="number"
                    min={0}
                    step={1}
                    value={runtimeForm.maxMessages}
                    onChange={(event) =>
                      setRuntimeForm({
                        ...runtimeForm,
                        maxMessages: Math.max(0, Number(event.target.value) || 0),
                      })
                    }
                  />
                  <small>只影响内存中的实时消息流，默认 500。</small>
                </label>
                <label className="field">
                  <span>活跃路由上限</span>
                  <input
                    className="input mono"
                    type="number"
                    min={0}
                    step={1}
                    value={runtimeForm.maxRoutes}
                    onChange={(event) =>
                      setRuntimeForm({
                        ...runtimeForm,
                        maxRoutes: Math.max(0, Number(event.target.value) || 0),
                      })
                    }
                  />
                  <small>0 = 不限制；超过上限时按最早进入 RIB 的顺序淘汰。</small>
                </label>
                <label className="field">
                  <span>Peer 状态上限</span>
                  <input
                    className="input mono"
                    type="number"
                    min={0}
                    step={1}
                    value={runtimeForm.maxPeers}
                    onChange={(event) =>
                      setRuntimeForm({
                        ...runtimeForm,
                        maxPeers: Math.max(0, Number(event.target.value) || 0),
                      })
                    }
                  />
                  <small>0 = 不限制；淘汰 peer 时会同时清理其路由。</small>
                </label>
                <div className="form-actions full">
                  <button className="btn btn-primary btn-sm" type="submit" disabled={runtimeBusy}>
                    {runtimeBusy ? "保存中…" : "保存上限"}
                  </button>
                  <span className="muted small">
                    修改会立即裁剪当前内存数据；密码和实例登记仍保存在 admin.json。
                  </span>
                </div>
              </form>
            </div>
          </section>

          <section className="admin-panel">
            <AdminPanelHead title="运行信息" subtitle="服务端配置与实时计数" />
            <div className="panel-body">
              {overview ? (
                <>
                  <div className="setting-row">
                    <span>BMP 监听</span>
                    <b>tcp://{overview.bmpBind}</b>
                  </div>
                  <div className="setting-row">
                    <span>HTTP / API</span>
                    <b>http://{overview.httpBind}</b>
                  </div>
                  <div className="setting-row">
                    <span>运行时间</span>
                    <b>{formatUptime(overview.uptimeSeconds)}</b>
                  </div>
                  <div className="setting-row">
                    <span>活跃 BMP 连接</span>
                    <b>
                      {overview.activeConnections} / {overview.maxConnections}（累计 {formatNumber(overview.connectionsTotal)}）
                    </b>
                  </div>
                  <div className="setting-row">
                    <span>对等体 / 前缀</span>
                    <b>
                      {overview.peersUp} / {overview.peersTotal} · {formatNumber(overview.activeRoutes)} 条
                    </b>
                  </div>
                  <div className="setting-row">
                    <span>最大消息 / 空闲超时</span>
                    <b>
                      {formatBytes(overview.maxMsgSize)} / {overview.idleTimeoutSecs}s
                    </b>
                  </div>
                  <div className="setting-row">
                    <span>解析错误 / 拒绝连接</span>
                    <b>
                      {formatNumber(overview.totals.parseErrors)} /{" "}
                      {formatNumber(overview.totals.rejectedConnections)}
                    </b>
                  </div>
                </>
              ) : (
                <div className="panel-empty">加载中…</div>
              )}
            </div>
          </section>

          <section className="admin-panel">
            <AdminPanelHead title="修改页面密码" subtitle="Argon2id 哈希，会话 Cookie 有效期 12 小时" />
            <div className="panel-body">
              <form className="form-stack" onSubmit={handlePasswordChange}>
                <label className="field">
                  <span>当前页面密码</span>
                  <input
                    className="input"
                    type="password"
                    autoComplete="current-password"
                    value={currentPassword}
                    onChange={(event) => setCurrentPassword(event.target.value)}
                  />
                </label>
                <label className="field">
                  <span>新页面密码</span>
                  <input
                    className="input"
                    type="password"
                    autoComplete="new-password"
                    value={newPassword}
                    onChange={(event) => setNewPassword(event.target.value)}
                  />
                  <small>至少 8 个字符。修改成功后其他登录会话会立即失效。</small>
                </label>
                <label className="field">
                  <span>确认新页面密码</span>
                  <input
                    className="input"
                    type="password"
                    autoComplete="new-password"
                    value={confirmPassword}
                    onChange={(event) => setConfirmPassword(event.target.value)}
                  />
                </label>
                <button
                  className="btn btn-primary btn-sm"
                  type="submit"
                  disabled={passwordBusy || !currentPassword || !newPassword}
                >
                  <ShieldIcon size={13} /> {passwordBusy ? "更新中…" : "更新密码"}
                </button>
              </form>
            </div>
          </section>

          <section className="admin-panel">
            <AdminPanelHead title="快速入口" subtitle="页面与接口均由同一密码保护" />
            <div className="panel-body">
              <div className="setting-row">
                <span>仪表盘</span>
                <Link className="inline-link" href="/">
                  打开仪表盘
                </Link>
              </div>
              <div className="setting-row">
                <span>健康检查</span>
                <a className="inline-link" href="/api/health" target="_blank" rel="noreferrer">
                  /api/health
                </a>
              </div>
              <div className="setting-row">
                <span>实例 API</span>
                <a className="inline-link" href="/api/admin/instances" target="_blank" rel="noreferrer">
                  /api/admin/instances
                </a>
              </div>
            </div>
          </section>
        </div>
      </div>

      <footer className="admin-footer">
        <span>
          BMP Status Admin · 密码与会话仅保存在服务端 · 生产环境请置于 HTTPS 反向代理之后
        </span>
        <Link href="/">← 返回仪表盘</Link>
      </footer>

      {toast && <div className={`toast ${toast.kind}`}>{toast.text}</div>}
    </main>
  );
}


export default function AdminPage() {
  const auth = useAuth();

  useEffect(() => {
    if (auth.status === "unauthenticated") {
      window.location.replace("/");
    }
  }, [auth.status]);

  if (auth.status === "authenticated") {
    return <AdminView onLogout={auth.logout} />;
  }

  return (
    <AdminAuthLoading
      text={auth.status === "loading" ? "正在验证页面会话…" : "需要登录，正在返回仪表盘…"}
    />
  );
}
