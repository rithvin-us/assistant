/**
 * Which Google scopes each feature needs.
 *
 * Kept out of the component file so the picker exports only a component, and
 * so the server's scope list has exactly one mirror on this side.
 *
 * Consent is not incremental: an account connected before a feature existed
 * did not grant its scope, and every request against it will fail with a
 * permission error. Checking here lets the UI say "reconnect this account"
 * instead of surfacing an unexplained failure.
 */

import type { AccountSummary } from "./types";

export type ScopedFeature = "classroom" | "drive";

const REQUIRED: Record<ScopedFeature, string[]> = {
  classroom: ["classroom.courses.readonly", "classroom.coursework.me.readonly"],
  drive: ["drive.readonly"],
};

export function hasScopesFor(
  account: AccountSummary | undefined,
  feature: ScopedFeature,
): boolean {
  if (!account) return false;
  return REQUIRED[feature].every((needle) =>
    account.scopes.some((granted) => granted.endsWith(needle)),
  );
}
