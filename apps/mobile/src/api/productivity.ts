/**
 * Standalone Productivity API Layer (Tasks, Reminders, Notes, Ideas).
 *
 * Implements deterministic HTTP REST calls to the backend server with local storage
 * offline fallback. Works 100% without any AI API key or network dependency.
 */

import { SERVER_BASE_URL, DEV_TOKEN, probeServer } from "./bridge";
import type { TaskItem, ReminderItem, NoteItem, IdeaItem } from "./types";

const LOCAL_TASKS_KEY = "assistant_local_tasks";
const LOCAL_REMINDERS_KEY = "assistant_local_reminders";
const LOCAL_NOTES_KEY = "assistant_local_notes";
const LOCAL_IDEAS_KEY = "assistant_local_ideas";

function getLocal<T>(key: string): T[] {
  try {
    const raw = localStorage.getItem(key);
    return raw ? JSON.parse(raw) : [];
  } catch {
    return [];
  }
}

function setLocal<T>(key: string, data: T[]): void {
  try {
    localStorage.setItem(key, JSON.stringify(data));
  } catch {
    // ignore
  }
}

async function isServerAvailable(): Promise<boolean> {
  const probe = await probeServer();
  return probe.state === "reachable";
}

const DEV_USER_ID = "deadbeef-0000-4000-8000-000000000001";

// ==================== TASKS ====================

export async function fetchTasks(filter?: {
  status?: string;
  priority?: string;
  project?: string;
  q?: string;
}): Promise<TaskItem[]> {
  if (await isServerAvailable()) {
    try {
      const query = new URLSearchParams();
      if (filter?.status) query.set("status", filter.status);
      if (filter?.priority) query.set("priority", filter.priority);
      if (filter?.project) query.set("project", filter.project);
      if (filter?.q) query.set("q", filter.q);

      const url = `${SERVER_BASE_URL}/v1/tasks?${query.toString()}`;
      const res = await fetch(url, {
        headers: DEV_TOKEN ? { Authorization: `Bearer ${DEV_TOKEN}` } : {},
      });
      if (res.ok) {
        const data = await res.json();
        setLocal(LOCAL_TASKS_KEY, data);
        return data;
      }
    } catch {
      // fallback
    }
  }

  let items = getLocal<TaskItem>(LOCAL_TASKS_KEY);
  if (filter?.status) items = items.filter((t) => t.status === filter.status);
  if (filter?.priority) items = items.filter((t) => t.priority === filter.priority);
  if (filter?.project) items = items.filter((t) => t.project.toLowerCase() === filter.project?.toLowerCase());
  if (filter?.q) {
    const q = filter.q.toLowerCase();
    items = items.filter((t) => t.title.toLowerCase().includes(q) || t.description.toLowerCase().includes(q));
  }
  return items;
}

