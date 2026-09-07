/**
 * Long-term Memory API layer.
 *
 * Deterministic HTTP calls to `assistant-server`. No model provider is
 * involved in any of it: the server holds authoritative memory, this file just
 * wraps the endpoints defined in `services/assistant-server/src/routes/memory.rs`.
 *
 * Memory is not treated as an offline outbox (see productivity.ts for the
 * outbox pattern used there). A memory is a durable, attributed record; if the
 * server is unreachable, creation is refused up-front rather than shown as
 * saved in a per-browser cache that would never sync. That matches the M7
 * rule: the model may propose, the application (server) decides.
 */

import { SERVER_BASE_URL, DEV_TOKEN } from "./bridge";
import type {
  MemoryItem,
  MemoryKind,
  MemoryLifecycle,
  MemorySource,
  CreateMemoryRequest,
  UpdateMemoryRequest,
} from "./types";

const BASE = () => `${SERVER_BASE_URL.replace(/\/+$/, "")}/v1/memories`;

function authHeaders(): HeadersInit {
  return {
    Authorization: `Bearer ${DEV_TOKEN}`,
    "Content-Type": "application/json",
  };
}

async function readJson<T>(response: Response): Promise<T> {
  if (!response.ok) {
    let message = `HTTP ${response.status}`;
    try {
      const body = await response.json();
      if (body?.message) message = body.message;
    } catch {
      // fall through with the generic message
    }
    throw new Error(message);
  }
  return (await response.json()) as T;
}

export interface MemorySearchParams {
  q?: string;
  kind?: MemoryKind[];
  lifecycle?: MemoryLifecycle[];
  minImportance?: number;
  limit?: number;
}

function buildQuery(params: MemorySearchParams): string {
  const search = new URLSearchParams();
  if (params.q) search.set("q", params.q);
  if (params.minImportance != null)
    search.set("min_importance", String(params.minImportance));
  if (params.limit != null) search.set("limit", String(params.limit));
  // `kind` and `lifecycle` are comma-separated because axum's default query
  // extractor uses serde_urlencoded, which does not accept repeated keys as
  // a `Vec`. The server splits these on `,` before deserialising each piece.
  if (params.kind && params.kind.length > 0)
    search.set("kind", params.kind.join(","));
  if (params.lifecycle && params.lifecycle.length > 0)
    search.set("lifecycle", params.lifecycle.join(","));
  const qs = search.toString();
  return qs ? `?${qs}` : "";
}

export async function listMemories(
  params: MemorySearchParams = {},
): Promise<MemoryItem[]> {
  const response = await fetch(`${BASE()}${buildQuery(params)}`, {
    headers: authHeaders(),
  });
  return readJson<MemoryItem[]>(response);
}

export async function getMemory(id: string): Promise<MemoryItem> {
  const response = await fetch(`${BASE()}/${encodeURIComponent(id)}`, {
    headers: authHeaders(),
  });
  return readJson<MemoryItem>(response);
}

export interface CreateMemoryInput {
  kind: MemoryKind;
  content: string;
  importance?: number;
  confidence?: number;
  source_kind?: MemorySource;
  source_ref?: string;
  expires_at?: string | null;
  supersedes?: string | null;
}

export async function createMemory(input: CreateMemoryInput): Promise<MemoryItem> {
  const body: CreateMemoryRequest = {
    kind: input.kind,
    content: input.content,
    importance: input.importance,
    confidence: input.confidence,
    source_kind: input.source_kind,
    source_ref: input.source_ref,
    expires_at: input.expires_at ?? undefined,
    supersedes: input.supersedes ?? undefined,
  };
  const response = await fetch(BASE(), {
    method: "POST",
    headers: authHeaders(),
    body: JSON.stringify(body),
  });
  return readJson<MemoryItem>(response);
}

export async function updateMemory(
  id: string,
  patch: UpdateMemoryRequest,
): Promise<MemoryItem> {
  const response = await fetch(`${BASE()}/${encodeURIComponent(id)}`, {
    method: "PATCH",
    headers: authHeaders(),
    body: JSON.stringify(patch),
  });
  return readJson<MemoryItem>(response);
}

export async function archiveMemory(id: string): Promise<MemoryItem> {
  const response = await fetch(
    `${BASE()}/${encodeURIComponent(id)}/archive`,
    {
      method: "POST",
      headers: authHeaders(),
    },
  );
  return readJson<MemoryItem>(response);
}

export async function restoreMemory(id: string): Promise<MemoryItem> {
  const response = await fetch(
    `${BASE()}/${encodeURIComponent(id)}/restore`,
    {
      method: "POST",
      headers: authHeaders(),
    },
  );
  return readJson<MemoryItem>(response);
}

export const MEMORY_KIND_LABELS: Record<MemoryKind, string> = {
  preference: "Preference",
  fact: "Fact",
  idea: "Idea",
  commitment: "Commitment",
  project: "Project",
  temporary: "Temporary",
};

export const MEMORY_SOURCE_LABELS: Record<MemorySource, string> = {
  explicit_user_input: "You told me",
  conversation: "Conversation",
  task: "Task",
  note: "Note",
  idea: "Idea",
  project: "Project",
  document: "Document",
  external_source: "External source",
};

export function importanceLabel(value: number): string {
  switch (Math.max(1, Math.min(5, Math.round(value)))) {
    case 1:
      return "Very low";
    case 2:
      return "Low";
    case 3:
      return "Normal";
    case 4:
      return "High";
    case 5:
      return "Very high";
    default:
      return `Level ${value}`;
  }
}
