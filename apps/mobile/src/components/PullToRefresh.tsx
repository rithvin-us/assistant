/**
 * PullToRefresh — overscroll-to-refresh for server-backed scroll containers.
 *
 * Only engages when the container is already scrolled to the very top, so it
 * never competes with normal scrolling, and it locks to the vertical axis the
 * same way SwipeableRow locks to horizontal. `touchAction: pan-y` is left in
 * place so the Android system back gesture and the WebView's own fling keep
 * working.
 *
 * Honesty rules this component enforces:
 *   - `onRefresh` is the screen's real data path. Nothing here fabricates rows.
 *   - A refresh already in flight swallows further pulls, so a user cannot
 *     stack duplicate requests by yanking the list repeatedly.
 *   - A rejected refresh surfaces through the caller's own error state; this
 *     component simply stops spinning. It never reports success it did not see.
 */

import { useCallback, useRef, useState } from "react";
import type { PointerEvent as ReactPointerEvent, ReactNode } from "react";
import Box from "@mui/material/Box";
import CircularProgress from "@mui/material/CircularProgress";

import { haptic } from "../lib/haptics";
import { MOTION, prefersReducedMotion } from "../lib/motion";
import { pullDistance, shouldArmPull } from "../lib/gesture";

interface PullToRefreshProps {
  children: ReactNode;
  /** The screen's real refresh path. Rejections are the caller's to surface. */
  onRefresh: () => Promise<void>;
  /** Disables the gesture (offline, already loading, non-refreshable state). */
  disabled?: boolean;
  /** Applied to the scrolling element so callers keep their own layout. */
  sx?: object;
}

/** Distance, in px, the user must drag before a release triggers a refresh. */
const TRIGGER_DISTANCE = 64;
/** Hard cap on how far the indicator travels, independent of drag length. */
const MAX_PULL = 96;
export default function PullToRefresh({
  children,
  onRefresh,
  disabled = false,
  sx,
}: PullToRefreshProps) {
  const [pull, setPull] = useState(0);
  const [refreshing, setRefreshing] = useState(false);
  const [dragging, setDragging] = useState(false);

  const scroller = useRef<HTMLDivElement | null>(null);
  const startY = useRef(0);
  const startX = useRef(0);
  const active = useRef(false);
  const passedTrigger = useRef(false);

  const handlePointerDown = (e: ReactPointerEvent<HTMLDivElement>) => {
    if (disabled || refreshing) return;
    // Only arm the gesture at the top of the list; anywhere else this is a
    // normal scroll and must be left entirely alone.
    if ((scroller.current?.scrollTop ?? 0) > 0) return;
    startY.current = e.clientY;
    startX.current = e.clientX;
    active.current = false;
    passedTrigger.current = false;
    setDragging(true);
  };

  const handlePointerMove = (e: ReactPointerEvent<HTMLDivElement>) => {
    if (disabled || refreshing || !dragging) return;

    const dy = e.clientY - startY.current;
    const dx = e.clientX - startX.current;

    if (!active.current) {
      const scrollTop = scroller.current?.scrollTop ?? 0;
      if (!shouldArmPull(scrollTop, dx, dy)) {
        // Not ours: either still inside the slop radius, or an ordinary scroll.
        if (Math.abs(dy) >= 6 || Math.abs(dx) >= 6) setDragging(false);
        return;
      }
      active.current = true;
    }

    const next = pullDistance(dy, MAX_PULL);
    setPull(next);

    const crossed = next >= TRIGGER_DISTANCE;
    if (crossed !== passedTrigger.current) {
      passedTrigger.current = crossed;
      if (crossed) haptic("threshold");
    }
  };

  const finish = useCallback(async () => {
    setDragging(false);
    if (!active.current || pull < TRIGGER_DISTANCE) {
      setPull(0);
      return;
    }
    // Park the indicator at the trigger point while the real request runs.
    setRefreshing(true);
    setPull(TRIGGER_DISTANCE);
    try {
      await onRefresh();
    } finally {
      // Whether it resolved or rejected, stop spinning. Success is the
      // caller's to claim, not ours.
      setRefreshing(false);
      setPull(0);
      active.current = false;
    }
  }, [pull, onRefresh]);

  const handlePointerCancel = () => {
    // Interruption is not a refresh request.
    setDragging(false);
    active.current = false;
    setPull(0);
  };

  const reduced = prefersReducedMotion();

  return (
    <Box sx={{ position: "relative", flex: 1, minHeight: 0, display: "flex", flexDirection: "column" }}>
      <Box
        aria-hidden={pull === 0}
        sx={{
          position: "absolute",
          top: 0,
          left: 0,
          right: 0,
          height: pull,
          display: "flex",
          alignItems: "center",
          justifyContent: "center",
          overflow: "hidden",
          pointerEvents: "none",
          opacity: pull > 0 ? 1 : 0,
          transition: dragging
            ? "none"
            : `height ${MOTION.duration.base}ms ${MOTION.easing.decelerate}, opacity ${MOTION.duration.fast}ms linear`,
          zIndex: 1,
        }}
      >
        <CircularProgress
          size={22}
          thickness={4}
          // Before the trigger the ring tracks the drag; after it, it spins.
          variant={refreshing || reduced ? "indeterminate" : "determinate"}
          value={Math.min(100, (pull / TRIGGER_DISTANCE) * 100)}
        />
      </Box>

      <Box
        ref={scroller}
        onPointerDown={handlePointerDown}
        onPointerMove={handlePointerMove}
        onPointerUp={() => void finish()}
        onPointerCancel={handlePointerCancel}
        sx={{
          flex: 1,
          minHeight: 0,
          overflowY: "auto",
          // Momentum/fling on the WebView's own scroller.
          WebkitOverflowScrolling: "touch",
          overscrollBehaviorY: "contain",
          touchAction: "pan-y",
          transform: `translateY(${pull}px)`,
          transition: dragging
            ? "none"
            : `transform ${MOTION.duration.base}ms ${MOTION.easing.decelerate}`,
          ...sx,
        }}
      >
        {children}
      </Box>
    </Box>
  );
}
