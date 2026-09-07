/**
 * SwipeableRow — swipe-to-reveal actions for list rows.
 *
 * Deliberately swipe-to-REVEAL, not swipe-to-act. A horizontal swipe uncovers
 * an action tray; running an action still needs an explicit tap. That keeps
 * destructive operations off a single gesture (no destructive action is
 * reachable by gesture alone) and gives TalkBack users the same buttons without
 * performing any gesture -- the tray is in the accessibility tree at all times,
 * only visually clipped.
 *
 * Axis locking: the first few pixels of movement decide whether the gesture
 * belongs to this row (horizontal) or to the scroll container (vertical). Once
 * the vertical lock is taken the row never moves, so list scrolling is not
 * hijacked. Pointer cancellation -- an incoming call, the Android back gesture,
 * the WebView stealing capture -- snaps the row closed rather than leaving it
 * stranded mid-swipe.
 *
 * No animation library: a CSS transform plus a transition, disabled while the
 * finger is down so tracking is 1:1, re-enabled on release so the snap animates.
 */

import { useCallback, useRef, useState } from "react";
import type { PointerEvent as ReactPointerEvent, ReactNode } from "react";
import Box from "@mui/material/Box";
import ButtonBase from "@mui/material/ButtonBase";
import Typography from "@mui/material/Typography";

import { haptic } from "../lib/haptics";
import { MOTION } from "../lib/motion";
import { OPEN_THRESHOLD, clampOffset, resolveAxis, settleOpen } from "../lib/gesture";
import type { Axis } from "../lib/gesture";

/** One revealed action. `destructive` only changes affordance, never behaviour. */
export interface SwipeAction {
  id: string;
  label: string;
  icon: ReactNode;
  /** Tray background behind this action. */
  color: string;
  /** Foreground colour for icon + label. */
  textColor?: string;
  destructive?: boolean;
  /** Invoked on explicit tap. May be async; the row closes once it settles. */
  onPress: () => void | Promise<void>;
}

interface SwipeableRowProps {
  children: ReactNode;
  actions: SwipeAction[];
  /** Width of a single action button. The tray is `actions.length * this`. */
  actionWidth?: number;
  /** Suppresses gesture handling entirely (loading, offline, read-only rows). */
  disabled?: boolean;
  /** Background behind the row content, so the tray never shows through. */
  background?: string;
  /** Notifies the parent so it can keep only one row open at a time. */
  onOpenChange?: (open: boolean) => void;
  /** Parent-driven close, used to enforce single-open-row behaviour. */
  forceClosed?: boolean;
}

