/**
 * Text conversation.
 *
 * The minimum that makes the assistant usable: type, send, watch the answer
 * arrive, see an error you can act on. It is a sheet rather than a screen
 * because voice is still the intended interface — this is the surface that
 * exists until the microphone does.
 *
 * Deliberately absent: message actions, editing, regeneration, a conversation
 * list, markdown rendering. None of them is needed to hold a conversation, and
 * each would be a thing to redo once voice arrives.
 *
 * Nothing here is a debug console. Frames the user cannot act on — tool
 * proposals, approvals, ids, codes — do not reach the screen; they belong to
 * milestones whose features exist.
 */

import { useCallback, useEffect, useRef, useState } from "react";
import Alert from "@mui/material/Alert";
import Box from "@mui/material/Box";
import CircularProgress from "@mui/material/CircularProgress";
import Drawer from "@mui/material/Drawer";
import IconButton from "@mui/material/IconButton";
import InputAdornment from "@mui/material/InputAdornment";
import TextField from "@mui/material/TextField";
import Typography from "@mui/material/Typography";
import ArrowUpwardRoundedIcon from "@mui/icons-material/ArrowUpwardRounded";

import {
  closeConversation,
  friendlyError,
  openConversation,
  sendUserText,
  subscribe,
  type SocketStatus,
} from "../api/conversation";
import type { ServerFrame } from "../api/types";

interface Message {
  id: string;
  role: "user" | "assistant";
  text: string;
  /** An assistant message whose stream ended before the turn did. */
  interrupted?: boolean;
}

/** Distinguishes "waiting for the first token" from "no turn in flight". */
type TurnState = "idle" | "sending" | "streaming";

