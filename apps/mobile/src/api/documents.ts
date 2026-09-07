/**
 * Documents API layer (M8).
 *
 * The server owns processing, storage and ranking. This file is a thin
 * transport wrapper: no processing logic, no offline outbox (a document is
 * not the kind of thing you queue for later -- it has bytes attached, and
 * the pipeline needs to see them).
 */

import { SERVER_BASE_URL, DEV_TOKEN } from "./bridge";
import type {
  DocumentItem,
  DocumentPageItem,
  DocumentProcessingState,
  DocumentSearchHit,
} from "./types";

const BASE = () => `${SERVER_BASE_URL.replace(/\/+$/, "")}/v1/documents`;

function authJsonHeaders(): HeadersInit {
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
      // fall through
    }
    throw new Error(message);
  }
  return (await response.json()) as T;
}

async function readOk(response: Response): Promise<void> {
  if (!response.ok) {
    let message = `HTTP ${response.status}`;
    try {
      const body = await response.json();
      if (body?.message) message = body.message;
    } catch {
      // fall through
    }
    throw new Error(message);
  }
}

export interface ListDocumentsParams {
  q?: string;
  mime_type?: string;
  state?: DocumentProcessingState[];
  limit?: number;
}

function buildListQuery(params: ListDocumentsParams): string {
  const search = new URLSearchParams();
  if (params.q) search.set("q", params.q);
  if (params.mime_type) search.set("mime_type", params.mime_type);
  if (params.limit != null) search.set("limit", String(params.limit));
  if (params.state && params.state.length > 0)
    search.set("state", params.state.join(","));
  const qs = search.toString();
  return qs ? `?${qs}` : "";
}

export async function listDocuments(
  params: ListDocumentsParams = {},
): Promise<DocumentItem[]> {
  const response = await fetch(`${BASE()}${buildListQuery(params)}`, {
    headers: { Authorization: `Bearer ${DEV_TOKEN}` },
  });
  return readJson<DocumentItem[]>(response);
}

export async function getDocument(id: string): Promise<DocumentItem> {
  const response = await fetch(`${BASE()}/${encodeURIComponent(id)}`, {
    headers: { Authorization: `Bearer ${DEV_TOKEN}` },
  });
  return readJson<DocumentItem>(response);
}

/**
 * Uploads a document by streaming the raw bytes. The MIME type must be one of
 * the server's supported types (see `is_supported_mime` on the server); an
 * unsupported type comes back as a 400.
 */
export async function uploadDocument(
  file: File | Blob,
  opts: { filename?: string; mimeType?: string } = {},
): Promise<DocumentItem> {
  const filename =
    opts.filename ?? (file as File).name ?? "document";
  const mimeType =
    opts.mimeType ?? (file as File).type ?? "application/octet-stream";
  const response = await fetch(BASE(), {
    method: "POST",
    headers: {
      Authorization: `Bearer ${DEV_TOKEN}`,
      "Content-Type": mimeType,
      "X-Filename": filename,
    },
    body: file,
  });
  return readJson<DocumentItem>(response);
}

export async function ingestFromDrive(
  accountId: string,
  fileId: string,
): Promise<DocumentItem> {
  const response = await fetch(`${BASE()}/from-drive`, {
    method: "POST",
    headers: authJsonHeaders(),
    body: JSON.stringify({ account_id: accountId, file_id: fileId }),
  });
  return readJson<DocumentItem>(response);
}

export async function listPages(id: string): Promise<DocumentPageItem[]> {
  const response = await fetch(`${BASE()}/${encodeURIComponent(id)}/pages`, {
    headers: { Authorization: `Bearer ${DEV_TOKEN}` },
  });
  return readJson<DocumentPageItem[]>(response);
}

export async function getPage(
  id: string,
  pageNumber: number,
): Promise<DocumentPageItem> {
  const response = await fetch(
    `${BASE()}/${encodeURIComponent(id)}/pages/${pageNumber}`,
    { headers: { Authorization: `Bearer ${DEV_TOKEN}` } },
  );
  return readJson<DocumentPageItem>(response);
}

export async function reprocessDocument(id: string): Promise<DocumentItem> {
  const response = await fetch(
    `${BASE()}/${encodeURIComponent(id)}/reprocess`,
    {
      method: "POST",
      headers: authJsonHeaders(),
    },
  );
  return readJson<DocumentItem>(response);
}

export async function deleteDocument(id: string): Promise<void> {
  const response = await fetch(`${BASE()}/${encodeURIComponent(id)}`, {
    method: "DELETE",
    headers: { Authorization: `Bearer ${DEV_TOKEN}` },
  });
  await readOk(response);
}

export interface SearchPagesParams {
  q: string;
  limit?: number;
}

export async function searchPages(
  params: SearchPagesParams,
): Promise<DocumentSearchHit[]> {
  const search = new URLSearchParams();
  search.set("q", params.q);
  if (params.limit != null) search.set("limit", String(params.limit));
  const response = await fetch(`${BASE()}/search?${search.toString()}`, {
    headers: { Authorization: `Bearer ${DEV_TOKEN}` },
  });
  return readJson<DocumentSearchHit[]>(response);
}

export function humaniseSize(size: number): string {
  if (size < 1024) return `${size} B`;
  if (size < 1024 * 1024) return `${Math.round(size / 102.4) / 10} KB`;
  return `${Math.round(size / (1024 * 102.4)) / 10} MB`;
}

export function stateLabel(state: DocumentProcessingState): string {
  switch (state) {
    case "uploaded":
      return "Uploaded";
    case "extracting":
      return "Extracting…";
    case "ocr":
      return "OCR…";
    case "verifying":
      return "Verifying…";
    case "indexed":
      return "Indexed";
    case "failed":
      return "Failed";
    default:
      return state;
  }
}
