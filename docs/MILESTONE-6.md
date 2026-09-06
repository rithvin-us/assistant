# Milestone 6 — Academic Intelligence

Google Classroom and Google Drive join Gmail and Calendar, and coursework
becomes deadlines the rest of the application already understands.

The organising idea is that there is **one implementation of each capability**.
The Classroom screen, the Academic Overview and the `classroom.*` tools all
call the same functions. A model asking "what is due this week" runs the code a
person runs by opening a screen, so the manual path and the AI path cannot
drift apart — and every one of these features works with no model configured at
all.

---

## 1. What was built

| Area | Capability |
|---|---|
| Classroom | List courses, coursework and announcements for a connected account |
| Drive | Search, browse folders, read file metadata, read small text files |
| Academic context | Unified deadlines, counts, and the Academic Overview |
| Sync | Import coursework into `tasks`, idempotently, with provenance |
| Mobile | Classroom, Drive and Academic screens, plus account switching |
| Tools | Ten deterministic tools over the same providers |

Deliberately **not** built: assignment submission, grading, roster or teacher
operations, file delete/move/rename/share, PDF text extraction, OCR, semantic
deduplication, embeddings, and any background polling. See §7.

---

## 2. Architecture

```
apps/mobile  ClassroomScreen · DriveScreen · AcademicScreen
     │                    (no Google types, no raw JSON)
     ▼
assistant-server/routes/academic.rs        transport only
     ▼
assistant-server/academic.rs               cache · sync · overview
     ▼
ClassroomProvider · DriveProvider · AcademicProvider   (traits, assistant-tools)
     ▼
google/classroom.rs · google/drive.rs      the only files that know Google JSON
     ▼
Google Classroom API v1 · Google Drive API v3
```

`assistant-core` gains no Google type, in line with ADR-0003. The provider
traits live in `assistant-tools`; the HTTP implementations live in
`assistant-server` next to the credential store; the normalised types
(`Course`, `CourseworkItem`, `Announcement`, `DriveFile`, `AcademicDeadline`)
live in `assistant-protocol` and are mirrored in
`apps/mobile/src/api/types.ts`. `PROTOCOL_VERSION` is 5.

`AcademicProvider` is declared in `assistant-tools` but implemented in
`assistant-server`, because it needs the pool as well as Google. That keeps
`assistant-tools` free of `sqlx`.

---

## 3. OAuth scopes

Added in this milestone:

| Scope | Purpose |
|---|---|
| `classroom.courses.readonly` | List enrolled courses |
| `classroom.coursework.me.readonly` | List the signed-in student's assignments |
| `classroom.announcements.readonly` | List course announcements |
| `drive.readonly` | Search Drive and read file content |

All read-only, and all Classroom scopes are the `.me` (student) variant. The
`.students` variants, which read other students' work, are not requested.

### Limitations you will actually hit

1. **Consent is not incremental.** An account connected before this milestone
   granted only the Gmail and Calendar scopes. Classroom and Drive calls
   against it fail with 403 until it is reconnected. The account picker detects
   this from the stored scope list and offers "Reconnect" rather than showing
   an unexplained error. Reconnecting does not create a duplicate account.

2. **`drive.readonly` is a Google *restricted* scope.** A published app using
   it must pass OAuth verification and a security assessment. Until then the
   project must stay in Google's testing mode, where the scope works for
   explicitly listed test users and everyone else sees an unverified-app
   warning. `drive.file` would avoid this but only grants access to files
   picked one at a time through Google's picker, which cannot implement search.

3. **Teacher-owned courses do not appear.** `courses.list` is called with
   `studentId=me`. A course where the user is the teacher is not returned, by
   design — see ADR-0032.

4. **A school or work domain can block this entirely.** A Workspace for
   Education administrator can disallow unverified third-party apps, and
   Classroom then returns 403 for that account no matter what was granted. The
   error message says an administrator may need to allow it.

---

## 4. Due dates

Classroom sends `dueDate` (a calendar date) and `dueTime` (a time of day,
documented as UTC) as separate optional objects that must appear together.
Normalisation is strict:

| Classroom sends | Stored `due_at` |
|---|---|
| Complete date + time | That instant, in UTC |
| Complete date, no time | 23:59:59 UTC that day |
| Partial date (no day, say) | `NULL` |
| Nothing | `NULL` |

`NULL` means **no deadline**, never "unknown". A partial date is refused rather
than completed, because a guessed deadline would be shown to the user and fed
to the scheduler. A date with no time defaults to end of day rather than
midnight, which would otherwise move the deadline a day earlier.

---

## 5. Sync and task provenance

Coursework becomes a row in `tasks`, not a parallel entity, so one assignment
is one obligation everywhere in the app.

