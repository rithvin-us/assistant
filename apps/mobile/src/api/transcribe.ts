import { SERVER_BASE_URL, DEV_TOKEN } from "./bridge";

export interface TranscribeResult {
  text: string;
}

export async function transcribeAudio(audioBlob: Blob): Promise<string> {
  const url = `${SERVER_BASE_URL.replace(/\/+$/, "")}/v1/audio/transcribe`;
  const response = await fetch(url, {
    method: "POST",
    headers: {
      Authorization: `Bearer ${DEV_TOKEN}`,
      "Content-Type": audioBlob.type || "audio/webm",
    },
    body: audioBlob,
  });

  if (!response.ok) {
    const errorData = await response.json().catch(() => null);
    throw new Error(
      errorData?.message || `Transcription failed with status ${response.status}`,
    );
  }

  const data = (await response.json()) as TranscribeResult;
  return data.text;
}