export async function createTask(input: {
  title: string;
  description?: string;
  priority?: string;
  due_at?: string;
  project?: string;
}): Promise<TaskItem> {
  if (await isServerAvailable()) {
    try {
      const res = await fetch(`${SERVER_BASE_URL}/v1/tasks`, {
        method: "POST",
        headers: {
          "Content-Type": "application/json",
          ...(DEV_TOKEN ? { Authorization: `Bearer ${DEV_TOKEN}` } : {}),
        },
        body: JSON.stringify(input),
      });
      if (res.ok) {
        const item: TaskItem = await res.json();
        const local = getLocal<TaskItem>(LOCAL_TASKS_KEY);
        setLocal(LOCAL_TASKS_KEY, [item, ...local]);
        return item;
      }
    } catch {
      // fallback
    }
  }

  const now = new Date().toISOString();
  const newItem: TaskItem = {
    id: crypto.randomUUID(),
    user_id: DEV_USER_ID,
    title: input.title,
    description: input.description ?? "",
    priority: input.priority ?? "P4",
    status: "todo",
    due_at: input.due_at ?? null,
    project: input.project ?? "Inbox",
    created_at: now,
    updated_at: now,
    completed_at: null,
  };

  const local = getLocal<TaskItem>(LOCAL_TASKS_KEY);
  setLocal(LOCAL_TASKS_KEY, [newItem, ...local]);
  return newItem;
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
  }
): Promise<TaskItem> {
  if (await isServerAvailable()) {
    try {
      const res = await fetch(`${SERVER_BASE_URL}/v1/tasks/${id}`, {
        method: "PATCH",
        headers: {
          "Content-Type": "application/json",
          ...(DEV_TOKEN ? { Authorization: `Bearer ${DEV_TOKEN}` } : {}),
        },
        body: JSON.stringify(input),
      });
      if (res.ok) {
        const item: TaskItem = await res.json();
        const local = getLocal<TaskItem>(LOCAL_TASKS_KEY);
        setLocal(
          LOCAL_TASKS_KEY,
          local.map((t) => (t.id === id ? item : t))
        );
        return item;
      }
    } catch {
      // fallback
    }
  }

  const now = new Date().toISOString();
  const local = getLocal<TaskItem>(LOCAL_TASKS_KEY);
  let updatedItem: TaskItem | null = null;

  const nextLocal = local.map((t) => {
    if (t.id === id) {
      const newStatus = input.status ?? t.status;
      updatedItem = {
        ...t,
        title: input.title ?? t.title,
        description: input.description ?? t.description,
        priority: input.priority ?? t.priority,
        status: newStatus,
        due_at: input.due_at !== undefined ? input.due_at : t.due_at,
        project: input.project ?? t.project,
        updated_at: now,
        completed_at: newStatus === "completed" ? now : newStatus === "todo" ? null : t.completed_at,
      };
      return updatedItem;
    }
    return t;
  });

  setLocal(LOCAL_TASKS_KEY, nextLocal);
  if (!updatedItem) throw new Error("Task not found");
  return updatedItem;
}

export async function deleteTask(id: string): Promise<void> {
  if (await isServerAvailable()) {
    try {
      await fetch(`${SERVER_BASE_URL}/v1/tasks/${id}`, {
        method: "DELETE",
        headers: DEV_TOKEN ? { Authorization: `Bearer ${DEV_TOKEN}` } : {},
      });
    } catch {
      // fallback
    }
  }
  const local = getLocal<TaskItem>(LOCAL_TASKS_KEY);
  setLocal(
    LOCAL_TASKS_KEY,
    local.filter((t) => t.id !== id)
  );
}

// ==================== REMINDERS ====================

export async function fetchReminders(filter?: { status?: string; q?: string }): Promise<ReminderItem[]> {
  if (await isServerAvailable()) {
    try {
      const query = new URLSearchParams();
      if (filter?.status) query.set("status", filter.status);
      if (filter?.q) query.set("q", filter.q);

      const res = await fetch(`${SERVER_BASE_URL}/v1/reminders?${query.toString()}`, {
        headers: DEV_TOKEN ? { Authorization: `Bearer ${DEV_TOKEN}` } : {},
      });
      if (res.ok) {
        const data = await res.json();
        setLocal(LOCAL_REMINDERS_KEY, data);
        return data;
      }
    } catch {
      // fallback
    }
  }

  let items = getLocal<ReminderItem>(LOCAL_REMINDERS_KEY);
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
  if (await isServerAvailable()) {
    try {
      const res = await fetch(`${SERVER_BASE_URL}/v1/reminders`, {
        method: "POST",
        headers: {
          "Content-Type": "application/json",
          ...(DEV_TOKEN ? { Authorization: `Bearer ${DEV_TOKEN}` } : {}),
        },
        body: JSON.stringify(input),
      });
      if (res.ok) {
        const item: ReminderItem = await res.json();
        const local = getLocal<ReminderItem>(LOCAL_REMINDERS_KEY);
        setLocal(LOCAL_REMINDERS_KEY, [item, ...local]);
        return item;
      }
    } catch {
      // fallback
    }
  }

  const now = new Date().toISOString();
  const newItem: ReminderItem = {
    id: crypto.randomUUID(),
    user_id: DEV_USER_ID,
    task_id: input.task_id ?? null,
    title: input.title,
    remind_at: input.remind_at,
    status: "pending",
    created_at: now,
    updated_at: now,
  };

  const local = getLocal<ReminderItem>(LOCAL_REMINDERS_KEY);
  setLocal(LOCAL_REMINDERS_KEY, [newItem, ...local]);
  return newItem;
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
  if (await isServerAvailable()) {
    try {
      const res = await fetch(`${SERVER_BASE_URL}/v1/reminders/${id}`, {
        method: "PATCH",
        headers: {
          "Content-Type": "application/json",
          ...(DEV_TOKEN ? { Authorization: `Bearer ${DEV_TOKEN}` } : {}),
        },
        body: JSON.stringify(input),
      });
      if (res.ok) {
        const item: ReminderItem = await res.json();
        const local = getLocal<ReminderItem>(LOCAL_REMINDERS_KEY);
        setLocal(
          LOCAL_REMINDERS_KEY,
          local.map((r) => (r.id === id ? item : r))
        );
        return item;
      }
    } catch {
      // fallback
    }
  }

  const now = new Date().toISOString();
  const local = getLocal<ReminderItem>(LOCAL_REMINDERS_KEY);
  let updatedItem: ReminderItem | null = null;

  const nextLocal = local.map((r) => {
    if (r.id === id) {
      updatedItem = {
        ...r,
        title: input.title ?? r.title,
        remind_at: input.remind_at ?? r.remind_at,
        status: input.status ?? r.status,
        task_id: input.task_id !== undefined ? input.task_id : r.task_id,
        updated_at: now,
      };
      return updatedItem;
    }
    return r;
  });

  setLocal(LOCAL_REMINDERS_KEY, nextLocal);
  if (!updatedItem) throw new Error("Reminder not found");
  return updatedItem;
}

