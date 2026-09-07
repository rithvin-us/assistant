/**
 * Standalone Productivity API Layer (Projects, Labels, Tasks, Reminders, Notes, Ideas).
 *
 * Deterministic HTTP REST calls to `assistant-server`. No model provider is
 * involved in any of it.
 *
 * Durability model (ADR-0029). `localStorage` is a cache of server state plus a
 * write outbox. It is never the system of record. A write made while the server
 * is unreachable is:
 *
 *   1. applied to the cache immediately, marked `pending: true`, so the UI shows
 *      it without waiting;
 *   2. appended to a durable outbox keyed by a client-generated UUID;
 *   3. replayed, in order, on the next reachable probe.
 *
 * The previous revision of this file wrote to `localStorage` and returned the
 * item as though it had been saved. Nothing replayed it. A task created while
 * the server was down existed only in that browser profile: it never reached
 * Postgres, so it did not survive reinstalling the app, clearing site data, or
 * opening the app on another device -- and the UI reported success either way.
 * Reporting a durability the system does not have is the same class of mistake
 * as a canned assistant reply that looks like inference.
 */

import { SERVER_BASE_URL, DEV_TOKEN, probeServer } from "./bridge";
import type {
  TaskItem,
  ReminderItem,
  NoteItem,
  IdeaItem,
  ProjectItem,
  LabelItem,
} from "./types";

const CACHE_TASKS = "assistant_cache_tasks";
const CACHE_REMINDERS = "assistant_cache_reminders";
const CACHE_NOTES = "assistant_cache_notes";
const CACHE_IDEAS = "assistant_cache_ideas";
const CACHE_PROJECTS = "assistant_cache_projects";
const CACHE_LABELS = "assistant_cache_labels";

const OUTBOX_KEY = "assistant_outbox";
const DEADLETTER_KEY = "assistant_outbox_dead";

/**
 * Keys used by the revision that treated local storage as a database. Read once
 * on first load so work captured under the old scheme is migrated into the
 * outbox and actually reaches Postgres, rather than being stranded by the fix.
 */
const LEGACY_KEYS: Record<EntityKind, string> = {
  task: "assistant_local_tasks",
  reminder: "assistant_local_reminders",
  note: "assistant_local_notes",
  idea: "assistant_local_ideas",
};

const CACHE_KEYS: Record<EntityKind, string> = {
  task: CACHE_TASKS,
  reminder: CACHE_REMINDERS,
  note: CACHE_NOTES,
  idea: CACHE_IDEAS,
};

const PATHS: Record<EntityKind, string> = {
  task: "/v1/tasks",
  reminder: "/v1/reminders",
  note: "/v1/notes",
  idea: "/v1/ideas",
};

type EntityKind = "task" | "reminder" | "note" | "idea";
type OpKind = "create" | "update" | "delete";

interface OutboxEntry {
  /** Entry id, distinct from the entity id: one entity has many entries. */
  seq: string;
  kind: EntityKind;
  op: OpKind;
  entityId: string;
  payload: Record<string, unknown>;
  queuedAt: string;
  attempts: number;
  lastError?: string;
}

interface DeadEntry extends OutboxEntry {
  failedAt: string;
  status: number;
}

/** Anything carrying an `id` and the client-only `pending` marker. */
type Storable = { id: string; pending?: boolean };

// ---------------------------------------------------------------------------
// storage
// ---------------------------------------------------------------------------

function readJson<T>(key: string, fallback: T): T {
  try {
    const raw = localStorage.getItem(key);
    return raw ? (JSON.parse(raw) as T) : fallback;
  } catch {
    return fallback;
  }
}

function writeJson(key: string, data: unknown): void {
  try {
    localStorage.setItem(key, JSON.stringify(data));
  } catch {
    // A full or disabled store must not fail the write path. The server copy is
    // the one that matters; losing the cache costs a refetch.
  }
}

function readCache<T>(kind: EntityKind): T[] {
  return readJson<T[]>(CACHE_KEYS[kind], []);
}

function writeCache<T>(kind: EntityKind, items: T[]): void {
  writeJson(CACHE_KEYS[kind], items);
}