export default function SwipeableRow({
  children,
  actions,
  actionWidth = 76,
  disabled = false,
  background = "transparent",
  onOpenChange,
  forceClosed = false,
}: SwipeableRowProps) {
  const trayWidth = actions.length * actionWidth;

  const [offset, setOffset] = useState(0);
  const [dragging, setDragging] = useState(false);
  const [busy, setBusy] = useState(false);

  const startX = useRef(0);
  const startY = useRef(0);
  const startOffset = useRef(0);
  const lastX = useRef(0);
  const lastT = useRef(0);
  const velocity = useRef(0);
  const axis = useRef<Axis>("undecided");
  /** Guards the one-shot haptic so crossing the threshold buzzes once, not per frame. */
  const passedThreshold = useRef(false);
  /** Distinguishes a tap from the tail of a swipe when the row is released. */
  const moved = useRef(false);

  const open = useCallback(() => {
    setOffset(-trayWidth);
    onOpenChange?.(true);
  }, [trayWidth, onOpenChange]);

  const close = useCallback(() => {
    setOffset(0);
    onOpenChange?.(false);
  }, [onOpenChange]);

  // The parent closes other rows when one opens. Adjusting state during render
  // (React's documented derived-state escape hatch) rather than in an effect:
  // an effect would paint the row open for one frame and then snap it shut.
  const [prevForceClosed, setPrevForceClosed] = useState(forceClosed);
  if (forceClosed !== prevForceClosed) {
    setPrevForceClosed(forceClosed);
    if (forceClosed) setOffset(0);
  }

  const handlePointerDown = (e: ReactPointerEvent<HTMLDivElement>) => {
    if (disabled || busy) return;
    if (e.pointerType === "mouse" && e.button !== 0) return;
    startX.current = e.clientX;
    startY.current = e.clientY;
    lastX.current = e.clientX;
    lastT.current = e.timeStamp;
    startOffset.current = offset;
    velocity.current = 0;
    axis.current = "undecided";
    passedThreshold.current = offset !== 0;
    moved.current = false;
    setDragging(true);
  };

  const handlePointerMove = (e: ReactPointerEvent<HTMLDivElement>) => {
    if (disabled || busy || !dragging) return;

    const dx = e.clientX - startX.current;
    const dy = e.clientY - startY.current;

    if (axis.current === "undecided") {
      const resolved = resolveAxis(dx, dy);
      if (resolved === "undecided") return;
      axis.current = resolved;
      if (axis.current === "vertical") {
        setDragging(false);
        return;
      }
      moved.current = true;
      e.currentTarget.setPointerCapture?.(e.pointerId);
    }

    if (axis.current !== "horizontal") return;

    const dt = e.timeStamp - lastT.current;
    if (dt > 0) {
      velocity.current = (e.clientX - lastX.current) / dt;
      lastX.current = e.clientX;
      lastT.current = e.timeStamp;
    }

    const next = clampOffset(startOffset.current + dx, trayWidth);
    setOffset(next);

    const crossed = next <= -trayWidth * OPEN_THRESHOLD;
    if (crossed !== passedThreshold.current) {
      passedThreshold.current = crossed;
      if (crossed) haptic("threshold");
    }
  };

  const handlePointerUp = (e: ReactPointerEvent<HTMLDivElement>) => {
    e.currentTarget.releasePointerCapture?.(e.pointerId);
    if (!dragging) return;
    setDragging(false);
    if (axis.current !== "horizontal") return;

    if (settleOpen(offset, velocity.current, trayWidth)) return open();
    close();
  };

  /** Cancellation must never leave the row stranded mid-swipe. */
  const handlePointerCancel = (e: ReactPointerEvent<HTMLDivElement>) => {
    e.currentTarget.releasePointerCapture?.(e.pointerId);
    setDragging(false);
    axis.current = "undecided";
    close();
  };

  const runAction = async (action: SwipeAction) => {
    if (busy) return; // Repeated taps must not fire the operation twice.
    setBusy(true);
    try {
      await action.onPress();
      if (action.destructive) haptic("confirm");
    } finally {
      setBusy(false);
      close();
    }
  };

  const isOpen = offset !== 0;

  return (
    <Box sx={{ position: "relative", overflow: "hidden", bgcolor: background }}>
      {/* Always mounted so TalkBack reaches these buttons with no gesture. */}
      <Box
        sx={{
          position: "absolute",
          top: 0,
          bottom: 0,
          right: 0,
          display: "flex",
          width: trayWidth,
        }}
      >
        {actions.map((action) => (
          <ButtonBase
            key={action.id}
            onClick={() => void runAction(action)}
            disabled={busy || disabled}
            aria-label={action.label}
            sx={{
              width: actionWidth,
              flexDirection: "column",
              gap: 0.5,
              bgcolor: action.color,
              color: action.textColor ?? "#fff",
              opacity: busy ? 0.6 : 1,
              transition: `opacity ${MOTION.duration.fast}ms ${MOTION.easing.standard}`,
            }}
          >
            {action.icon}
            <Typography sx={{ fontSize: "0.68rem", fontWeight: 600, lineHeight: 1 }}>
              {action.label}
            </Typography>
          </ButtonBase>
        ))}
      </Box>

      <Box
        onPointerDown={handlePointerDown}
        onPointerMove={handlePointerMove}
        onPointerUp={handlePointerUp}
        onPointerCancel={handlePointerCancel}
        // A tap that ends a swipe must not also activate the row beneath.
        onClickCapture={(e) => {
          if (moved.current || isOpen) {
            e.stopPropagation();
            e.preventDefault();
            if (isOpen) close();
          }
        }}
        sx={{
          position: "relative",
          bgcolor: background,
          transform: `translateX(${offset}px)`,
          transition: dragging
            ? "none"
            : `transform ${MOTION.duration.base}ms ${MOTION.easing.decelerate}`,
          // Let the browser own vertical panning; we only ever take the X axis.
          touchAction: "pan-y",
        }}
      >
        {children}
      </Box>
    </Box>
  );
}