export async function deleteReminder(id: string): Promise<void> {
  if (await isServerAvailable()) {
    try {
      await fetch(`${SERVER_BASE_URL}/v1/reminders/${id}`, {
        method: "DELETE",
        headers: DEV_TOKEN ? { Authorization: `Bearer ${DEV_TOKEN}` } : {},
      });
    } catch {
      // fallback
    }
  }
  const local = getLocal<ReminderItem>(LOCAL_REMINDERS_KEY);
  setLocal(
    LOCAL_REMINDERS_KEY,
    local.filter((r) => r.id !== id)
  );
}

// ==================== NOTES ====================

export async function fetchNotes(filter?: { is_archived?: boolean; q?: string }): Promise<NoteItem[]> {
  if (await isServerAvailable()) {
    try {
      const query = new URLSearchParams();
      if (filter?.is_archived !== undefined) query.set("is_archived", String(filter.is_archived));
      if (filter?.q) query.set("q", filter.q);

      const res = await fetch(`${SERVER_BASE_URL}/v1/notes?${query.toString()}`, {
        headers: DEV_TOKEN ? { Authorization: `Bearer ${DEV_TOKEN}` } : {},
      });
      if (res.ok) {
        const data = await res.json();
        setLocal(LOCAL_NOTES_KEY, data);
        return data;
      }
    } catch {
      // fallback
    }
  }

  let items = getLocal<NoteItem>(LOCAL_NOTES_KEY);
  if (filter?.is_archived !== undefined) items = items.filter((n) => n.is_archived === filter.is_archived);
  if (filter?.q) {
    const q = filter.q.toLowerCase();
    items = items.filter((n) => n.title.toLowerCase().includes(q) || n.content.toLowerCase().includes(q));
  }
  return items;
}

export async function createNote(input: {
  title: string;
  content?: string;
  tags?: string[];
}): Promise<NoteItem> {
  if (await isServerAvailable()) {
    try {
      const res = await fetch(`${SERVER_BASE_URL}/v1/notes`, {
        method: "POST",
        headers: {
          "Content-Type": "application/json",
          ...(DEV_TOKEN ? { Authorization: `Bearer ${DEV_TOKEN}` } : {}),
        },
        body: JSON.stringify(input),
      });
      if (res.ok) {
        const item: NoteItem = await res.json();
        const local = getLocal<NoteItem>(LOCAL_NOTES_KEY);
        setLocal(LOCAL_NOTES_KEY, [item, ...local]);
        return item;
      }
    } catch {
      // fallback
    }
  }

  const now = new Date().toISOString();
  const newItem: NoteItem = {
    id: crypto.randomUUID(),
    user_id: DEV_USER_ID,
    title: input.title,
    content: input.content ?? "",
    is_archived: false,
    tags: input.tags ?? [],
    created_at: now,
    updated_at: now,
  };

  const local = getLocal<NoteItem>(LOCAL_NOTES_KEY);
  setLocal(LOCAL_NOTES_KEY, [newItem, ...local]);
  return newItem;
}

