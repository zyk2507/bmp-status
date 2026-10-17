"use client";

import { useState } from "react";
import { ApiError } from "@/lib/api";
import { ShieldIcon } from "./icons";

function errorMessage(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}

export function LoginScreen({ onLogin }: { onLogin: (password: string) => Promise<void> }) {
  const [password, setPassword] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const submit = async (event: React.FormEvent) => {
    event.preventDefault();
    setBusy(true);
    setError(null);
    try {
      await onLogin(password);
    } catch (err) {
      setError(err instanceof ApiError ? err.message : errorMessage(err));
    } finally {
      setBusy(false);
    }
  };

  return (
    <div className="login-wrap">
      <div className="login-card">
        <div className="login-brand">
          <div className="brand-mark">
            <ShieldIcon size={21} />
          </div>
          <div>
            <h1>BMP Status</h1>
            <p>Protected Console</p>
          </div>
        </div>
        <h2>输入页面密码</h2>
        <p>
          仪表盘、管理员面板和所有业务 API 使用同一个密码。登录后会写入 HttpOnly
          会话 Cookie，关闭页面再打开会自动登录，直到会话过期。
        </p>
        <form className="form-stack" onSubmit={submit}>
          <label className="field">
            <span>页面密码</span>
            <input
              className="input"
              type="password"
              autoFocus
              autoComplete="current-password"
              value={password}
              onChange={(event) => setPassword(event.target.value)}
              placeholder="请输入页面密码"
            />
          </label>
          {error && (
            <div className="alert alert-error" style={{ margin: 0 }}>
              {error}
            </div>
          )}
          <button className="btn btn-primary" type="submit" disabled={busy || !password}>
            {busy ? "登录中…" : "登录"}
          </button>
        </form>
        <div className="login-hint">
          首次启动时，随机生成的密码保存在 <code>data/initial-admin-password.txt</code>；
          也可以通过 <code>ADMIN_PASSWORD</code> 环境变量设置。
        </div>
        <div className="login-footer">
          <span>HttpOnly Session · SameSite=Lax · Argon2id</span>
        </div>
      </div>
    </div>
  );
}
