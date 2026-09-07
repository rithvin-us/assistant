/**
 * useAndroidBack — makes the Android back button and back gesture mean "go
 * back", not "quit".
 *
 * The app navigates by swapping a screen out of a state machine, so as far as
 * the WebView is concerned nothing ever happened: the history stack stays one
 * entry deep and Android's back button leaves the app entirely. From a
 * sub-screen that looks like the app dying.
 *
 * The fix is to give the system something to pop. Entering a sub-screen pushes
 * a history entry; Android's back — button or edge gesture, they are the same
 * event here — pops it and we route that to the caller's handler instead of
 * letting the activity finish.
 *
 * This deliberately does NOT hijack the gesture. We never call
 * `preventDefault`, never bind a touch handler at the screen edge, and never
 * suppress the default when we are already at the root — back from the home
 * screen should still leave the app, because that is what the user means.
 */

import { useEffect, useRef } from "react";

/** Marks the entries this hook owns, so it ignores anyone else's. */
const MARKER = "assistant:screen";

/**
 * @param active  True while a back-consuming screen is open.
 * @param onBack  Runs when the user goes back. Should return to the previous
 *                screen; it must not itself push history.
 */
export function useAndroidBack(active: boolean, onBack: () => void): void {
  // Held in a ref so the subscription depends only on `active`. Callers pass
  // inline arrows, and a new identity on every render would otherwise tear the
  // subscription down and push a fresh history entry each time -- growing the
  // stack without bound and making back appear to do nothing.
  const handler = useRef(onBack);
  useEffect(() => {
    handler.current = onBack;
  }, [onBack]);

  useEffect(() => {
    if (!active) return;

    // One entry per activation. Re-entering pushes a fresh one, so a deep path
    // unwinds a step at a time rather than jumping straight to the root.
    window.history.pushState({ [MARKER]: true }, "");

    const onPopState = () => {
      handler.current();
    };

    window.addEventListener("popstate", onPopState);

    return () => {
      window.removeEventListener("popstate", onPopState);

      // The screen closed some other way -- a back arrow, a menu choice -- so
      // the entry we pushed is stale. Drop it, or the next system back would
      // be swallowed doing nothing and the app would feel stuck.
      if (window.history.state?.[MARKER]) {
        window.history.back();
      }
    };
  }, [active]);
}
