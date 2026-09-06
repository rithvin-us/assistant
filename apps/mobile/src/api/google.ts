/**
 * Google Ecosystem API Client (Accounts, Gmail, Calendar, Free Time).
 *
 * All requests are scoped to the authenticated user and require an explicit account_id
 * to ensure strict multi-account isolation.
 */

import { SERVER_BASE_URL, DEV_TOKEN } from "./bridge";
import type {
  AccountSummary,
  CalendarEvent,
  CreateCalendarEvent,
  EmailDetail,
  EmailSummary,
  FreeSlot,
  UpdateCalendarEvent,
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

// ==================== OAUTH & ACCOUNTS ====================

export async function startGoogleOAuth(redirectUri?: string): Promise<{ auth_url: string }> {
  const res = await fetch(`${SERVER_BASE_URL}/v1/auth/google/start`, {
    method: "POST",
    headers: authHeaders(),
    body: JSON.stringify({ redirect_uri: redirectUri ?? null }),
  });
  if (!res.ok) {
    const text = await res.text();
    throw new Error(`Failed to start Google OAuth: ${text}`);
  }
  return res.json();
}

export async function exchangeGoogleOAuth(
  code: string,
  redirectUri: string,
): Promise<AccountSummary> {
  const res = await fetch(`${SERVER_BASE_URL}/v1/auth/google/exchange`, {
    method: "POST",
    headers: authHeaders(),
    body: JSON.stringify({ code, redirect_uri: redirectUri }),
  });
  if (!res.ok) {
    const text = await res.text();
    throw new Error(`Failed to exchange Google OAuth code: ${text}`);
  }
  return res.json();
}

export async function fetchGoogleAccounts(): Promise<AccountSummary[]> {
  const res = await fetch(`${SERVER_BASE_URL}/v1/google/accounts`, {
    headers: authHeaders(),
  });
  if (!res.ok) {
    const text = await res.text();
    throw new Error(`Failed to fetch Google accounts: ${text}`);
  }
  return res.json();
}

export async function disconnectGoogleAccount(id: string): Promise<void> {
  const res = await fetch(`${SERVER_BASE_URL}/v1/google/accounts/${id}`, {
    method: "DELETE",
    headers: authHeaders(),
  });
  if (!res.ok) {
    const text = await res.text();
    throw new Error(`Failed to disconnect account: ${text}`);
  }
}

// ==================== GMAIL ====================

export async function searchGmail(
  accountId: string,
  q: string,
  limit: number = 20,
): Promise<EmailSummary[]> {
  const query = new URLSearchParams({
    account_id: accountId,
    q,
    limit: limit.toString(),
  });
  const res = await fetch(`${SERVER_BASE_URL}/v1/google/gmail/search?${query.toString()}`, {
    headers: authHeaders(),
  });
  if (!res.ok) {
    const text = await res.text();
    throw new Error(`Gmail search failed: ${text}`);
  }
  return res.json();
}

export async function readGmail(
  accountId: string,
  messageId: string,
): Promise<EmailDetail> {
  const query = new URLSearchParams({ account_id: accountId });
  const res = await fetch(
    `${SERVER_BASE_URL}/v1/google/gmail/messages/${messageId}?${query.toString()}`,
    {
      headers: authHeaders(),
    },
  );
  if (!res.ok) {
    const text = await res.text();
    throw new Error(`Failed to read email: ${text}`);
  }
  return res.json();
}

// ==================== CALENDAR ====================

export async function fetchCalendarEvents(
  accountId: string,
  timeMin: string,
  timeMax: string,
): Promise<CalendarEvent[]> {
  const query = new URLSearchParams({
    account_id: accountId,
    time_min: timeMin,
    time_max: timeMax,
  });
  const res = await fetch(
    `${SERVER_BASE_URL}/v1/google/calendar/events?${query.toString()}`,
    {
      headers: authHeaders(),
    },
  );
  if (!res.ok) {
    const text = await res.text();
    throw new Error(`Failed to fetch calendar events: ${text}`);
  }
  return res.json();
}

export async function createCalendarEvent(
  accountId: string,
  event: CreateCalendarEvent,
): Promise<CalendarEvent> {
  const query = new URLSearchParams({ account_id: accountId });
  const res = await fetch(
    `${SERVER_BASE_URL}/v1/google/calendar/events?${query.toString()}`,
    {
      method: "POST",
      headers: authHeaders(),
      body: JSON.stringify(event),
    },
  );
  if (!res.ok) {
    const text = await res.text();
    throw new Error(`Failed to create calendar event: ${text}`);
  }
  return res.json();
}

export async function updateCalendarEvent(
  accountId: string,
  eventId: string,
  event: UpdateCalendarEvent,
): Promise<CalendarEvent> {
  const query = new URLSearchParams({ account_id: accountId });
  const res = await fetch(
    `${SERVER_BASE_URL}/v1/google/calendar/events/${eventId}?${query.toString()}`,
    {
      method: "PATCH",
      headers: authHeaders(),
      body: JSON.stringify(event),
    },
  );
  if (!res.ok) {
    const text = await res.text();
    throw new Error(`Failed to update calendar event: ${text}`);
  }
  return res.json();
}

export async function deleteCalendarEvent(
  accountId: string,
  eventId: string,
): Promise<void> {
  const query = new URLSearchParams({ account_id: accountId });
  const res = await fetch(
    `${SERVER_BASE_URL}/v1/google/calendar/events/${eventId}?${query.toString()}`,
    {
      method: "DELETE",
      headers: authHeaders(),
    },
  );
  if (!res.ok) {
    const text = await res.text();
    throw new Error(`Failed to delete calendar event: ${text}`);
  }
}

export async function fetchFreeSlots(
  accountId: string,
  startTime: string,
  endTime: string,
  durationMinutes: number,
): Promise<FreeSlot[]> {
  const query = new URLSearchParams({
    account_id: accountId,
    start_time: startTime,
    end_time: endTime,
    duration_minutes: durationMinutes.toString(),
  });
  const res = await fetch(
    `${SERVER_BASE_URL}/v1/google/calendar/free-slots?${query.toString()}`,
    {
      headers: authHeaders(),
    },
  );
  if (!res.ok) {
    const text = await res.text();
    throw new Error(`Failed to fetch free slots: ${text}`);
  }
  return res.json();
}
