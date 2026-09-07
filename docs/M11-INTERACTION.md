# M11 — Native Android interaction and motion

Status: **substantially complete**. Primitives are built, tested and verified on
hardware, and rolled out across the mutation-bearing screens. Some read-mostly
screens and the two hand-rolled drawer drags remain; see "What is not done".

## Principles

The backend is the system of record. A gesture is only a UI trigger: it calls
the same authorised operation a button calls, and the UI reflects what the
server actually returned. No gesture asserts a durable change the server has
not confirmed.

Three rules follow from that, and every gesture below obeys them:

1. **No destructive action is reachable by gesture alone.** Swiping reveals an
   action tray; running the action still needs a deliberate tap.
2. **Every gesture action has a non-gesture equivalent**, visible on the row or
   in the item's editor, so nothing is discoverable only by swiping.
3. **A failed request rolls the UI back** or surfaces an error. It never leaves
   the screen asserting something that did not happen.

## Motion

Tokens live in `apps/mobile/src/lib/motion.ts`. One vocabulary, deliberately
short, because this is a daily-use tool:

| Token | Duration | Used for |
| --- | --- | --- |
| `fast` | 120 ms | press/release feedback, opacity swaps |
| `base` | 200 ms | row snaps, list item enter/exit |
| `slow` | 280 ms | full-screen and sheet presentation |

Easing: `standard` for moves that start and end on screen, `decelerate` for
entrances, `accelerate` for exits.

`prefersReducedMotion()` reads Android's "Remove animations" accessibility
setting at call time, not once at startup, because users toggle it without
restarting. `motionSafeTransition()` collapses decorative movement to `none`
when it is set. Opacity-only feedback stays on, since it does not induce motion
sickness and removing it would leave taps with no acknowledgement.

## Gesture map

Only rows that are actually implemented are listed. Reversibility and
confirmation reflect the existing backend lifecycle, which M11 did not change.

### Notes — `NotesScreen.tsx`

| Element | Gesture | Action | Reversible | Confirm | Non-gesture alternative | Haptic | Backend | On failure |
| --- | --- | --- | --- | --- | --- | --- | --- | --- |
| Note row | swipe left | reveals tray | n/a | no | tray is always in the a11y tree | at threshold | none | snaps closed |
| Tray → Pin | tap | toggle `is_pinned` | yes | no | inline pin icon on row | no | `PATCH /v1/notes/{id}` | error shown |
| Tray → Archive | tap | toggle `is_archived` | yes | no | inline archive icon on row | no | `PATCH /v1/notes/{id}` | error shown |
| Tray → Delete | tap | delete note | no | tap is the confirmation | editor | on completion | `DELETE /v1/notes/{id}` | row stays, error shown |
| List | pull down at top | refresh | n/a | no | 15 s poll | at threshold | `GET /v1/notes` | spinner stops, error shown |
| FAB | tap | new note | yes | no | pencil in header | no | none until saved | n/a |

### Tasks — `TasksScreen.tsx`

| Element | Gesture | Action | Reversible | Confirm | Non-gesture alternative | Haptic | Backend | On failure |
| --- | --- | --- | --- | --- | --- | --- | --- | --- |
| Task row | swipe left | reveals tray | n/a | no | tray is always in the a11y tree | at threshold | none | snaps closed |
| Tray → Done | tap | toggle status | yes | no | priority checkbox on row | on completion | `PATCH /v1/tasks/{id}` | **rolls back**, error shown |
| Tray → Delete | tap | delete task | no | tap is the confirmation | delete icon on row | on completion | `DELETE /v1/tasks/{id}` | row stays, error shown |

Task completion is the one deliberately optimistic path: it is frequent and
reversible, and a checkbox that waits for a round trip feels broken. It rolls
back to the previous status when the request fails.

### Reminders — `RemindersScreen.tsx`

| Element | Gesture | Action | Reversible | Confirm | Non-gesture alternative | Haptic | Backend | On failure |
| --- | --- | --- | --- | --- | --- | --- | --- | --- |
| Reminder row | swipe left | reveals tray | n/a | no | tray is always in the a11y tree | at threshold | none | snaps closed |
| Tray → Done | tap | toggle `status: handled` | yes | no | labelled control on row | on completion | `PATCH /v1/reminders/{id}` | **rolls back**, error shown |
| Tray → Delete | tap | delete reminder | no | tap is the confirmation | labelled icon on row | on completion | `DELETE /v1/reminders/{id}` | row stays, error shown |
| List | pull down at top | refresh | n/a | no | existing poll | at threshold | `GET /v1/reminders` | spinner stops, error shown |

### Ideas — `IdeasScreen.tsx`

