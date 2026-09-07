/**
 * Voice API Client & WebAudio helper methods for M10.
 */

import { SERVER_BASE_URL, DEV_TOKEN } from './bridge';

export interface TranscribeResponse {
  text: string;
}

export interface SpeakResponse {
  audio_base64: string;
  encoding: string;
}

export interface VoiceDiagnosticResponse {
  cartesia_configured: boolean;
  stt_model: string;
  tts_model: string;
  tts_voice_id: string;
  status: string;
}

const BASE = () => `${SERVER_BASE_URL.replace(/\/+$/, '')}/v1/voice`;

function authHeaders(): HeadersInit {
  return {
    Authorization: `Bearer ${DEV_TOKEN}`,
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

export async function transcribeVoiceAudio(audioBlob: Blob): Promise<TranscribeResponse> {
  const arrayBuffer = await audioBlob.arrayBuffer();
  return request<TranscribeResponse>(`${BASE()}/transcribe`, {
    method: 'POST',
    headers: {
      'Content-Type': audioBlob.type || 'audio/wav',
    },
    body: arrayBuffer,
  });
}

export async function speakVoiceText(text: string, voiceId?: string): Promise<SpeakResponse> {
  return request<SpeakResponse>(`${BASE()}/speak`, {
    method: 'POST',
    headers: {
      'Content-Type': 'application/json',
    },
    body: JSON.stringify({ text, voice_id: voiceId }),
  });
}

export async function getVoiceDiagnostic(): Promise<VoiceDiagnosticResponse> {
  return request<VoiceDiagnosticResponse>(`${BASE()}/diagnostic`, {
    method: 'GET',
  });
}

/** Audio Player helper for playing base64 audio chunks/responses */
export class AudioPlaybackController {
  private currentAudio: HTMLAudioElement | null = null;

  playBase64(base64Data: string, mimeType: string = 'audio/wav'): Promise<void> {
    this.stop();
    return new Promise((resolve, reject) => {
      try {
        const audioSrc = `data:${mimeType};base64,${base64Data}`;
        const audio = new Audio(audioSrc);
        this.currentAudio = audio;
        audio.onended = () => {
          this.currentAudio = null;
          resolve();
        };
        audio.onerror = (e) => {
          this.currentAudio = null;
          reject(e);
        };
        audio.play().catch(reject);
      } catch (err) {
        reject(err);
      }
    });
  }

  stop() {
    if (this.currentAudio) {
      this.currentAudio.pause();
      this.currentAudio.currentTime = 0;
      this.currentAudio = null;
    }
  }
}
