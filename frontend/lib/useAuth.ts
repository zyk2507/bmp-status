"use client";

import { useCallback, useEffect, useState } from "react";
import { fetchJson, postJson } from "./api";
import type { AdminSessionInfo } from "./types";

export type AuthStatus = "loading" | "authenticated" | "unauthenticated";

export function useAuth() {
  const [status, setStatus] = useState<AuthStatus>("loading");

  const check = useCallback(async () => {
    try {
      const info = await fetchJson<AdminSessionInfo>("/api/auth/session");
      setStatus(info.authenticated ? "authenticated" : "unauthenticated");
    } catch {
      setStatus("unauthenticated");
    }
  }, []);

  useEffect(() => {
    void check();
    const onUnauthorized = () => setStatus("unauthenticated");
    window.addEventListener("bmp:unauthorized", onUnauthorized);
    return () => window.removeEventListener("bmp:unauthorized", onUnauthorized);
  }, [check]);

  const login = useCallback(async (password: string) => {
    await postJson("/api/auth/login", { password });
    setStatus("authenticated");
  }, []);

  const logout = useCallback(async () => {
    try {
      await postJson("/api/auth/logout");
    } finally {
      setStatus("unauthenticated");
    }
  }, []);

  return { status, login, logout, refresh: check };
}
