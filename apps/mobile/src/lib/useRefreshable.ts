/**
 * useRefreshable — one honest data-loading lifecycle for server-backed screens.
 *
 * Every list screen in this app needs the same six things, and before M11 each
 * one hand-rolled a subset of them:
 *
 *   - an initial load with a real loading state
 *   - background polling that does NOT flash a spinner over content
 *   - polling that pauses while the app is backgrounded
 *   - a manual refresh for pull-to-refresh
 *   - an error state that survives a failed refresh instead of looking empty
 *   - no overlapping duplicate requests
 *
 * The honesty rules are the point. A failed load keeps the last good data and
 * reports the failure; it never blanks the list, which would read as "you have
 * nothing" when the truth is "we could not reach the server". A refresh that is
 * already running swallows further requests rather than stacking them.
 *
 * `fetcher` must be the screen's real data path. Nothing here invents rows.
 */

import { useCallback, useEffect, useRef, useState } from "react";

export interface Refreshable<T> {
  data: T | null;
  /** True only for the first load, so a poll never covers content in a spinner. */
  loading: boolean;
  /** Non-null when the most recent attempt failed. Cleared by a success. */
  error: string | null;
  /** True while a manual refresh is in flight. */
  refreshing: boolean;
  /** The pull-to-refresh entry point. Resolves when the attempt settles. */
  refresh: () => Promise<void>;
  /** Lets a screen apply a confirmed server result without a round trip. */
  setData: React.Dispatch<React.SetStateAction<T | null>>;
}

export interface RefreshableOptions {
  /** Background poll interval in ms. Omit or pass 0 to disable polling. */
  pollMs?: number;
  /** Skips the initial load until true (e.g. an account must be picked first). */
  enabled?: boolean;
}

export function useRefreshable<T>(
  fetcher: () => Promise<T>,
  options: RefreshableOptions = {},
): Refreshable<T> {
  const { pollMs = 0, enabled = true } = options;

  const [data, setData] = useState<T | null>(null);
  const [loading, setLoading] = useState(enabled);
  const [error, setError] = useState<string | null>(null);
  const [refreshing, setRefreshing] = useState(false);

  /** Guards against overlapping requests from poll + pull + mount. */
  const inFlight = useRef(false);
  /** Stops a late response from writing state into an unmounted screen. */
  const alive = useRef(true);
  /** Keeps the effect from re-subscribing when the caller passes a new closure. */
  const fetcherRef = useRef(fetcher);

  // Synced in an effect rather than during render: writing a ref while
  // rendering is unsafe under concurrent rendering. This effect is declared
  // before the loading effect, so it has always run by the time the deferred
  // initial load fires.
  useEffect(() => {
    fetcherRef.current = fetcher;
  }, [fetcher]);

  useEffect(() => {
    alive.current = true;
    return () => {
      alive.current = false;
    };
  }, []);

  const run = useCallback(
    async (mode: "initial" | "manual" | "poll") => {
      if (inFlight.current) return;
      inFlight.current = true;
      if (mode === "manual") setRefreshing(true);
      try {
        const next = await fetcherRef.current();
        if (!alive.current) return;
        setData(next);
        setError(null);
      } catch (err) {
        if (!alive.current) return;
        // Deliberately does NOT clear `data`. Losing the network must not look
        // the same as having no records.
        setError(
          err instanceof Error ? err.message : "Could not reach the server.",
        );
      } finally {
        inFlight.current = false;
        if (alive.current) {
          if (mode === "initial") setLoading(false);
          if (mode === "manual") setRefreshing(false);
        }
      }
    },
    [],
  );

  useEffect(() => {
    if (!enabled) return;

    // Kick the first load off the synchronous effect body. Beyond satisfying
    // the cascading-render lint, this lets the screen paint its loading state
    // before any state update from the response can land.
    const initial = setTimeout(() => void run("initial"), 0);

    if (!pollMs) return () => clearTimeout(initial);

    const iv = setInterval(() => {
      // Polling a backgrounded app burns battery and mobile data for a screen
      // nobody is looking at.
      if (document.visibilityState === "visible") void run("poll");
    }, pollMs);

    return () => {
      clearTimeout(initial);
      clearInterval(iv);
    };
  }, [enabled, pollMs, run]);

  const refresh = useCallback(() => run("manual"), [run]);

  return { data, loading, error, refreshing, refresh, setData };
}
