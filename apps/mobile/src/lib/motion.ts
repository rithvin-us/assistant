/**
 * Motion tokens.
 *
 * One vocabulary so transitions across screens agree. The assistant is a
 * daily-use tool, so durations are short by design: motion here exists to say
 * where something came from and where it went, not to decorate. Nothing should
 * exceed `slow`, and `slow` is reserved for full-screen transitions.
 *
 * `prefersReducedMotion` is read at call time rather than cached, because
 * Android users can toggle "Remove animations" without restarting the app.
 */

export const MOTION = {
  duration: {
    /** Press/release feedback, ripples, opacity swaps. */
    fast: 120,
    /** Row snaps, sheet content, list item enter/exit. */
    base: 200,
    /** Full-screen and sheet presentation. */
    slow: 280,
  },
  easing: {
    /** Symmetric moves that both start and end on screen. */
    standard: "cubic-bezier(0.2, 0, 0, 1)",
    /** Entering the screen: fast start, soft landing. */
    decelerate: "cubic-bezier(0, 0, 0, 1)",
    /** Leaving the screen: gentle start, quick exit. */
    accelerate: "cubic-bezier(0.3, 0, 1, 1)",
  },
} as const;

/**
 * Whether the user asked the system to suppress animation.
 *
 * Android's "Remove animations" accessibility setting surfaces to the WebView
 * as `prefers-reduced-motion: reduce`.
 */
export function prefersReducedMotion(): boolean {
  if (typeof window === "undefined" || !window.matchMedia) return false;
  return window.matchMedia("(prefers-reduced-motion: reduce)").matches;
}

/**
 * A transition string that collapses to `none` when the user has asked for
 * reduced motion. Use for decorative movement; keep opacity-only feedback
 * (which does not induce motion sickness) unconditional.
 */
export function motionSafeTransition(value: string): string {
  return prefersReducedMotion() ? "none" : value;
}