export default function ConversationSheet({
  open,
  onClose,
}: {
  open: boolean;
  onClose: () => void;
}) {
  const [messages, setMessages] = useState<Message[]>([]);
  const [draft, setDraft] = useState("");
  const [turn, setTurn] = useState<TurnState>("idle");
  const [error, setError] = useState<string | null>(null);
  const [connected, setConnected] = useState(false);

  /**
   * The conversation id, generated once and kept for the life of the sheet.
   *
   * The server treats it as the conversation's identity and scopes it to the
   * authenticated user, so history follows this id across turns and across a
   * server restart.
   */
  const conversationId = useRef<string>(crypto.randomUUID());
  const bottom = useRef<HTMLDivElement | null>(null);

  /** Appends a delta to the assistant message it belongs to. */
  const applyDelta = useCallback((messageId: string, text: string) => {
    setMessages((current) => {
      const existing = current.findIndex((message) => message.id === messageId);
      if (existing === -1) {
        return [...current, { id: messageId, role: "assistant", text }];
      }
      const next = [...current];
      next[existing] = { ...next[existing], text: next[existing].text + text };
      return next;
    });
  }, []);

  const onFrame = useCallback(
    (frame: ServerFrame) => {
      switch (frame.type) {
        case "assistant_delta":
          setTurn("streaming");
          applyDelta(frame.message_id, frame.text);
          break;

        case "turn_end":
          setTurn("idle");
          break;

        case "error":
          setTurn("idle");
          setError(friendlyError(frame.code, "The assistant couldn't answer. Try again."));
          break;

        // Everything else — ready, pong, tool and approval frames — is either
        // handled by the shell or belongs to a feature that does not exist yet.
        // Showing it would turn this into a protocol inspector.
        default:
          break;
      }
    },
    [applyDelta],
  );

  const onStatus = useCallback((status: SocketStatus) => {
    if (status.state === "open") {
      setConnected(true);
      return;
    }

    setConnected(false);
    setTurn((current) => {
      if (current === "idle") return current;
      // A stream cut short must not look like a finished answer. The text that
      // did arrive is kept — it is what the assistant actually said — and the
      // message is marked so the user knows it stopped early.
      setMessages((messages) =>
        messages.map((message, index) =>
          index === messages.length - 1 && message.role === "assistant"
            ? { ...message, interrupted: true }
            : message,
        ),
      );
      setError("The connection dropped before the answer finished.");
      return "idle";
    });
  }, []);

  // One connection for as long as the sheet is open. Closing it also stops
  // whatever turn was in flight: the server cancels on disconnect, so leaving
  // the screen stops the assistant spending.
  useEffect(() => {
    if (!open) return;

    let unsubscribe: (() => void) | undefined;
    let cancelled = false;

    void (async () => {
      unsubscribe = await subscribe({ onFrame, onStatus });
      if (cancelled) {
        unsubscribe();
        return;
      }
      try {
        await openConversation(conversationId.current);
      } catch (cause) {
        setError(cause instanceof Error ? cause.message : "Couldn't reach the assistant.");
      }
    })();

    return () => {
      cancelled = true;
      unsubscribe?.();
      void closeConversation();
      setConnected(false);
    };
  }, [open, onFrame, onStatus]);

  useEffect(() => {
    bottom.current?.scrollIntoView({ behavior: "smooth" });
  }, [messages, turn]);

  const busy = turn !== "idle";

  async function send() {
    const text = draft.trim();
    // The guard on `busy` is what stops a double tap sending twice: a second
    // turn while one is in flight would be two answers interleaved in one
    // stream, and two model calls paid for.
    if (!text || busy) return;

    setDraft("");
    setError(null);
    setMessages((current) => [
      ...current,
      { id: crypto.randomUUID(), role: "user", text },
    ]);
    setTurn("sending");

    try {
      await sendUserText(text);
    } catch {
      setTurn("idle");
      setError("Couldn't send that. Check your connection.");
    }
  }

  return (
    <Drawer
      anchor="bottom"
      open={open}
      onClose={onClose}
      slotProps={{
        paper: {
          sx: {
            height: "85dvh",
            borderTopLeftRadius: 20,
            borderTopRightRadius: 20,
            display: "flex",
            flexDirection: "column",
            pb: "env(safe-area-inset-bottom)",
          },
        },
      }}
    >
      <Box sx={{ px: 3, pt: 2, pb: 1 }}>
        <Typography variant="subtitle1" sx={{ fontWeight: 700 }}>
          Assistant
        </Typography>
        <Typography variant="caption" color="text.secondary">
          {connected ? "Connected" : "Connecting…"}
        </Typography>
      </Box>

      <Box sx={{ flex: 1, overflowY: "auto", px: 3, py: 1 }}>
        {messages.length === 0 && (
          <Typography variant="body2" color="text.secondary" sx={{ mt: 4, textAlign: "center" }}>
            Ask anything. The assistant remembers this conversation.
          </Typography>
        )}

        {messages.map((message) => (
          <Box
            key={message.id}
            sx={{
              display: "flex",
              justifyContent: message.role === "user" ? "flex-end" : "flex-start",
              mb: 1.5,
            }}
          >
            <Box
              sx={{
                maxWidth: "85%",
                px: 2,
                py: 1.25,
                borderRadius: 3,
                bgcolor: message.role === "user" ? "error.main" : "action.hover",
                color: message.role === "user" ? "common.white" : "text.primary",
              }}
            >
              <Typography variant="body2" sx={{ whiteSpace: "pre-wrap" }}>
                {message.text}
              </Typography>
              {message.interrupted && (
                <Typography
                  variant="caption"
                  sx={{ display: "block", mt: 0.5, opacity: 0.7, fontStyle: "italic" }}
                >
                  Stopped early
                </Typography>
              )}
            </Box>
          </Box>
        ))}

        {turn === "sending" && (
          <Box sx={{ display: "flex", alignItems: "center", gap: 1, mb: 1.5 }}>
            <CircularProgress size={14} color="inherit" />
            <Typography variant="caption" color="text.secondary">
              Thinking…
            </Typography>
          </Box>
        )}

        <div ref={bottom} />
      </Box>

      {error && (
        <Alert severity="error" onClose={() => setError(null)} sx={{ mx: 3, mb: 1 }}>
          {error}
        </Alert>
      )}

      <Box sx={{ px: 3, pb: 2 }}>
        <TextField
          fullWidth
          size="small"
          multiline
          maxRows={4}
          placeholder="Message"
          value={draft}
          onChange={(event) => setDraft(event.target.value)}
          onKeyDown={(event) => {
            if (event.key === "Enter" && !event.shiftKey) {
              event.preventDefault();
              void send();
            }
          }}
          slotProps={{
            input: {
              endAdornment: (
                <InputAdornment position="end">
                  <IconButton
                    aria-label="Send"
                    size="small"
                    color="error"
                    disabled={busy || draft.trim().length === 0}
                    onClick={() => void send()}
                  >
                    <ArrowUpwardRoundedIcon fontSize="small" />
                  </IconButton>
                </InputAdornment>
              ),
              sx: { borderRadius: 6 },
            },
          }}
        />
      </Box>
    </Drawer>
  );
}
