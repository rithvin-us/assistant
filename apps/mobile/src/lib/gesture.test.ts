/**
 * Tests for the gesture decision rules.
 *
 * These assert real behaviour the UI depends on -- axis arbitration, threshold
 * commitment, cancellation, and the scroll-vs-swipe boundary -- rather than
 * re-stating constants back to themselves.
 */

import { describe, expect, it } from "vitest";

import {
  AXIS_LOCK_SLOP,
  clampOffset,
  pullDistance,
  resolveAxis,
  settleOpen,
  shouldArmPull,
} from "./gesture";

describe("resolveAxis", () => {
  it("stays undecided inside the slop radius so a shaky tap commits to nothing", () => {
    expect(resolveAxis(0, 0)).toBe("undecided");
    expect(resolveAxis(AXIS_LOCK_SLOP - 1, AXIS_LOCK_SLOP - 1)).toBe("undecided");
  });

  it("claims the gesture for the row only when it is clearly horizontal", () => {
    expect(resolveAxis(30, 4)).toBe("horizontal");
    expect(resolveAxis(-30, 4)).toBe("horizontal");
  });

  it("gives the gesture to the scroll container when it is vertical", () => {
    expect(resolveAxis(4, 30)).toBe("vertical");
    expect(resolveAxis(4, -30)).toBe("vertical");
  });

  it("breaks ties toward vertical, because hijacking a scroll is worse than missing a swipe", () => {
    expect(resolveAxis(20, 20)).toBe("vertical");
  });

  it("commits as soon as either axis leaves the slop radius, even diagonally", () => {
    expect(resolveAxis(0, AXIS_LOCK_SLOP)).toBe("vertical");
    expect(resolveAxis(AXIS_LOCK_SLOP, 0)).toBe("horizontal");
  });
});

describe("clampOffset", () => {
  const TRAY = 200;

  it("tracks the finger one-to-one inside the tray", () => {
    expect(clampOffset(-50, TRAY)).toBe(-50);
    expect(clampOffset(-TRAY, TRAY)).toBe(-TRAY);
  });

  it("rubber-bands past the open end instead of running away", () => {
    const past = clampOffset(-300, TRAY);
    expect(past).toBeLessThan(-TRAY);
    expect(past).toBeGreaterThan(-300);
  });

  it("resists dragging a closed row to the right", () => {
    expect(clampOffset(100, TRAY)).toBe(25);
  });
});

describe("settleOpen", () => {
  const TRAY = 200;

  it("cancels when the finger lifts short of the threshold", () => {
    expect(settleOpen(-40, 0, TRAY)).toBe(false);
  });

  it("opens once the drag passes the threshold", () => {
    expect(settleOpen(-120, 0, TRAY)).toBe(true);
  });

  it("opens on a fast flick that never reached the threshold", () => {
    expect(settleOpen(-20, -1.2, TRAY)).toBe(true);
  });

  it("closes on a fast flick back, even from fully open", () => {
    expect(settleOpen(-TRAY, 1.2, TRAY)).toBe(false);
  });

  it("ignores slow drift and falls back to distance", () => {
    expect(settleOpen(-120, 0.1, TRAY)).toBe(true);
    expect(settleOpen(-20, -0.1, TRAY)).toBe(false);
  });
});

describe("shouldArmPull", () => {
  it("refuses to arm when the list is scrolled away from the top", () => {
    expect(shouldArmPull(120, 0, 40)).toBe(false);
  });

  it("arms on a downward drag at the top", () => {
    expect(shouldArmPull(0, 0, 40)).toBe(true);
  });

  it("ignores upward drags, which are ordinary scrolling", () => {
    expect(shouldArmPull(0, 0, -40)).toBe(false);
  });

  it("ignores mostly-horizontal drags so row swipes still work at the top", () => {
    expect(shouldArmPull(0, 60, 20)).toBe(false);
  });

  it("stays disarmed inside the slop radius", () => {
    expect(shouldArmPull(0, 1, 2)).toBe(false);
  });
});

describe("pullDistance", () => {
  it("applies resistance so the indicator feels attached to the list", () => {
    expect(pullDistance(100, 96)).toBe(50);
  });

  it("caps travel no matter how far the finger goes", () => {
    expect(pullDistance(1000, 96)).toBe(96);
  });

  it("never reports a negative pull", () => {
    expect(pullDistance(-50, 96)).toBe(0);
  });
});