export async function updateNote(
  id: string,
  input: {
    title?: string;
    content?: string;
    is_archived?: boolean;
    tags?: string[];
  }
): Promise<NoteItem> {
  if (await isServerAvailable()) {
    try {
      const res = await fetch(`${SERVER_BASE_URL}/v1/notes/${id}`, {
        method: "PATCH",
        headers: {
          "Content-Type": "application/json",
          ...(DEV_TOKEN ? { Authorization: `Bearer ${DEV_TOKEN}` } : {}),
        },
        body: JSON.stringify(input),
      });
      if (res.ok) {
        const item: NoteItem = await res.json();
        const local = getLocal<NoteItem>(LOCAL_NOTES_KEY);
        setLocal(
          LOCAL_NOTES_KEY,
          local.map((n) => (n.id === id ? item : n))
        );
        return item;
      }
    } catch {
      // fallback
    }
  }

  const now = new Date().toISOString();
  const local = getLocal<NoteItem>(LOCAL_NOTES_KEY);
  let updatedItem: NoteItem | null = null;

  const nextLocal = local.map((n) => {
    if (n.id === id) {
      updatedItem = {
        ...n,
        title: input.title ?? n.title,
        content: input.content ?? n.content,
        is_archived: input.is_archived !== undefined ? input.is_archived : n.is_archived,
        tags: input.tags ?? n.tags,
        updated_at: now,
      };
      return updatedItem;
    }
    return n;
  });

  setLocal(LOCAL_NOTES_KEY, nextLocal);
  if (!updatedItem) throw new Error("Note not found");
  return updatedItem;
}

export async function deleteNote(id: string): Promise<void> {
  if (await isServerAvailable()) {
    try {
      await fetch(`${SERVER_BASE_URL}/v1/notes/${id}`, {
        method: "DELETE",
        headers: DEV_TOKEN ? { Authorization: `Bearer ${DEV_TOKEN}` } : {},
      });
    } catch {
      // fallback
    }
  }
  const local = getLocal<NoteItem>(LOCAL_NOTES_KEY);
  setLocal(
    LOCAL_NOTES_KEY,
    local.filter((n) => n.id !== id)
  );
}

// ==================== IDEAS ====================

export async function fetchIdeas(filter?: { status?: string; q?: string }): Promise<IdeaItem[]> {
  if (await isServerAvailable()) {
    try {
      const query = new URLSearchParams();
      if (filter?.status) query.set("status", filter.status);
      if (filter?.q) query.set("q", filter.q);

      const res = await fetch(`${SERVER_BASE_URL}/v1/ideas?${query.toString()}`, {
        headers: DEV_TOKEN ? { Authorization: `Bearer ${DEV_TOKEN}` } : {},
      });
      if (res.ok) {
        const data = await res.json();
        setLocal(LOCAL_IDEAS_KEY, data);
        return data;
      }
    } catch {
      // fallback
    }
  }

  let items = getLocal<IdeaItem>(LOCAL_IDEAS_KEY);
  if (filter?.status) items = items.filter((i) => i.status === filter.status);
  if (filter?.q) {
    const q = filter.q.toLowerCase();
    items = items.filter((i) => i.title.toLowerCase().includes(q) || i.description.toLowerCase().includes(q));
  }
  return items;
}

