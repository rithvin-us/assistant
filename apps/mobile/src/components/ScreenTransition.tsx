/**
 * ScreenTransition — entry motion for a swapped screen.
 *
 * `App` renders one screen at a time from a state machine, so there is no
 * router to hang transitions off. This wraps whichever screen is active and
 * animates it in when the identity changes, which is enough to say "this came
 * from over there" without holding the user up.
 *
 * Deliberately entry-only. Animating the outgoing screen would mean keeping two
 * screens mounted, each with its own polling and network traffic, to decorate a
 * 200ms moment. Not worth it in a tool used dozens of times a day.
 *
 * The transition never blocks input: the incoming screen is interactive on its
 * first frame, and the motion is purely visual. Reduced-motion drops it
 * entirely rather than shortening it.
 */

import { useEffect, useState } from "react";
import type { ReactNode } from "react";
import Box from "@mui/material/Box";

import { MOTION, prefersReducedMotion } from "../lib/motion";

interface ScreenTransitionProps {
  /** Changes whenever a different screen is shown. Drives the animation. */
  screenKey: string;
  /** True when moving back toward the root, so the motion reverses. */
  back?: boolean;
  children: ReactNode;
}

export default function ScreenTransition({
  screenKey,
  back = false,
  children,
}: ScreenTransitionProps) {
  const [entering, setEntering] = useState(false);
  const [previousKey, setPreviousKey] = useState(screenKey);

  // Detected during render rather than in an effect, so the incoming screen's
  // very first paint is already offset. Catching it in an effect would show one
  // frame at the final position and then jump back to animate, which flickers.
  if (screenKey !== previousKey) {
    setPreviousKey(screenKey);
    if (!prefersReducedMotion()) setEntering(true);
  }

  useEffect(() => {
    if (!entering) return;
    // Release on the next frame, once the browser has a "from" value to
    // animate out of.
    const raf = requestAnimationFrame(() => setEntering(false));
    return () => cancelAnimationFrame(raf);
  }, [entering]);

  // Forward motion enters from the right, back motion from the left, so the
  // direction of travel matches where the user thinks they are going.
  const offset = back ? -16 : 16;

  return (
    <Box
      sx={{
        height: "100%",
        opacity: entering ? 0 : 1,
        transform: entering ? `translateX(${offset}px)` : "translateX(0)",
        transition: entering
          ? "none"
          : `opacity ${MOTION.duration.base}ms ${MOTION.easing.decelerate}, transform ${MOTION.duration.base}ms ${MOTION.easing.decelerate}`,
      }}
    >
      {children}
    </Box>
  );
}