function readOutbox(): OutboxEntry[] {
  return readJson<OutboxEntry[]>(OUTBOX_KEY, []);
}

function writeOutbox(entries: OutboxEntry[]): void {
  writeJson(OUTBOX_KEY, entries);
}

/** Entries the server rejected with a 4xx. Exposed so the UI can show them. */
export function deadLetters(): DeadEntry[] {
  return readJson<DeadEntry[]>(DEADLETTER_KEY, []);
}

export function clearDeadLetters(): void {
  writeJson(DEADLETTER_KEY, []);
}

/** Number of writes not yet acknowledged by the server. */
export function pendingCount(): number {
  return readOutbox().length;
}

function newId(): string {
  if (typeof crypto !== "undefined" && typeof crypto.randomUUID === "function") {
    return crypto.randomUUID();
  }
  // Deployment targets all provide `crypto.randomUUID`; this exists so a write
  // never throws on an older webview, and the value is still a v4-shaped UUID
  // the server will accept as a primary key.
  return "xxxxxxxx-xxxx-4xxx-yxxx-xxxxxxxxxxxx".replace(/[xy]/g, (c) => {
    const r = (Math.random() * 16) | 0;
    return (c === "x" ? r : (r & 0x3) | 0x8).toString(16);
  });
}

function authHeaders(json: boolean): Record<string, string> {
  return {
    ...(json ? { "Content-Type": "application/json" } : {}),
    ...(DEV_TOKEN ? { Authorization: `Bearer ${DEV_TOKEN}` } : {}),
  };
}

async function serverReachable(): Promise<boolean> {
  const probe = await probeServer();
  return probe.state === "reachable";
}

// ---------------------------------------------------------------------------
// outbox
// ---------------------------------------------------------------------------

/**
 * Queues a write and applies it to the cache immediately.
 *
 * The cached copy is marked `pending` so a screen can tell "saved" from "not
 * saved yet". `flushOutbox` clears the marker when the server answers.
 */
function enqueue(
  kind: EntityKind,
  op: OpKind,
  entityId: string,
  payload: Record<string, unknown>
): void {
  const entries = readOutbox();
  entries.push({
    seq: newId(),
    kind,
    op,
    entityId,
    payload,
    queuedAt: new Date().toISOString(),
    attempts: 0,
  });
  writeOutbox(entries);
}

/**
 * Replays queued writes in order, stopping at the first retryable failure.
 *
 * Order matters and is preserved: a create must reach the server before the
 * update that follows it. Stopping rather than skipping is what keeps that
 * true; a later entry is not independent of an earlier one for the same entity.
 */
export async function flushOutbox(): Promise<{ sent: number; remaining: number }> {
  let entries = readOutbox();
  if (entries.length === 0) return { sent: 0, remaining: 0 };
  if (!(await serverReachable())) return { sent: 0, remaining: entries.length };

  let sent = 0;

  while (entries.length > 0) {
    const entry = entries[0];
    let response: Response;

    try {
      response = await sendEntry(entry);
    } catch (error) {
      // Network-level failure: the server may or may not have applied it.
      // Retrying is safe because creates carry the client's id and the server
      // upserts on it, and updates are last-write-wins on the same row.
      entry.attempts += 1;
      entry.lastError = error instanceof Error ? error.message : "network error";
      writeOutbox(entries);
      break;
    }

    if (response.ok || (entry.op === "delete" && response.status === 404)) {
      // A delete of something the server no longer has is the outcome asked for.
      if (response.ok && entry.op !== "delete") {
        const item = (await response.json()) as Storable;
        applyServerItem(entry.kind, item);
      } else if (entry.op === "delete") {
        removeFromCache(entry.kind, entry.entityId);
      }
      entries.shift();
      writeOutbox(entries);
      sent += 1;
      continue;
    }

    // 408/429 are the server asking for a retry, not a verdict on the request.
    const retryable =
      response.status >= 500 || response.status === 408 || response.status === 429;

    if (retryable) {
      entry.attempts += 1;
      entry.lastError = `http ${response.status}`;
      writeOutbox(entries);
      break;
    }

    // A 4xx is a judgement the request is invalid. Sending it again will not
    // make it valid, so it is dead-lettered rather than retried forever --
    // otherwise one malformed write blocks every write behind it.
    entries = deadLetter(entries, entry, response.status);
    writeOutbox(entries);
  }

  return { sent, remaining: entries.length };
}