| Element | Gesture | Action | Reversible | Confirm | Non-gesture alternative | Haptic | Backend | On failure |
| --- | --- | --- | --- | --- | --- | --- | --- | --- |
| Idea row | swipe left | reveals tray | n/a | no | tray is always in the a11y tree | at threshold | none | snaps closed |
| Tray → To Task | tap | convert to task | no | tap is the confirmation | labelled icon on row | no | `POST /v1/ideas/{id}/convert` | error shown |
| Tray → Delete | tap | delete idea | no | tap is the confirmation | labelled icon on row | on completion | `DELETE /v1/ideas/{id}` | row stays, error shown |
| List | pull down at top | refresh | n/a | no | existing poll | at threshold | `GET /v1/ideas` | spinner stops, error shown |

### Memory and Academic

Pull-to-refresh only. Memory keeps its existing non-destructive archive/restore
lifecycle: no swipe actions, no delete. Academic is read-only.

## Navigation and system gestures

`App.tsx` swaps one screen at a time out of a state machine, which had two
consequences M11 fixes.

**Back.** The history stack stayed one entry deep, so Android's back button and
back gesture left the app entirely — from a sub-screen that reads as a crash.
`lib/useAndroidBack.ts` pushes a history entry per screen and routes the pop to
a handler: back returns to home, and closes an open sheet before it touches
navigation. Back from the home screen still exits, because that is what the user
means there.

The system gesture is **not** hijacked. Nothing calls `preventDefault`, no touch
handler is bound in the edge region, and the default is never suppressed at the
root. We give Android something to pop rather than intercepting the gesture.

**Motion.** `components/ScreenTransition.tsx` animates the incoming screen in
over 200 ms from the direction of travel. Entry only: animating the outgoing
screen would mean keeping two screens mounted, each with its own polling and
network traffic, to decorate a moment. Reduced motion drops it entirely.

## Primitives

- `components/SwipeableRow.tsx` — swipe-to-reveal. Locks to an axis after 8 px,
  with ties going to vertical so list scrolling is never hijacked. Snaps closed
  on pointer cancel. `touchAction: pan-y` leaves the Android back gesture and
  the WebView's own fling alone. `busy` guards against a double tap firing an
  operation twice.
- `components/PullToRefresh.tsx` — arms only at `scrollTop === 0`. Calls the
  screen's real fetch. Stops spinning whether the request resolved or rejected,
  and never claims success.
- `lib/useRefreshable.ts` — one loading/error/poll lifecycle. A failed load keeps
  the last good data rather than blanking the list, which would read as "you
  have nothing" when the truth is "we could not reach the server". Polling
  pauses while the app is backgrounded. Overlapping requests are dropped.
- `lib/gesture.ts` — the pure decision rules, extracted so they are testable
  without simulating pointer streams. The components import them; the tests
  exercise the same functions the UI runs.

## Accessibility

The action tray is always mounted and always in the accessibility tree — only
visually clipped — so TalkBack reaches Pin, Archive and Delete without any
gesture. Note rows carry `role="button"`, an `aria-label` naming the note, and
Enter/Space activation. Tray buttons carry explicit `aria-label`s. The delete
icon button on task rows names its task rather than announcing a bare "delete".

Reduced motion is honoured for movement and for haptics: a user who suppressed
animation generally does not want the device buzzing either.

## Haptics — known limitation

**Haptics are inert on Android today.** `navigator.vibrate` requires
`android.permission.VIBRATE`, and the generated manifest at
`apps/mobile/src-tauri/gen/android/app/src/main/AndroidManifest.xml` declares
only `INTERNET`. That whole `gen/android` tree is gitignored, so adding the
permission there works on one machine and disappears on the next regeneration —
not a reproducible fix.

`lib/haptics.ts` therefore feature-detects and returns `false` rather than
pretending. Nothing visual depends on the return value, so behaviour is
identical either way, and granting the permission (or routing `vibrate()`
through the Tauri haptics plugin) makes every existing call site start working
with no further changes.

Resolving this properly needs a decision on whether to commit `gen/android` or
adopt the plugin. That is a repository-structure decision, not an M11 one.

## Verification

Commands run, with actual results:

| Command | Result |
| --- | --- |
| `pnpm --dir apps/mobile typecheck` | pass |
| `pnpm --dir apps/mobile lint` | pass, 0 errors (was 3) |
| `pnpm --dir apps/mobile build` | pass |
| `pnpm --dir apps/mobile test` | 21 passed |
| `cargo fmt --all --check` | pass |
| `cargo clippy --workspace --all-targets -- -D warnings` | pass |
| `cargo test --workspace --no-fail-fast` | see below |

`cargo test` previously failed in two suites, `documents` (9/18) and `memory`
(15/21), all with `relation "..." does not exist`. Migrations 0009-0011 were
pending on the database. After applying them with `sqlx migrate run --source
migrations`, memory passes 21/21 and documents passes 17/18.