export async function createIdea(input: { title: string; description?: string }): Promise<IdeaItem> {
  if (await isServerAvailable()) {
    try {
      const res = await fetch(`${SERVER_BASE_URL}/v1/ideas`, {
        method: "POST",
        headers: {
          "Content-Type": "application/json",
          ...(DEV_TOKEN ? { Authorization: `Bearer ${DEV_TOKEN}` } : {}),
        },
        body: JSON.stringify(input),
      });
      if (res.ok) {
        const item: IdeaItem = await res.json();
        const local = getLocal<IdeaItem>(LOCAL_IDEAS_KEY);
        setLocal(LOCAL_IDEAS_KEY, [item, ...local]);
        return item;
      }
    } catch {
      // fallback
    }
  }

  const now = new Date().toISOString();
  const newItem: IdeaItem = {
    id: crypto.randomUUID(),
    user_id: DEV_USER_ID,
    title: input.title,
    description: input.description ?? "",
    status: "active",
    converted_task_id: null,
    created_at: now,
    updated_at: now,
  };

  const local = getLocal<IdeaItem>(LOCAL_IDEAS_KEY);
  setLocal(LOCAL_IDEAS_KEY, [newItem, ...local]);
  return newItem;
}

export async function updateIdea(
  id: string,
  input: { title?: string; description?: string; status?: string }
): Promise<IdeaItem> {
  if (await isServerAvailable()) {
    try {
      const res = await fetch(`${SERVER_BASE_URL}/v1/ideas/${id}`, {
        method: "PATCH",
        headers: {
          "Content-Type": "application/json",
          ...(DEV_TOKEN ? { Authorization: `Bearer ${DEV_TOKEN}` } : {}),
        },
        body: JSON.stringify(input),
      });
      if (res.ok) {
        const item: IdeaItem = await res.json();
        const local = getLocal<IdeaItem>(LOCAL_IDEAS_KEY);
        setLocal(
          LOCAL_IDEAS_KEY,
          local.map((i) => (i.id === id ? item : i))
        );
        return item;
      }
    } catch {
      // fallback
    }
  }

  const now = new Date().toISOString();
  const local = getLocal<IdeaItem>(LOCAL_IDEAS_KEY);
  let updatedItem: IdeaItem | null = null;

  const nextLocal = local.map((i) => {
    if (i.id === id) {
      updatedItem = {
        ...i,
        title: input.title ?? i.title,
        description: input.description ?? i.description,
        status: input.status ?? i.status,
        updated_at: now,
      };
      return updatedItem;
    }
    return i;
  });

  setLocal(LOCAL_IDEAS_KEY, nextLocal);
  if (!updatedItem) throw new Error("Idea not found");
  return updatedItem;
}

export async function convertIdeaToTask(id: string): Promise<{ idea: IdeaItem; task: TaskItem }> {
  if (await isServerAvailable()) {
    try {
      const res = await fetch(`${SERVER_BASE_URL}/v1/ideas/${id}/convert`, {
        method: "POST",
        headers: DEV_TOKEN ? { Authorization: `Bearer ${DEV_TOKEN}` } : {},
      });
      if (res.ok) {
        const data = await res.json();
        const localIdeas = getLocal<IdeaItem>(LOCAL_IDEAS_KEY);
        const localTasks = getLocal<TaskItem>(LOCAL_TASKS_KEY);
        setLocal(
          LOCAL_IDEAS_KEY,
          localIdeas.map((i) => (i.id === id ? data.idea : i))
        );
        setLocal(LOCAL_TASKS_KEY, [data.task, ...localTasks]);
        return data;
      }
    } catch {
      // fallback
    }
  }

  const localIdeas = getLocal<IdeaItem>(LOCAL_IDEAS_KEY);
  const idea = localIdeas.find((i) => i.id === id);
  if (!idea) throw new Error("Idea not found");

  const task = await createTask({
    title: idea.title,
    description: idea.description,
    priority: "P4",
    project: "Inbox",
  });

  const updatedIdea = await updateIdea(id, { status: "converted" });
  return { idea: updatedIdea, task };
}

export async function deleteIdea(id: string): Promise<void> {
  if (await isServerAvailable()) {
    try {
      await fetch(`${SERVER_BASE_URL}/v1/ideas/${id}`, {
        method: "DELETE",
        headers: DEV_TOKEN ? { Authorization: `Bearer ${DEV_TOKEN}` } : {},
      });
    } catch {
      // fallback
    }
  }
  const local = getLocal<IdeaItem>(LOCAL_IDEAS_KEY);
  setLocal(
    LOCAL_IDEAS_KEY,
    local.filter((i) => i.id !== id)
  );
}