/**
 * Moves an entry to the dead-letter list, along with every queued entry for the
 * same entity.
 *
 * If a create was rejected the row does not exist, so the update and delete
 * behind it can only 404. Dropping them together keeps the queue honest instead
 * of producing a cascade of failures with one real cause.
 */
function deadLetter(
  entries: OutboxEntry[],
  failed: OutboxEntry,
  status: number
): OutboxEntry[] {
  const dead = deadLetters();
  const failedAt = new Date().toISOString();
  const casualties =
    failed.op === "create"
      ? entries.filter((e) => e.entityId === failed.entityId)
      : [failed];

  for (const entry of casualties) {
    dead.push({ ...entry, failedAt, status });
  }
  writeJson(DEADLETTER_KEY, dead);

  const dropped = new Set(casualties.map((e) => e.seq));
  const kept = entries.filter((e) => !dropped.has(e.seq));

  // The optimistic copy was never saved anywhere durable. Leaving it on screen
  // marked "pending" forever would be the lie this whole change removes.
  if (failed.op === "create") {
    removeFromCache(failed.kind, failed.entityId);
  }
  return kept;
}

function sendEntry(entry: OutboxEntry): Promise<Response> {
  const base = `${SERVER_BASE_URL}${PATHS[entry.kind]}`;
  switch (entry.op) {
    case "create":
      return fetch(base, {
        method: "POST",
        headers: authHeaders(true),
        body: JSON.stringify({ id: entry.entityId, ...entry.payload }),
      });
    case "update":
      return fetch(`${base}/${entry.entityId}`, {
        method: "PATCH",
        headers: authHeaders(true),
        body: JSON.stringify(entry.payload),
      });
    case "delete":
      return fetch(`${base}/${entry.entityId}`, {
        method: "DELETE",
        headers: authHeaders(false),
      });
  }
}

// ---------------------------------------------------------------------------
// cache maintenance
// ---------------------------------------------------------------------------

function upsertCache<T extends Storable>(kind: EntityKind, item: T): void {
  const items = readCache<T>(kind);
  const index = items.findIndex((i) => i.id === item.id);
  if (index >= 0) {
    items[index] = { ...items[index], ...item };
  } else {
    items.unshift(item);
  }
  writeCache(kind, items);
}

function applyServerItem(kind: EntityKind, item: Storable): void {
  const items = readCache<Storable>(kind);
  const index = items.findIndex((i) => i.id === item.id);
  const acknowledged = { ...item, pending: false };
  if (index >= 0) items[index] = acknowledged;
  else items.unshift(acknowledged);
  writeCache(kind, items);
}

function removeFromCache(kind: EntityKind, id: string): void {
  writeCache(
    kind,
    readCache<Storable>(kind).filter((i) => i.id !== id)
  );
}

/**
 * Overlays still-queued writes onto a freshly fetched server list.
 *
 * A list fetched mid-flush does not contain writes the server has not seen yet.
 * Without this the item a user created a second ago would disappear on the next
 * refresh and reappear once the queue drained.
 */
function overlayPending<T extends Storable>(kind: EntityKind, fromServer: T[]): T[] {
  const queued = readOutbox().filter((e) => e.kind === kind);
  if (queued.length === 0) return fromServer;

  const cached = readCache<T>(kind);
  const result = [...fromServer];

  for (const entry of queued) {
    const index = result.findIndex((i) => i.id === entry.entityId);
    if (entry.op === "delete") {
      if (index >= 0) result.splice(index, 1);
      continue;
    }
    const local = cached.find((i) => i.id === entry.entityId);
    if (!local) continue;
    const pendingCopy = { ...local, pending: true };
    if (index >= 0) result[index] = pendingCopy;
    else result.unshift(pendingCopy);
  }
  return result;
}

/**
 * Migrates items written by the pre-outbox revision into the queue, once.
 *
 * Those rows never reached the server. They are re-sent as creates carrying
 * their original id, so an item that somehow did reach Postgres is upserted
 * rather than duplicated.
 */
