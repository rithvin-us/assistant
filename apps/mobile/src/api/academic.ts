/**
 * Classroom, Drive and unified academic context.
 *
 * Every call names an account explicitly. With several Google accounts
 * connected, inferring which one a request meant would quietly read the wrong
 * person's coursework, so `account_id` is always a parameter and never a
 * default.
 *
 * The read paths default to the server's cache. `refresh: true` is what asks
 * Google, and it is only ever passed in response to something the user did --
 * nothing here polls.
 */

import { SERVER_BASE_URL, DEV_TOKEN } from "./bridge";
import type {
  AcademicOverview,
  AcademicSyncResult,
  Announcement,
  Course,
  CourseworkItem,
  DriveFile,
  DriveFileContent,
} from "./types";

function authHeaders(): Record<string, string> {
  const headers: Record<string, string> = {
    "Content-Type": "application/json",
  };
  if (DEV_TOKEN) {
    headers["Authorization"] = `Bearer ${DEV_TOKEN}`;
  }
  return headers;
}

/**
 * Turns a failed response into a message worth showing.
 *
 * The server already replaces Google's error bodies with a sentence a person
 * can act on ("reconnect this account", "an administrator may need to allow
 * it"), so that text is surfaced as-is rather than being buried under a
 * status code.
 */
async function readError(res: Response, fallback: string): Promise<Error> {
  const text = await res.text().catch(() => "");
  if (!text) return new Error(fallback);
  try {
    const parsed = JSON.parse(text) as { message?: string; error?: string };
    return new Error(parsed.message ?? parsed.error ?? text);
  } catch {
    return new Error(text);
  }
}

async function getJson<T>(url: string, fallback: string): Promise<T> {
  const res = await fetch(url, { headers: authHeaders() });
  if (!res.ok) throw await readError(res, fallback);
  return res.json() as Promise<T>;
}

// ==================== CLASSROOM ====================

export async function fetchCourses(
  accountId: string,
  refresh = false,
): Promise<Course[]> {
  const params = new URLSearchParams({ account_id: accountId });
  if (refresh) params.set("refresh", "true");
  return getJson<Course[]>(
    `${SERVER_BASE_URL}/v1/classroom/courses?${params.toString()}`,
    "Could not load courses.",
  );
}

export async function fetchCoursework(
  accountId: string,
  courseId?: string,
  refresh = false,
): Promise<CourseworkItem[]> {
  const params = new URLSearchParams({ account_id: accountId });
  if (courseId) params.set("course_id", courseId);
  if (refresh) params.set("refresh", "true");
  return getJson<CourseworkItem[]>(
    `${SERVER_BASE_URL}/v1/classroom/coursework?${params.toString()}`,
    "Could not load coursework.",
  );
}

export async function fetchAnnouncements(
  accountId: string,
  courseId?: string,
  limit = 20,
  refresh = false,
): Promise<Announcement[]> {
  const params = new URLSearchParams({
    account_id: accountId,
    limit: String(limit),
  });
  if (courseId) params.set("course_id", courseId);
  if (refresh) params.set("refresh", "true");
  return getJson<Announcement[]>(
    `${SERVER_BASE_URL}/v1/classroom/announcements?${params.toString()}`,
    "Could not load announcements.",
  );
}

// ==================== DRIVE ====================

export async function searchDrive(
  accountId: string,
  query: string,
  mimeType?: string,
  limit = 25,
): Promise<DriveFile[]> {
  const params = new URLSearchParams({
    account_id: accountId,
    q: query,
    limit: String(limit),
  });
  if (mimeType) params.set("mime_type", mimeType);
  return getJson<DriveFile[]>(
    `${SERVER_BASE_URL}/v1/drive/search?${params.toString()}`,
    "Could not search Drive.",
  );
}

export async function listDrive(
  accountId: string,
  folderId?: string,
  limit = 50,
): Promise<DriveFile[]> {
  const params = new URLSearchParams({
    account_id: accountId,
    limit: String(limit),
  });
  if (folderId) params.set("folder_id", folderId);
  return getJson<DriveFile[]>(
    `${SERVER_BASE_URL}/v1/drive/files?${params.toString()}`,
    "Could not list Drive files.",
  );
}

