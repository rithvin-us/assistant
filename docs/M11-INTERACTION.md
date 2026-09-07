# M11 — Native Android interaction and motion

Status: **partial**. Primitives are built, tested and verified on hardware;
rollout across screens is incomplete. See "What is not done" before relying on
this document as a description of the whole app.

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

`cargo test` fails in two suites, `documents` (9/18) and `memory` (15/21), all
with `relation "..." does not exist`. This is unapplied migrations in the local
Postgres, confirmed pre-existing by re-running against a stashed tree and
getting identical counts. Run `scripts/migrate.ps1`. Not an M11 regression, and
it means the documents pipeline is **unverified**, not "working".

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

- Swipe actions and pull-to-refresh on Reminders, Ideas, Memory, Documents,
  Classroom, Calendar, Gmail, Drive, Academic, Planning, Connections.
- Optimistic writes still needing the same rollback treatment:
  `RemindersScreen.tsx:123`, `IdeasScreen.tsx:132`.
- Navigation transitions between screens (`App.tsx` swaps screens with no
  motion).
- Drag-to-reorder.
- List insertion/removal animation.
- Bottom-sheet drag-to-dismiss audit (`GmailScreen` and `DriveScreen` hand-roll
  touch drags that predate `SwipeableRow` and should move onto shared logic).
- Virtualization. Every list renders its whole array; this is fine at current
  data volumes but is the first thing to fail on a long list.

## Findings for M12

Not fixed here, because they are outside M11 scope:

- **Six built screens are unreachable.** `App.tsx` renders 14 screens;
  `MoreSheet` exposes 7. Reminders, Ideas, Drive, Academic, Memory and Planning
  have no entry point, and `ProductivityScreen.tsx` is not wired into `App.tsx`
  at all. They ship in the bundle but no user can open them.
- **One orphaned route.** `POST /v1/planning/plan` has no caller in
  `apps/mobile/src/api/`.
- **Bundle size.** One 743 kB chunk with no code splitting. It is the app's
  cold-start cost on Android.
- **`tile memory limits exceeded`** appears in logcat on the home screen, from
  the animated SiriWave surface. Worth measuring before it becomes jank.
- **The Notes editor shows "Auto-saved" before anything has been typed or
  saved**, which claims a persistence event that did not occur.