The one remaining documents failure is
`upload_rejects_body_over_the_size_limit_with_400`, and it is a Windows
artifact rather than a defect: the server correctly rejects the oversized body
and closes the connection, but Windows surfaces `WSAECONNABORTED (10053)` to
the client before it can read the 400 response.

### Documents pipeline

Verified end to end against the running server, not inferred:

| Step | Result |
| --- | --- |
| `POST /v1/documents` (raw body, `X-Filename`) | 200, `processing_state: "indexed"`, `page_count: 1` |
| `GET /v1/documents/{id}/pages` | page stored, `extraction_method: "native_text"`, content intact |
| `GET /v1/documents/search?q=` | returns the document with snippet and score |
| `DELETE /v1/documents/{id}` | 200; search then returns `[]` |

Note the upload route takes a **raw body** with the document's own
`Content-Type` plus an `X-Filename` header. It is not multipart — posting
`multipart/form-data` returns `400 unsupported document type`.

### Android build

`pnpm tauri android build` fails on this machine for two environment reasons,
neither of them M11 code:

1. The standalone `C:\Program Files\Rust stable MSVC 1.96` toolchain shadows
   rustup on PATH and has no `aarch64-linux-android` std. Prepend
   `~/.cargo/bin` to PATH.
2. Tauri symlinks the built `.so` into `jniLibs`, which needs Windows Developer
   Mode. Without it the build stops after compiling the library.

Working sequence used:

```sh
export PATH="$HOME/.cargo/bin:$PATH"
pnpm --dir apps/mobile build
cp target/aarch64-linux-android/debug/libassistant_mobile_lib.so \
   apps/mobile/src-tauri/gen/android/app/src/main/jniLibs/arm64-v8a/
cd apps/mobile/src-tauri/gen/android
./gradlew.bat assembleArm64Debug -x rustBuildArm64Debug
```

`BUILD SUCCESSFUL`. Enabling Developer Mode removes the copy and the `-x`.

### Physical device

Verified on an attached physical device (`adb` serial `1025f519`), debug APK
installed with `adb install -r`:

- App launches; no `AndroidRuntime` fatal and no JS exception in logcat.
- Notes renders the redesign; previously it fell through to a blank page.
- Created a note; it persisted and the folder count went 0 → 1, so the write
  reached the server.
- Swiped a note row: the tray revealed Pin/Archive/Delete and **did not**
  delete, which is the whole point of reveal-over-act.
- Tapped Delete: the note was removed and the count returned to 0 — the UI
  followed the server, not the gesture.

Not verified on device: TalkBack, offline behaviour, rotation, reduced-motion,
and haptics (inert, see above).

## What is not done

M11 is partial. Honestly scoped, the following remain:

- Pull-to-refresh on Gmail and Drive. Both hand-roll their own drawer drags
  (below) and should be done together with that cleanup.
- Drag-to-reorder.
- List insertion/removal animation.
- `GmailScreen` and `DriveScreen` still hand-roll drawer drags with raw
  `onTouchStart/Move/End` and a fixed 100 px threshold. They predate
  `SwipeableRow`, do no axis locking and ignore cancellation, and should move
  onto the shared gesture rules.
- Virtualization. Every list renders its whole array; this is fine at current
  data volumes but is the first thing to fail on a long list.

## Findings for M12

Not fixed here, because they are outside M11 scope:

- **Seven built screens are unreachable.** `App.tsx` renders 14 screens;
  `MoreSheet` now exposes 6. Reminders, Ideas, Drive, Academic, Memory,
  Planning and — as of this change, deliberately — Documents have no entry
  point, and `ProductivityScreen.tsx` is not wired into `App.tsx` at all. They
  ship in the bundle but no user can open them. Documents was removed from the
  drawer on request to keep it minimal, once the pipeline was verified working;
  it needs an entry point from somewhere real before it is usable again.
- **One orphaned route.** `POST /v1/planning/plan` has no caller in
  `apps/mobile/src/api/`.
- **Bundle size.** One 743 kB chunk with no code splitting. It is the app's
  cold-start cost on Android.
- **`tile memory limits exceeded`** appears in logcat on the home screen, from
  the animated SiriWave surface. Worth measuring before it becomes jank.
- **The Notes editor shows "Auto-saved" before anything has been typed or
  saved**, which claims a persistence event that did not occur.
- **Migrations are not part of any startup or deploy check.** The app gave no
  hint that the schema was three migrations behind; Documents simply 500'd and
  the screen rewrote that into a reassuring message. A readiness check that
  compares applied migrations against `migrations/` would have named the cause
  immediately.
