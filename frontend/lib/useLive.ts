"use client";

import { useCallback, useEffect, useRef, useState } from "react";
import { fetchJson, wsUrl } from "./api";
import type { MessageView, Overview, ServerEvent } from "./types";

const MAX_FEED = 400;

function mergeMessages(incoming: MessageView[], existing: MessageView[]): MessageView[] {
  const seen = new Set<number>();
  const merged: MessageView[] = [];
  for (const message of [...incoming, ...existing]) {
    if (seen.has(message.id)) continue;
    seen.add(message.id);
    merged.push(message);
    if (merged.length >= MAX_FEED) break;
  }
  return merged;
}

function isNewerOverview(next: Overview, previous: Overview | null): boolean {
  if (!previous) return true;
  // A server restart resets overviewSeq but produces a newer startedAt.
  if (next.startedAt !== previous.startedAt) {
    return next.startedAt > previous.startedAt;
  }
  return next.overviewSeq >= previous.overviewSeq;
}

export type LiveStatus = "connecting" | "live" | "offline";

export type RoutePulseKind = "up" | "down";

export interface RoutePulse {
  kind: RoutePulseKind;
  seq: number;
}

export function useLive() {
  const [overview, setOverview] = useState<Overview | null>(null);
  const [messages, setMessages] = useState<MessageView[]>([]);
  const [routePulse, setRoutePulse] = useState<RoutePulse | null>(null);
  const [status, setStatus] = useState<LiveStatus>("connecting");
  const [paused, setPausedFlag] = useState(false);
  const [lastEventAt, setLastEventAt] = useState<number | null>(null);
  const [error, setError] = useState<string | null>(null);
  const pausedRef = useRef(false);
  const pendingRef = useRef<MessageView[]>([]);
  const flushTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  const routePulseSeqRef = useRef(0);

  const flushPending = useCallback(() => {
    flushTimerRef.current = null;
    if (pendingRef.current.length === 0) return;
    const pending = pendingRef.current;
    pendingRef.current = [];
    setMessages((previous) => mergeMessages(pending, previous));
  }, []);

  const queueMessage = useCallback(
    (message: MessageView) => {
      if (pausedRef.current) return;
      // Batch high-rate feeds into ~8 UI updates per second.
      pendingRef.current.unshift(message);
      if (pendingRef.current.length > MAX_FEED) pendingRef.current.length = MAX_FEED;
      if (flushTimerRef.current === null) {
        flushTimerRef.current = setTimeout(flushPending, 120);
      }
    },
    [flushPending],
  );

  useEffect(() => {
    return () => {
      if (flushTimerRef.current) clearTimeout(flushTimerRef.current);
    };
  }, []);

  const applyOverview = useCallback((next: Overview) => {
    setOverview((previous) => (isNewerOverview(next, previous) ? next : previous));
  }, []);

  const loadSnapshot = useCallback(async () => {
    try {
      const [nextOverview, nextMessages] = await Promise.all([
        fetchJson<Overview>("/api/overview"),
        fetchJson<MessageView[]>("/api/messages?limit=200"),
      ]);
      applyOverview(nextOverview);
      if (!pausedRef.current) {
        setMessages((previous) => mergeMessages(nextMessages, previous));
      }
      setError(null);
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err));
    }
  }, [applyOverview]);

  useEffect(() => {
    void loadSnapshot();
  }, [loadSnapshot]);

  // Live WebSocket feed with automatic reconnect and gap recovery.
  useEffect(() => {
    let socket: WebSocket | null = null;
    let closed = false;
    let attempts = 0;
    let retryTimer: ReturnType<typeof setTimeout> | undefined;

    const connect = () => {
      if (closed) return;
      setStatus("connecting");
      socket = new WebSocket(wsUrl());
      socket.onopen = () => {
        attempts = 0;
        setStatus("live");
        void loadSnapshot();
      };
      socket.onmessage = (event) => {
        let parsed: ServerEvent;
        try {
          parsed = JSON.parse(String(event.data)) as ServerEvent;
        } catch {
          return;
        }
        if (parsed.type === "overview") {
          applyOverview(parsed.data);
        } else if (parsed.type === "message") {
          setLastEventAt(Date.now());
          const delta = parsed.data.announced - parsed.data.withdrawn;
          if (delta !== 0) {
            routePulseSeqRef.current += 1;
            setRoutePulse({
              kind: delta > 0 ? "up" : "down",
              seq: routePulseSeqRef.current,
            });
          }
          queueMessage(parsed.data);
        } else if (parsed.type === "lagged") {
          void loadSnapshot();
        }
      };
      socket.onerror = () => socket?.close();
      socket.onclose = () => {
        if (closed) return;
        setStatus("offline");
        const delay = Math.min(10_000, 1_000 * 2 ** Math.min(attempts, 4));
        attempts += 1;
        retryTimer = setTimeout(connect, delay);
      };
    };

    connect();
    return () => {
      closed = true;
      if (retryTimer) clearTimeout(retryTimer);
      socket?.close();
    };
  }, [applyOverview, loadSnapshot, queueMessage]);

  // Fallback polling when the WebSocket is unavailable.
  useEffect(() => {
    if (status === "live") return;
    let stopped = false;
    const poll = async () => {
      if (document.hidden) return;
      try {
        const [nextOverview, nextMessages] = await Promise.all([
          fetchJson<Overview>("/api/overview"),
          fetchJson<MessageView[]>("/api/messages?limit=200"),
        ]);
        if (stopped) return;
        applyOverview(nextOverview);
        if (!pausedRef.current) {
          setMessages((previous) => mergeMessages(nextMessages, previous));
        }
      } catch (err) {
        setError(err instanceof Error ? err.message : String(err));
      }
    };
    void poll();
    const timer = setInterval(poll, 5_000);
    return () => {
      stopped = true;
      clearInterval(timer);
    };
  }, [applyOverview, status]);

  const setPaused = useCallback(
    (value: boolean) => {
      pausedRef.current = value;
      setPausedFlag(value);
      if (value) {
        pendingRef.current = [];
        if (flushTimerRef.current) {
          clearTimeout(flushTimerRef.current);
          flushTimerRef.current = null;
        }
      } else {
        void loadSnapshot();
      }
    },
    [loadSnapshot],
  );

  return {
    overview,
    messages,
    routePulse,
    status,
    paused,
    setPaused,
    lastEventAt,
    error,
    refresh: loadSnapshot,
  };
}
