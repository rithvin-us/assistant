/**
 * The only place React talks to the outside world.
 *
 * Network calls go through Tauri commands rather than `fetch`, so the request
 * path is identical on desktop and Android and the webview never needs CORS or
 * cleartext-HTTP exemptions. Adding a screen should mean adding a function here,
 * not a `fetch` inside a component.
 */

import { invoke } from "@tauri-apps/api/core";
import type { HealthResponse } from "./types";

export type ProbeResult =
  | { state: "reachable"; health: HealthResponse; latencyMs: number }
  | { state: "unreachable"; reason: string };

/**
 * Where the development server lives. Overridable at build time because an
 * Android device cannot reach the host's `localhost`.
 */
export const SERVER_BASE_URL: string =
  import.meta.env.VITE_SERVER_BASE_URL ?? "http://127.0.0.1:8787";

/**
 * Development bearer token. This is a placeholder credential for local work
 * only; real authentication replaces it, and no production secret ever belongs
 * in the frontend bundle.
 */
const DEV_TOKEN: string = import.meta.env.VITE_DEV_AUTH_TOKEN ?? "";

export function probeServer(): Promise<ProbeResult> {
  return invoke<ProbeResult>("probe_server", {
    baseUrl: SERVER_BASE_URL,
    token: DEV_TOKEN,
  });
}

export function localCacheReady(): Promise<boolean> {
  return invoke<boolean>("local_cache_ready");
}