function adoptLegacyLocalItems(): void {
  const marker = "assistant_outbox_migrated_v1";
  if (readJson<boolean>(marker, false)) return;

  for (const kind of Object.keys(LEGACY_KEYS) as EntityKind[]) {
    const legacy = readJson<Record<string, unknown>[]>(LEGACY_KEYS[kind], []);
    for (const item of legacy) {
      const id = typeof item.id === "string" ? item.id : null;
      if (!id) continue;
      const payload = { ...item };
      // Server-owned fields; sending them back is meaningless at best.
      for (const field of ["id", "user_id", "created_at", "updated_at", "pending"]) {
        delete payload[field];
      }
      enqueue(kind, "create", id, payload);
      upsertCache(kind, { ...(item as Storable), pending: true });
    }
  }
  writeJson(marker, true);
}

adoptLegacyLocalItems();

/** Fetches a list, falling back to the cache when the server cannot be reached. */
async function fetchList<T extends Storable>(
  kind: EntityKind,
  query: URLSearchParams
): Promise<T[]> {
  await flushOutbox();

  if (await serverReachable()) {
    try {
      const res = await fetch(`${SERVER_BASE_URL}${PATHS[kind]}?${query.toString()}`, {
        headers: authHeaders(false),
      });
      if (res.ok) {
        const data = (await res.json()) as T[];
        // The cache holds the server's answer plus anything still queued, so a
        // later offline read shows the same list this one did.
        const merged = overlayPending(kind, data);
        writeCache(kind, merged);
        return merged;
      }
    } catch {
      // fall through to the cache
    }
  }
  return readCache<T>(kind);
}

/**
 * Sends a write immediately when the server is up, and queues it when not.
 *
 * The queue path is the only path that returns an unsaved item, and it marks it
 * `pending` so the caller cannot mistake it for a durable one.
 */
async function write<T extends Storable>(
  kind: EntityKind,
  op: OpKind,
  entityId: string,
  payload: Record<string, unknown>,
  optimistic: T
): Promise<T> {
  enqueue(kind, op, entityId, payload);
  upsertCache(kind, { ...optimistic, pending: true });

  await flushOutbox();

  const settled = readCache<T>(kind).find((i) => i.id === entityId);
  if (settled) return settled;

  // Only reachable for a delete, which leaves nothing behind.
  return { ...optimistic, pending: true };
}

// ==================== PROJECTS ====================

export async function fetchProjects(): Promise<ProjectItem[]> {
  if (await serverReachable()) {
    try {
      const res = await fetch(`${SERVER_BASE_URL}/v1/projects`, {
        headers: authHeaders(false),
      });
      if (res.ok) {
        const data = (await res.json()) as ProjectItem[];
        writeJson(CACHE_PROJECTS, data);
        return data;
      }
    } catch {
      // fall through
    }
  }
  return readJson<ProjectItem[]>(CACHE_PROJECTS, []);
}

/**
 * Projects and labels are not queued.
 *
 * They are containers, not captured work: a task can be created offline and
 * filed later, but inventing a project id offline and then reconciling it
 * against one the server may already have under the same name is a merge
 * problem with no obviously right answer. Failing loudly is better than
 * guessing.
 */
export async function createProject(input: {
  name: string;
  color?: string;
  position?: number;
}): Promise<ProjectItem> {
  const res = await fetch(`${SERVER_BASE_URL}/v1/projects`, {
    method: "POST",
    headers: authHeaders(true),
    body: JSON.stringify(input),
  });
  if (!res.ok) throw new Error(`could not create project (http ${res.status})`);
  return (await res.json()) as ProjectItem;
}

export async function updateProject(
  id: string,
  input: { name?: string; color?: string; position?: number }
): Promise<ProjectItem> {
  const res = await fetch(`${SERVER_BASE_URL}/v1/projects/${id}`, {
    method: "PATCH",
    headers: authHeaders(true),
    body: JSON.stringify(input),
  });
  if (!res.ok) throw new Error(`could not update project (http ${res.status})`);
  return (await res.json()) as ProjectItem;
}

