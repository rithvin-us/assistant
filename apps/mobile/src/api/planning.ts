import { SERVER_BASE_URL, DEV_TOKEN } from './bridge';
import type { TodayPlan, UpcomingPlanning, Conflict } from './types';

const BASE = () => `${SERVER_BASE_URL.replace(/\/+$/, '')}/v1/planning`;

function authHeaders(): HeadersInit {
  return {
    Authorization: `Bearer ${DEV_TOKEN}`,
    'Content-Type': 'application/json',
  };
}

async function request<T>(url: string, init?: RequestInit): Promise<T> {
  const res = await fetch(url, {
    ...init,
    headers: {
      ...authHeaders(),
      ...init?.headers,
    },
  });

  if (!res.ok) {
    const err = await res.json().catch(() => ({ message: res.statusText }));
    throw new Error(err.message || `Request failed with status ${res.status}`);
  }

  return res.json() as Promise<T>;
}

export async function getTodayPlan(): Promise<TodayPlan> {
  return request<TodayPlan>(`${BASE()}/today`);
}

export async function getUpcomingPlanning(days = 7): Promise<UpcomingPlanning> {
  return request<UpcomingPlanning>(`${BASE()}/upcoming?days=${days}`);
}

export async function analyzePlanning(horizonDays = 7): Promise<UpcomingPlanning> {
  return request<UpcomingPlanning>(`${BASE()}/analyze`, {
    method: 'POST',
    body: JSON.stringify({ horizon_days: horizonDays }),
  });
}

export async function getConflicts(): Promise<Conflict[]> {
  return request<Conflict[]>(`${BASE()}/conflicts`);
}
