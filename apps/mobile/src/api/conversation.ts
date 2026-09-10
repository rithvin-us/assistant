/**
 * The conversation socket, as React sees it.
 *
 * The socket itself lives in the Tauri shell (ADR-0008): this module invokes
 * commands and subscribes to the events the shell emits. No component opens a
 * connection, holds a token, or parses a frame.
 *
 * Frames cross the boundary as JSON text and are parsed here, once, against
 * `types.ts` — the mirror of `crates/assistant-protocol`. The shell forwards
 * them verbatim rather than reshaping them, so there is exactly one contract.
 */

import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";

import { getServerBaseUrl, DEV_TOKEN, isTauri } from "./bridge";
import type { ClientFrame, ServerFrame } from "./types";

const FRAME_EVENT = "conversation://frame";
const STATUS_EVENT = "conversation://status";

/** The socket's lifecycle, which is not part of the wire protocol. */
export type SocketStatus = { state: "open" } | { state: "closed"; reason: string };

/**
 * Opens the conversation socket for one conversation id.
 *
 * Throws a short, user-safe message. The shell deliberately does not pass
 * through the transport error, because the socket URL carries the access token
 * in its query string and a transport error can quote the URL.
 */
export async function openConversation(conversationId: string): Promise<void> {
  if (!isTauri) {
    throw new Error("The assistant is only available in the app.");
  }
  await invoke("conversation_open", {
    baseUrl: getServerBaseUrl(),
    token: DEV_TOKEN,
    conversationId,
  });
}


export async function closeConversation(): Promise<void> {
  if (!isTauri) return;
  await invoke("conversation_close");
}

/** Sends one user turn. */
export async function sendUserText(text: string): Promise<void> {
  const frame: ClientFrame = { type: "user_text", text };
  await invoke("conversation_send", { frame: JSON.stringify(frame) });
}

/**
 * Subscribes to server frames and socket status.
 *
 * Returns a function that removes both listeners. A frame that does not parse
 * is dropped with a console warning rather than thrown: one malformed frame
 * must not tear down a conversation the user is in the middle of.
 */
export async function subscribe(handlers: {
  onFrame: (frame: ServerFrame) => void;
  onStatus: (status: SocketStatus) => void;
}): Promise<UnlistenFn> {
  if (!isTauri) return () => {};

  const stopFrames = await listen<string>(FRAME_EVENT, (event) => {
    try {
      handlers.onFrame(JSON.parse(event.payload) as ServerFrame);
    } catch {
      console.warn("dropped an unparseable frame");
    }
  });

  const stopStatus = await listen<SocketStatus>(STATUS_EVENT, (event) => {
    handlers.onStatus(event.payload);
  });

  return () => {
    stopFrames();
    stopStatus();
  };
}

/** An action the turn stopped on, waiting for the user to approve it. */
export interface PendingApprovalNotice {
  name: string;
  summary: string;
  reason: string;
}

/** The outcome of a turn: what was said, and anything it stopped short of doing. */
export interface TurnOutcome {
  text: string;
  /**
   * Tools the turn proposed but did not run. Non-empty means the assistant
   * stopped and is waiting for approval -- nothing was executed.
   */
  pendingApprovals: PendingApprovalNotice[];
}

/** A turn that could not be completed. Carries the server's code where there is one. */
export class TurnFailedError extends Error {
  constructor(
    message: string,
    readonly code: string,
  ) {
    super(message);
    this.name = "TurnFailedError";
  }
}

/**
 * Executes one full assistant turn over the WebSocket and returns the answer.
 *
 * Pass a stable `conversationId` to keep context across turns. Omitting it mints
 * a fresh conversation, which is what the voice path used to do on every turn --
 * so a follow-up like "which one is due first?" arrived with no history behind
 * it and the assistant could not possibly answer it.
 *
 * Throws rather than returning placeholder prose. It previously returned strings
 * like "I heard you, but the assistant connection is offline." and "Done." as if
 * they were the assistant's reply; the voice path then spoke them aloud, so a
 * connection failure was indistinguishable from an answer.
 */
export async function executeTurn(
  userText: string,
  conversationId?: string,
): Promise<TurnOutcome> {
  const id = conversationId || crypto.randomUUID();
  let fullAnswer = "";
  // Approval-required frames were ignored entirely, so a voice request that
  // needed approval produced no audible acknowledgement and nothing ran -- the
  // user had no way to know the assistant had stopped and was waiting.
  const pendingApprovals: PendingApprovalNotice[] = [];

  try {
    await openConversation(id);
  } catch {
    throw new TurnFailedError(
      "Couldn't reach the assistant.",
      "assistant_unreachable",
    );
  }

  return new Promise<TurnOutcome>((resolve, reject) => {
    let unlisten: UnlistenFn | null = null;
    const timer = setTimeout(() => {
      cleanup();
      if (fullAnswer) {
        resolve({ text: fullAnswer, pendingApprovals });
      } else {
        reject(
          new TurnFailedError(
            "The assistant took too long to respond. Please try again.",
            "turn_timeout"
          )
        );
      }
    }, 30_000);

    const cleanup = () => {
      clearTimeout(timer);
      if (unlisten) unlisten();
      void closeConversation();
    };

    void subscribe({
      onFrame: (frame) => {
        if (frame.type === "assistant_delta") {
          fullAnswer += frame.text;
        } else if (frame.type === "approval_required") {
          pendingApprovals.push({
            name: frame.name,
            summary: frame.summary,
            reason: frame.reason,
          });
        } else if (frame.type === "turn_end") {
          cleanup();
          // An empty answer is returned as empty. The caller decides what that
          // means; inventing "Done." put words in the assistant's mouth.
          resolve({ text: fullAnswer, pendingApprovals });
        } else if (frame.type === "error") {
          cleanup();
          reject(new TurnFailedError(friendlyError(frame.code, frame.message), frame.code));
        }
      },
      onStatus: (status) => {
        if (status.state === "closed") {
          cleanup();
          // A socket that closes mid-turn has not answered. If some text had
          // already streamed it is real and worth keeping; nothing else is.
          if (fullAnswer) {
            resolve({ text: fullAnswer, pendingApprovals });
          } else {
            reject(
              new TurnFailedError(
                "The connection closed before the assistant answered.",
                "connection_closed",
              ),
            );
          }
        }
      },
    }).then((un) => {
      unlisten = un;
      void sendUserText(userText);
    });
  });
}


/**
 * Turns a server error code into something worth showing a user.
 *
 * The server's message is already safe — it never contains provider internals —
 * but it is written for a developer reading a log. These are written for
 * somebody holding a phone who wants to know whether to try again.
 */
export function friendlyError(code: string, fallback: string): string {
  switch (code) {
    case "no_model_provider":
      return "The assistant has no model configured yet.";
    case "provider_rate_limited":
      return "The assistant is busy. Try again in a moment.";
    case "transcription_rate_limited":
      return "Too many requests right now. Try again in a moment.";
    case "transcription_error":
      return "Couldn't make out the audio. Try again.";
    case "provider_timeout":
      return "That took too long. Try again.";
    case "provider_unavailable":
      return "Couldn't reach the assistant. Try again.";
    case "provider_auth_failed":
      return "The assistant isn't set up correctly on the server.";
    case "provider_refused":
      return "The assistant declined to answer that.";
    case "context_error":
      return "Couldn't load this conversation.";
    case "invalid_input":
      return "There was nothing to send.";
    case "iteration_limit_exceeded":
      return "The assistant got stuck and stopped.";
    default:
      return fallback;
  }
}
