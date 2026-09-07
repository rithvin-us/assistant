/**
 * Pure gesture decision logic.
 *
 * Kept free of React and the DOM so the rules that decide whether a drag
 * belongs to a row or to the scroll container -- and whether releasing commits
 * or cancels -- can be tested directly, without simulating pointer streams.
 * SwipeableRow and PullToRefresh own the state and the events; everything they
 * actually *decide* lives here.
 */

/** Movement, in px, before a gesture commits to an axis. */
export const AXIS_LOCK_SLOP = 8;
/** Fraction of the tray that must be uncovered for a row to snap open. */
export const OPEN_THRESHOLD = 0.4;
/** px/ms past which a flick decides the outcome regardless of distance. */
export const FLING_VELOCITY = 0.45;

export type Axis = "undecided" | "horizontal" | "vertical";

/**
 * Decides which axis a drag belongs to.
 *
 * Ties go to vertical: scrolling a list is the more common intent, and stealing
 * it is far more annoying than missing a swipe. Returns "undecided" while the
 * movement is still inside the slop radius, so a tap with a shaky finger never
 * commits to anything.
 */
export function resolveAxis(dx: number, dy: number, slop = AXIS_LOCK_SLOP): Axis {
  if (Math.abs(dx) < slop && Math.abs(dy) < slop) return "undecided";
  return Math.abs(dx) > Math.abs(dy) ? "horizontal" : "vertical";
}

/**
 * Clamps a raw drag offset to the tray, rubber-banding past both ends so the
 * row reads as bounded rather than broken.
 *
 * `raw` is negative when dragging left (opening). The result is never allowed
 * to run away from the tray on either side.
 */
export function clampOffset(raw: number, trayWidth: number, resistance = 0.25): number {
  if (raw > 0) return raw * resistance;
  if (raw < -trayWidth) return -trayWidth + (raw + trayWidth) * resistance;
  return raw;
}

/**
 * Decides where a row lands when the finger lifts.
 *
 * A decisive flick wins outright -- that is what makes a short, fast swipe feel
 * right -- and otherwise the decision falls back to how far the tray was
 * actually uncovered.
 */
export function settleOpen(
  offset: number,
  velocity: number,
  trayWidth: number,
  threshold = OPEN_THRESHOLD,
  fling = FLING_VELOCITY,
): boolean {
  if (velocity < -fling) return true;
  if (velocity > fling) return false;
  return offset <= -trayWidth * threshold;
}

/**
 * Whether a downward drag should arm pull-to-refresh.
 *
 * Requires the container to already be at the top, and the drag to be downward
 * and mostly vertical. Anything else is an ordinary scroll and must be left
 * entirely alone.
 */
export function shouldArmPull(
  scrollTop: number,
  dx: number,
  dy: number,
  slop = AXIS_LOCK_SLOP,
): boolean {
  if (scrollTop > 0) return false;
  if (Math.abs(dy) < slop && Math.abs(dx) < slop) return false;
  return dy > 0 && Math.abs(dy) >= Math.abs(dx);
}

/** Applies pull resistance and caps travel, so the indicator stays attached. */
export function pullDistance(dy: number, max: number, resistance = 0.5): number {
  return Math.min(max, Math.max(0, dy * resistance));
}
