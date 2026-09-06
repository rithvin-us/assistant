/**
 * The only place React talks to the outside world.
 *
 * Network calls go through Tauri commands rather than `fetch`, so the request
 * path is identical on desktop and Android and the webview never needs CORS or
 * cleartext-HTTP exemptions. Adding a screen should mean adding a function here,
 * not a `fetch` inside a component.
 */

import { invoke } from "@tauri-apps/api/core";
import { PROTOCOL_VERSION, type HealthResponse } from "./types";

export type ProbeResult =
  | { state: "reachable"; health: HealthResponse; latencyMs: number }
  | { state: "unreachable"; reason: string };

export const isTauri =
  typeof window !== "undefined" &&
  ("__TAURI_INTERNALS__" in window || "__TAURI_PATTERN__" in window || "__TAURI__" in window);

/**
 * Where the development server lives. Overridable at build time because an
 * Android device cannot reach the host's `localhost`.
 */
const rawServerUrl = import.meta.env.VITE_SERVER_BASE_URL ?? "http://127.0.0.1:8787";

export const SERVER_BASE_URL: string =
  typeof window !== "undefined" &&
  (window.location.hostname === "localhost" ||
    window.location.hostname === "127.0.0.1" ||
    window.location.hostname === "tauri.localhost" ||
    (isTauri && typeof window !== "undefined" && !/Android/i.test(navigator.userAgent)))
    ? "http://127.0.0.1:8787"
    : rawServerUrl;

/**
 * Development bearer token. This is a placeholder credential for local work
 * only; real authentication replaces it, and no production secret ever belongs
 * in the frontend bundle.
 */
export const DEV_TOKEN: string = import.meta.env.VITE_DEV_AUTH_TOKEN ?? "local-dev-token";



/** Narrows an unknown thrown value to something displayable. */
function reasonFrom(error: unknown, fallback: string): string {
  if (error instanceof Error) return error.message;
  if (typeof error === "string") return error;
  return fallback;
}

export async function probeServer(): Promise<ProbeResult> {
  if (isTauri) {
    try {
      return await invoke<ProbeResult>("probe_server", {
        baseUrl: SERVER_BASE_URL,
        token: DEV_TOKEN,
      });
    } catch (error: unknown) {
      return { state: "unreachable", reason: reasonFrom(error, "Tauri command failed") };
    }
  }

  const start = performance.now();
  try {
    const res = await fetch(`${SERVER_BASE_URL}/v1/health`, {
      headers: DEV_TOKEN ? { Authorization: `Bearer ${DEV_TOKEN}` } : {},
    });
    const latencyMs = Math.round(performance.now() - start);
    if (!res.ok) {
      return { state: "unreachable", reason: `HTTP status ${res.status}` };
    }
    const health: HealthResponse = await res.json();
    return { state: "reachable", health, latencyMs };
  } catch (error: unknown) {
    return { state: "unreachable", reason: reasonFrom(error, "Server unreachable") };
  }
}

export async function localCacheReady(): Promise<boolean> {
  if (isTauri) {
    try {
      return await invoke<boolean>("local_cache_ready");
    } catch {
      return false;
    }
  }
  // In a plain browser there is no Tauri shell and therefore no SQLite cache.
  // Reporting `true` here would make the UI claim an offline buffer that does
  // not exist; the sheet correctly shows "no local cache" instead.
  return false;
}

/**
 * Connection state, already reduced to what the UI shows.
 *
 * The reduction happens here rather than in a component so that Home can render
 * a single dot without knowing about protocol versions or degraded databases.
 */
export interface ConnectionState {
  kind: "checking" | "connected" | "offline";
  /** Only meaningful when connected: the server is up *and* fully configured. */
  healthy: boolean;
  /** One line, shown in the tooltip and in the sheet. */
  detail: string;
}

export async function loadConnection(): Promise<ConnectionState> {
  const [probe, cacheReady] = await Promise.all([probeServer(), localCacheReady()]);

  if (probe.state === "unreachable") {
    return { kind: "offline", healthy: false, detail: probe.reason };
  }

  const { health, latencyMs } = probe;

  // A mismatch means one side is an older build. Saying so beats letting a
  // field silently deserialise to undefined somewhere further in.
  if (health.protocol_version !== PROTOCOL_VERSION) {
    return {
      kind: "connected",
      healthy: false,
      detail: `Protocol mismatch: app v${PROTOCOL_VERSION}, server v${health.protocol_version}.`,
    };
  }

  const parts = [`v${health.version}`, `${latencyMs} ms`];
  if (health.status === "degraded") parts.push("no database");
  if (!cacheReady) parts.push("no local cache");

  return {
    kind: "connected",
    healthy: health.status === "ok" && cacheReady,
    detail: parts.join(" - "),
  };
}