/** The server reassigns the project's tasks to Inbox before deleting it. */
export async function deleteProject(id: string): Promise<void> {
  const res = await fetch(`${SERVER_BASE_URL}/v1/projects/${id}`, {
    method: "DELETE",
    headers: authHeaders(false),
  });
  if (!res.ok) throw new Error(`could not delete project (http ${res.status})`);
}

// ==================== LABELS ====================

export async function fetchLabels(): Promise<LabelItem[]> {
  if (await serverReachable()) {
    try {
      const res = await fetch(`${SERVER_BASE_URL}/v1/labels`, {
        headers: authHeaders(false),
      });
      if (res.ok) {
        const data = (await res.json()) as LabelItem[];
        writeJson(CACHE_LABELS, data);
        return data;
      }
    } catch {
      // fall through
    }
  }
  return readJson<LabelItem[]>(CACHE_LABELS, []);
}

export async function createLabel(input: {
  name: string;
  color?: string;
}): Promise<LabelItem> {
  const res = await fetch(`${SERVER_BASE_URL}/v1/labels`, {
    method: "POST",
    headers: authHeaders(true),
    body: JSON.stringify(input),
  });
  if (!res.ok) throw new Error(`could not create label (http ${res.status})`);
  return (await res.json()) as LabelItem;
}

/** One UPDATE renames it everywhere it is used -- the point of ADR-0028. */
export async function updateLabel(
  id: string,
  input: { name?: string; color?: string }
): Promise<LabelItem> {
  const res = await fetch(`${SERVER_BASE_URL}/v1/labels/${id}`, {
    method: "PATCH",
    headers: authHeaders(true),
    body: JSON.stringify(input),
  });
  if (!res.ok) throw new Error(`could not update label (http ${res.status})`);
  return (await res.json()) as LabelItem;
}

export async function deleteLabel(id: string): Promise<void> {
  const res = await fetch(`${SERVER_BASE_URL}/v1/labels/${id}`, {
    method: "DELETE",
    headers: authHeaders(false),
  });
  if (!res.ok) throw new Error(`could not delete label (http ${res.status})`);
}

// ==================== TASKS ====================

export async function fetchTasks(filter?: {
  status?: string;
  priority?: string;
  project?: string;
  project_id?: string;
  label?: string;
  q?: string;
}): Promise<TaskItem[]> {
  const query = new URLSearchParams();
  if (filter?.status) query.set("status", filter.status);
  if (filter?.priority) query.set("priority", filter.priority);
  if (filter?.project) query.set("project", filter.project);
  if (filter?.project_id) query.set("project_id", filter.project_id);
  if (filter?.label) query.set("label", filter.label);
  if (filter?.q) query.set("q", filter.q);

  const items = await fetchList<TaskItem>("task", query);

  // The server applies these too. Repeating them locally is what makes the
  // offline list match the online one instead of ignoring the active filter.
  let result = items;
  if (filter?.status) result = result.filter((t) => t.status === filter.status);
  if (filter?.priority) result = result.filter((t) => t.priority === filter.priority);
  if (filter?.project) {
    result = result.filter(
      (t) => t.project?.toLowerCase() === filter.project?.toLowerCase()
    );
  }
  if (filter?.project_id) result = result.filter((t) => t.project_id === filter.project_id);
  if (filter?.label) {
    const label = filter.label.toLowerCase();
    result = result.filter((t) => (t.labels ?? []).some((l) => l.toLowerCase() === label));
  }
  if (filter?.q) {
    const q = filter.q.toLowerCase();
    result = result.filter(
      (t) =>
        t.title.toLowerCase().includes(q) || t.description.toLowerCase().includes(q)
    );
  }
  return result;
}

export async function createTask(input: {
  title: string;
  description?: string;
  priority?: string;
  due_at?: string;
  project?: string;
  project_id?: string;
  labels?: string[];
  estimated_minutes?: number;
}): Promise<TaskItem> {
  const id = newId();
  const now = new Date().toISOString();

  // A placeholder only until the server answers. `project_id` is empty because
  // the server owns project resolution -- the optimistic copy shows the name
  // the user typed and is replaced wholesale on acknowledgement.
  const optimistic: TaskItem = {
    id,
    user_id: "",
    title: input.title,
    description: input.description ?? "",
    priority: input.priority ?? "P4",
    status: "todo",
    due_at: input.due_at ?? null,
    project_id: input.project_id ?? "",
    project: input.project ?? "Inbox",
    labels: input.labels ?? [],
    estimated_minutes: input.estimated_minutes ?? null,
    created_at: now,
    updated_at: now,
    completed_at: null,
  };

  return write<TaskItem>("task", "create", id, { ...input }, optimistic);
}