- **Identity** is `(user_id, external_provider, external_id)`, enforced by a
  partial unique index. Repeated syncs update; they never duplicate.
- **`source`** (`manual` / `google_classroom` / …) is what the UI uses to show
  the "Classroom" chip on imported work.
- **`source_title` / `source_due_at`** record what Classroom last sent. A field
  that still matches is provider-owned and gets updated; a field that differs
  has been edited by the user and is left alone. Title and due date are decided
  independently.
- **Deleting is not a sync operation.** Coursework that vanishes from Classroom
  leaves its task untouched.
- **Disconnecting an account** stops future requests and deletes nothing.
  `external_account_id` is `ON DELETE SET NULL`.

Sync is an explicit refresh, never a timer. `academic_sync_state` records when
each resource last synced, which is what the UI shows instead of implying a
live read.

---

## 6. Drive file policy

| Case | Behaviour |
|---|---|
| `text/*`, JSON, XML under 512 KiB | Read and returned |
| Google Doc / Sheet | Exported as `text/plain` / `text/csv` |
| Over 512 KiB | Refused, naming the file and the limit |
| No reported size | Refused — an unknown length is what a limit is for |
| PDF, images, video, archives | Metadata only; refused as text |
| Longer than 40,000 characters | Cut, with `truncated: true` |

Metadata is fetched **before** any content request, and type is checked before
size, so an unsupported file is rejected without a byte being downloaded.

Nothing from Drive is stored in Postgres. Metadata is cached on the device.

---

## 7. Boundaries

Not implemented here, and where each belongs:

- **PDF understanding, OCR, page extraction** — document-intelligence
  milestone. PDFs are discoverable now; they are not read.
- **Semantic deduplication** across Gmail / Classroom / manual entries.
  Identity here is exact: provider + external id. Matching "Compiler
  Assignment due Friday" in an email to a Classroom assignment needs semantic
  comparison and belongs to a later milestone. Guessing would silently merge
  two different obligations.
- **Embeddings, pgvector, long-term memory, proactive notifications, voice,
  browser or PC automation** — later milestones.
- **Schedule optimisation.** The free-time engine already accepts academic
  tasks, but nothing here plans a timetable automatically.

---

## 8. Database

Migration `0008_academic_intelligence.sql`:

- `tasks` gains `source`, `external_provider`, `external_id`,
  `external_account_id`, `source_title`, `source_due_at`, `source_synced_at`,
  a partial unique index on the external identity, and an index on
  `(user_id, source)`.
- `classroom_courses`, `classroom_coursework`, `classroom_announcements` —
  cached for offline use, unique per `(user_id, account_id, external_id)`.
- `academic_sync_state` — one row per account and resource.
- RLS enabled on every new table, with no policies, matching migrations 0005
  and 0007.

No Drive table exists, by design.

Apply with `scripts/migrate.ps1`. Migrations are never applied at startup
(ADR-0006).

---

## 9. Tools

| Tool | Risk | Notes |
|---|---|---|
| `classroom.courses` | Green | Read-only |
| `classroom.coursework` | Green | Read-only |
| `classroom.announcements` | Green | Read-only |
| `drive.search` | Green | Metadata only |
| `drive.list` | Green | Metadata only |
| `drive.get_metadata` | Green | Metadata only |
| `drive.read_small_file` | Green | Refuses oversized/unsupported |
| `academic.deadlines` | Green | Reads cache; no external call |
| `academic.assignments` | Green | Reads cache; no external call |
| `academic.sync` | **Yellow** | Writes tasks; reversible; cannot delete |

Risk is a static property of the `ToolSpec` in Rust and is evaluated by
`PermissionPolicy`. Model output cannot reach it (ADR-0005). Every tool takes
an explicit `account_id`; the authenticated user is injected by the executor as
`_user_id` and overwrites anything a model supplies under that key.

---

## 10. Verification

Automated, in this repository:

- 237 tests pass (`cargo test --workspace`), of which 42 are new.
- `cargo fmt --all --check` and
  `cargo clippy --workspace --all-targets -- -D warnings` are clean.
- `pnpm typecheck` and `pnpm build` succeed; `pnpm lint` reports nothing in any
  file this milestone touched.

New tests cover: due-date normalisation (complete, missing, partial,
date-only), material metadata extraction, Drive MIME and size gating, Drive
query escaping, the source-versus-user update decision, overview arithmetic,
account isolation across every tool, disconnected accounts, and sync
idempotency including a moved deadline and removed coursework.

Real Google and physical-device verification are recorded separately in
`docs/MILESTONE-6-VERIFICATION.md`; automated tests alone do not make this
milestone green.