export async function fetchDriveMetadata(
  accountId: string,
  fileId: string,
): Promise<DriveFile> {
  const params = new URLSearchParams({ account_id: accountId });
  return getJson<DriveFile>(
    `${SERVER_BASE_URL}/v1/drive/files/${encodeURIComponent(fileId)}?${params.toString()}`,
    "Could not read file details.",
  );
}

/**
 * Reads a small text file.
 *
 * The server refuses anything oversized, binary, or of unknown length, and the
 * refusal arrives as an error with a readable reason. Callers must show it
 * rather than rendering an empty document, which would look like a file that
 * was read and happened to be blank.
 */
export async function readDriveFile(
  accountId: string,
  fileId: string,
): Promise<DriveFileContent> {
  const params = new URLSearchParams({ account_id: accountId });
  return getJson<DriveFileContent>(
    `${SERVER_BASE_URL}/v1/drive/files/${encodeURIComponent(fileId)}/content?${params.toString()}`,
    "Could not read that file.",
  );
}

// ==================== ACADEMIC CONTEXT ====================

export async function fetchAcademicOverview(
  limit = 10,
): Promise<AcademicOverview> {
  return getJson<AcademicOverview>(
    `${SERVER_BASE_URL}/v1/academic/overview?limit=${limit}`,
    "Could not load the academic overview.",
  );
}

/** Explicit refresh. Imports coursework into tasks; safe to run repeatedly. */
export async function syncAcademic(
  accountId: string,
  announcementsPerCourse = 5,
): Promise<AcademicSyncResult> {
  const res = await fetch(`${SERVER_BASE_URL}/v1/academic/sync`, {
    method: "POST",
    headers: authHeaders(),
    body: JSON.stringify({
      account_id: accountId,
      announcements_per_course: announcementsPerCourse,
    }),
  });
  if (!res.ok) throw await readError(res, "Sync failed.");
  return res.json() as Promise<AcademicSyncResult>;
}

// ==================== FORMATTING HELPERS ====================

/** Formats a deadline, or says plainly that there is not one. */
export function formatDue(dueAt: string | null | undefined): string {
  if (!dueAt) return "No due date";
  const date = new Date(dueAt);
  if (Number.isNaN(date.getTime())) return "No due date";
  return date.toLocaleString(undefined, {
    weekday: "short",
    day: "numeric",
    month: "short",
    hour: "numeric",
    minute: "2-digit",
  });
}

/** Human-readable file size. Drive omits sizes for its own editor formats. */
export function formatSize(bytes: number | null | undefined): string {
  if (bytes === null || bytes === undefined) return "";
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${Math.round(bytes / 1024)} KB`;
  return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
}

/**
 * How stale the cache is, phrased so the screen never implies a live read.
 */
export function formatSynced(syncedAt: string | null | undefined): string {
  if (!syncedAt) return "Not synced yet";
  const then = new Date(syncedAt).getTime();
  if (Number.isNaN(then)) return "Not synced yet";
  const minutes = Math.floor((Date.now() - then) / 60000);
  if (minutes < 1) return "Synced just now";
  if (minutes < 60) return `Synced ${minutes} min ago`;
  const hours = Math.floor(minutes / 60);
  if (hours < 24) return `Synced ${hours} h ago`;
  return `Synced ${Math.floor(hours / 24)} d ago`;
}

/** A short, human label for a Drive MIME type. */
export function fileKind(mimeType: string): string {
  if (mimeType === "application/vnd.google-apps.folder") return "Folder";
  if (mimeType === "application/vnd.google-apps.document") return "Google Doc";
  if (mimeType === "application/vnd.google-apps.spreadsheet") return "Google Sheet";
  if (mimeType === "application/vnd.google-apps.presentation") return "Google Slides";
  if (mimeType === "application/pdf") return "PDF";
  if (mimeType.startsWith("image/")) return "Image";
  if (mimeType.startsWith("video/")) return "Video";
  if (mimeType.startsWith("audio/")) return "Audio";
  if (mimeType.startsWith("text/")) return "Text";
  return "File";
}