export async function updateTask(
  id: string,
  input: {
    title?: string;
    description?: string;
    priority?: string;
    status?: string;
    due_at?: string | null;
    project?: string;
    project_id?: string;
    labels?: string[];
    estimated_minutes?: number | null;
  }
): Promise<TaskItem> {
  const current = readCache<TaskItem>("task").find((t) => t.id === id);
  if (!current) throw new Error("Task not found");

  const now = new Date().toISOString();
  const status = input.status ?? current.status;
  const optimistic: TaskItem = {
    ...current,
    ...input,
    due_at: input.due_at !== undefined ? input.due_at : current.due_at,
    project: input.project ?? current.project,
    labels: input.labels ?? current.labels,
    status,
    updated_at: now,
    completed_at:
      status === "completed"
        ? (current.completed_at ?? now)
        : status === "todo"
          ? null
          : current.completed_at,
  };

  return write<TaskItem>("task", "update", id, { ...input }, optimistic);
}

export async function deleteTask(id: string): Promise<void> {
  enqueue("task", "delete", id, {});
  removeFromCache("task", id);
  await flushOutbox();
}

// ==================== REMINDERS ====================

export async function fetchReminders(filter?: {
  status?: string;
  q?: string;
}): Promise<ReminderItem[]> {
  const query = new URLSearchParams();
  if (filter?.status) query.set("status", filter.status);
  if (filter?.q) query.set("q", filter.q);

  let items = await fetchList<ReminderItem>("reminder", query);
  if (filter?.status) items = items.filter((r) => r.status === filter.status);
  if (filter?.q) {
    const q = filter.q.toLowerCase();
    items = items.filter((r) => r.title.toLowerCase().includes(q));
  }
  return items;
}

export async function createReminder(input: {
  title: string;
  remind_at: string;
  task_id?: string;
}): Promise<ReminderItem> {
  const id = newId();
  const now = new Date().toISOString();
  const optimistic: ReminderItem = {
    id,
    user_id: "",
    task_id: input.task_id ?? null,
    title: input.title,
    remind_at: input.remind_at,
    status: "pending",
    created_at: now,
    updated_at: now,
  };
  return write<ReminderItem>("reminder", "create", id, { ...input }, optimistic);
}

export async function updateReminder(
  id: string,
  input: {
    title?: string;
    remind_at?: string;
    status?: string;
    task_id?: string | null;
  }
): Promise<ReminderItem> {
  const current = readCache<ReminderItem>("reminder").find((r) => r.id === id);
  if (!current) throw new Error("Reminder not found");

  const optimistic: ReminderItem = {
    ...current,
    ...input,
    task_id: input.task_id !== undefined ? input.task_id : current.task_id,
    updated_at: new Date().toISOString(),
  };
  return write<ReminderItem>("reminder", "update", id, { ...input }, optimistic);
}

export async function deleteReminder(id: string): Promise<void> {
  enqueue("reminder", "delete", id, {});
  removeFromCache("reminder", id);
  await flushOutbox();
}

// ==================== NOTES ====================

export async function fetchNotes(filter?: {
  is_archived?: boolean;
  tag?: string;
  q?: string;
}): Promise<NoteItem[]> {
  const query = new URLSearchParams();
  if (filter?.is_archived !== undefined) {
    query.set("is_archived", String(filter.is_archived));
  }
  if (filter?.tag) query.set("tag", filter.tag);
  if (filter?.q) query.set("q", filter.q);

  let items = await fetchList<NoteItem>("note", query);
  if (filter?.is_archived !== undefined) {
    items = items.filter((n) => n.is_archived === filter.is_archived);
  }
  if (filter?.tag) {
    const tag = filter.tag.toLowerCase();
    items = items.filter((n) => (n.tags ?? []).some((t) => t.toLowerCase() === tag));
  }
  if (filter?.q) {
    const q = filter.q.toLowerCase();
    items = items.filter(
      (n) => n.title.toLowerCase().includes(q) || n.content.toLowerCase().includes(q)
    );
  }
  return items;
}

