# Milestone 6 — Verification

Three kinds of evidence, kept apart on purpose. Automated tests passing is not
the same as Google actually answering, and neither is the same as the feature
working on a phone. Anything not verified is listed as not verified.

Date: 2026-09-07. Baseline commit: `5c4ebff`.

(Lint is clean across the whole app as of `a1a2cb7`, which fixed three
pre-existing `react-hooks/set-state-in-effect` errors in the Milestone 5
screens separately from this work.)

---

## 1. Automated verification — PASSED

| Check | Result |
|---|---|
| `cargo fmt --all --check` | clean |
| `cargo clippy --workspace --all-targets -- -D warnings` | clean, exit 0 |
| `cargo test --workspace` | **237 passed, 0 failed** |
| `pnpm --dir apps/mobile typecheck` | clean |
| `pnpm --dir apps/mobile build` | succeeds |
| `pnpm --dir apps/mobile lint` | clean, no findings |

42 tests are new. What they actually prove:

**Due-date normalisation** (`google/classroom.rs`) — a complete `dueDate` +
`dueTime` becomes one UTC instant; a missing one becomes `None`; a *partial*
date becomes `None` rather than being completed; a date with no time lands at
23:59:59 and does not roll back a day. Material metadata is extracted as title
and link only.

**Drive gating** (`google/drive.rs`) — size parsed from Drive's string form;
absent size stays absent rather than becoming zero; folders detected by MIME;
PDFs excluded from text reading but not from discovery; Google editor formats
map to export types; query literals escaped so an apostrophe in a file name
cannot alter the query.

**Sync policy** (`academic.rs`) — unchanged coursework writes nothing; a moved
deadline updates; a cleared deadline is a real write; a user-edited title is
not overwritten; a user-edited title does *not* freeze the due date; a row with
no recorded provenance is treated as user-owned; overview arithmetic counts
overdue and the next seven days and ignores completed items.

**Security and sync behaviour** (`tests/academic_security.rs`, 18 tests) —
every tool refuses an account owned by another user with `NotFound`, so
"not yours" is indistinguishable from "does not exist"; a client-supplied
`account_id` does not bypass ownership; a model-supplied `_user_id` cannot
override the executor's; disconnected accounts stop requests; oversized,
unknown-size and unsupported files are refused with a reason; a second sync
creates no duplicate; a moved deadline updates the same task; coursework
removed from Classroom leaves the task alone; two courses may hold assignments
with the same title.

**Not covered by automated tests:** the Postgres upsert paths in `academic.rs`
(`upsert_course`, `upsert_coursework`, `upsert_announcement`, `sync_one_task`,
`overview`) have no integration test, because the suite has no database
fixture. Their *decision* logic is tested through `plan_task_update` and a
fake store that models the unique index, but the SQL itself is currently only
exercised by running the server. This is the weakest point in the milestone's
test coverage and is stated rather than glossed over.

---

## 2. Real Google verification — BLOCKED, not attempted

**Status: cannot be completed from this repository.** The deployed server is
live and the new endpoints answer, but no Classroom or Drive data has been
retrieved from Google.

Confirmed working against the deployed server
(`https://assistant-server-vbrv.onrender.com`):

```
200  /v1/academic/overview      -> {"course_count":0,...,"oldest_synced_at":null}
200  /v1/classroom/courses      (cached read)
```

Confirmed blocked, with the intended message and no Google error body leaked:

```
400  /v1/classroom/courses?...&refresh=true
     "This Google account does not have access to Classroom. If it is a school
      or work account, an administrator may need to allow it, or the account may
      need to be reconnected to grant the newer permissions."

400  /v1/drive/search?...
     "This Google account does not have access to Drive. ..."
```

This is the correct behaviour for the current state, not a defect. All three
connected accounts hold only the Milestone 5 scopes:

```
openid, userinfo.email, userinfo.profile, calendar.events, gmail.readonly
```

### What has to happen before this section can pass

These steps need the Google Cloud console and an interactive browser consent on
the account owner's part. They cannot be done from here.

1. Enable **Google Classroom API** and **Google Drive API** on the project.
2. Add to the OAuth consent screen: `classroom.courses.readonly`,
   `classroom.coursework.me.readonly`, `classroom.announcements.readonly`,
   `drive.readonly`.
3. Because `drive.readonly` is a **restricted** scope, keep the consent screen
   in **Testing** and add each Google account as a **test user**.
4. Reconnect each account in the app (Connections → Reconnect). This updates
   the existing row; it does not create a duplicate.
5. Then verify: courses list, coursework with real due dates, announcements,
   Drive search, a small file read, an oversized file refusal, and a sync
   followed by a second sync producing no duplicate task.

`24z234@psgitech.ac.in` is a Workspace for Education account. Its domain
administrator may block unverified third-party apps regardless of the above, in
which case Classroom returns 403 for that account specifically.

---

## 3. Physical Android verification — PARTIAL

Device: OnePlus PJF110 (`1025f519`), arm64, debug APK built and installed via
`adb install -r`. Verified by screenshot and UI-hierarchy dump, not by
browser preview.

| Check | Result |
|---|---|
| APK builds | Yes — `BUILD SUCCESSFUL`, 225 MB debug APK |
| Installs and launches on device | Yes |
| Academic, Classroom and Drive appear in navigation | Yes |
| Academic Overview renders | Yes — `0 due this week / 0 overdue / 0 courses`, "Not synced yet", "Nothing outstanding. Sync Classroom to import coursework." |
| Classroom screen renders | Yes — three account chips, scope warning, Reconnect action |
| Drive screen renders | Yes — same, with the Drive-specific wording |
| Multi-account switching | Yes — tapping the second chip switched to `24z234@psgitech.ac.in` and both the warning text and the display name updated |
| Stale-data labelling | Yes — "Not synced yet" shown rather than implying a live read |
| Courses, coursework, announcements listing on device | **Not verified** — blocked on §2 |
| Drive search and file read on device | **Not verified** — blocked on §2 |
| Coursework importing into Tasks on device | **Not verified** — blocked on §2 |

---

## 4. Database

Migration `0008_academic_intelligence` applied to the live Supabase database:

```
8/installed academic intelligence
```

Additive only: new tables, new nullable columns on `tasks`, new indexes, RLS
enabled on each new table. No destructive statement.

---

## 5. Honest status

**Milestone 6 is not GREEN.**

Green requires Classroom listing courses, coursework and announcements from a
real account, Drive search returning real files, and imported coursework
becoming tasks. None of that has been observed, because no connected account
has granted the scopes yet.

What is done: the implementation, the schema, the tools, the screens, the
documentation and the automated tests, all verified. What remains is the
Google Cloud console configuration and one reconnect per account — after which
§2 and the blocked rows of §3 can be attempted and this file updated.
