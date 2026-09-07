/**
 * Haptic feedback.
 *
 * KNOWN LIMITATION -- read before relying on this.
 *
 * On Android these calls are currently INERT. `navigator.vibrate` requires
 * `android.permission.VIBRATE` in the manifest, and the generated manifest at
 * `apps/mobile/src-tauri/gen/android/app/src/main/AndroidManifest.xml` declares
 * only `INTERNET`. That whole `gen/android` tree is gitignored, so adding the
 * permission there would work on one machine and vanish on the next
 * regeneration -- not a reproducible fix. See docs/M11-INTERACTION.md.
 *
 * The module is written so that granting the permission (or adopting the Tauri
 * haptics plugin behind `vibrate()`) makes every existing call site start
 * working with no further changes. It never pretends: when the platform gives
 * us nothing, `haptic()` returns false and callers carry on silently. Nothing
 * visual is driven off the return value, so behaviour is identical either way.
 *
 * Restraint is deliberate. Only four events buzz -- crossing a swipe threshold,
 * confirming a destructive action, completing something, and picking up a row
 * to reorder. Taps and scrolls never do.
 */

export type HapticKind =
  /** A drag crossed the point where releasing would commit. */
  | "threshold"
  /** A destructive or otherwise consequential action was carried out. */
  | "confirm"
  /** Something finished successfully (a task completed). */
  | "success"
  /** A row was picked up for reordering. */
  | "pickup";

/**
 * Durations in ms. Short and dry: a long buzz on a phone in a pocket during a
 * list scroll is worse than no feedback at all.
 */
const PATTERN: Record<HapticKind, number | number[]> = {
  threshold: 10,
  confirm: [0, 18, 40, 18],
  success: 16,
  pickup: 12,
};

/** Cached so a missing API is probed once rather than on every gesture frame. */
let supported: boolean | null = null;

export function hapticsSupported(): boolean {
  if (supported !== null) return supported;
  supported =
    typeof navigator !== "undefined" && typeof navigator.vibrate === "function";
  return supported;
}

/**
 * Fires the pattern for `kind`.
 *
 * Returns whether the platform accepted it, so a diagnostic screen can report
 * the truth. Callers are expected to ignore it.
 */
export function haptic(kind: HapticKind): boolean {
  if (!hapticsSupported()) return false;
  // Respect the same accessibility signal as motion: a user who suppressed
  // animation generally does not want the device buzzing either.
  if (
    typeof window !== "undefined" &&
    window.matchMedia?.("(prefers-reduced-motion: reduce)").matches
  ) {
    return false;
  }
  try {
    return navigator.vibrate(PATTERN[kind]);
  } catch {
    // Some WebViews throw instead of returning false when the permission is
    // absent. A failed haptic must never break the interaction that caused it.
    return false;
  }
}