export async function createNote(input: {
  title: string;
  content?: string;
  tags?: string[];
}): Promise<NoteItem> {
  const id = newId();
  const now = new Date().toISOString();
  const optimistic: NoteItem = {
    id,
    user_id: "",
    title: input.title,
    content: input.content ?? "",
    is_archived: false,
    tags: input.tags ?? [],
    created_at: now,
    updated_at: now,
  };
  return write<NoteItem>("note", "create", id, { ...input }, optimistic);
}

export async function updateNote(
  id: string,
  input: {
    title?: string;
    content?: string;
    is_archived?: boolean;
    is_pinned?: boolean;
    tags?: string[];
  }
): Promise<NoteItem> {
  const current = readCache<NoteItem>("note").find((n) => n.id === id);
  if (!current) throw new Error("Note not found");

  const optimistic: NoteItem = {
    ...current,
    ...input,
    tags: input.tags ?? current.tags,
    updated_at: new Date().toISOString(),
  };
  return write<NoteItem>("note", "update", id, { ...input }, optimistic);
}

export async function deleteNote(id: string): Promise<void> {
  enqueue("note", "delete", id, {});
  removeFromCache("note", id);
  await flushOutbox();
}

// ==================== IDEAS ====================

export async function fetchIdeas(filter?: {
  status?: string;
  q?: string;
}): Promise<IdeaItem[]> {
  const query = new URLSearchParams();
  if (filter?.status) query.set("status", filter.status);
  if (filter?.q) query.set("q", filter.q);

  let items = await fetchList<IdeaItem>("idea", query);
  if (filter?.status) items = items.filter((i) => i.status === filter.status);
  if (filter?.q) {
    const q = filter.q.toLowerCase();
    items = items.filter(
      (i) =>
        i.title.toLowerCase().includes(q) || i.description.toLowerCase().includes(q)
    );
  }
  return items;
}

export async function createIdea(input: {
  title: string;
  description?: string;
}): Promise<IdeaItem> {
  const id = newId();
  const now = new Date().toISOString();
  const optimistic: IdeaItem = {
    id,
    user_id: "",
    title: input.title,
    description: input.description ?? "",
    status: "active",
    converted_task_id: null,
    created_at: now,
    updated_at: now,
  };
  return write<IdeaItem>("idea", "create", id, { ...input }, optimistic);
}

export async function updateIdea(
  id: string,
  input: { title?: string; description?: string; status?: string }
): Promise<IdeaItem> {
  const current = readCache<IdeaItem>("idea").find((i) => i.id === id);
  if (!current) throw new Error("Idea not found");

  const optimistic: IdeaItem = {
    ...current,
    ...input,
    updated_at: new Date().toISOString(),
  };
  return write<IdeaItem>("idea", "update", id, { ...input }, optimistic);
}

/**
 * Converting requires the server.
 *
 * The endpoint holds the idea row `for update` so a double tap produces one
 * task; reproducing that offline would mean inventing a task id and hoping the
 * server agrees later. Pending writes are flushed first so the idea being
 * converted is one the server actually has.
 */
export async function convertIdeaToTask(
  id: string
): Promise<{ idea: IdeaItem; task: TaskItem }> {
  await flushOutbox();

  const res = await fetch(`${SERVER_BASE_URL}/v1/ideas/${id}/convert`, {
    method: "POST",
    headers: authHeaders(false),
  });
  if (!res.ok) {
    throw new Error(
      `could not convert idea to task (http ${res.status}). It stays an idea until the server is reachable.`
    );
  }

  const data = (await res.json()) as { idea: IdeaItem; task: TaskItem };
  applyServerItem("idea", data.idea);
  applyServerItem("task", data.task);
  return data;
}

export async function deleteIdea(id: string): Promise<void> {
  enqueue("idea", "delete", id, {});
  removeFromCache("idea", id);
  await flushOutbox();
}
