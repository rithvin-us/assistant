/**
 * HomeScreen — Immersive Voice Interface.
 *
 * Features:
 * - Perfectly centered 1:1 voice orb with breathing aura rings.
 * - Zero tap highlight / blue selection box.
 * - Dynamic, playful Claude-style process status messages ("Clauding...", "Brewing thoughts...", "Flabbergasting...", etc.).
 */

import { useState, useEffect } from "react";
import Box from "@mui/material/Box";
import IconButton from "@mui/material/IconButton";
import Tooltip from "@mui/material/Tooltip";
import Typography from "@mui/material/Typography";
import MoreHorizRoundedIcon from "@mui/icons-material/MoreHorizRounded";
import { Orb, type OrbState } from "orb-ui";

import { loadConnection, type ConnectionState } from "../api/bridge";
import MoreSheet from "../components/MoreSheet";

const CHECKING: ConnectionState = {
  kind: "checking",
  healthy: false,
  detail: "Checking…",
};

const CLAUDE_THINKING_MESSAGES = [
  "Clauding…",
  "Brewing thoughts…",
  "Flabbergasting…",
  "Pondering cosmic truths…",
  "Consulting the neural oracle…",
  "Synergizing synapses…",
  "Percolating response…",
  "Assembling insights…",
  "Weaving context…",
  "Crunching tokens…",
  "Disentangling paradoxes…",
  "Summoning wisdom…",
  "Untangling complexity…",
  "Calibrating brilliance…",
  "Baking response…",
];

const LISTENING_MESSAGES = [
  "Listening closely…",
  "Absorbing your thoughts…",
  "Harkening…",
  "Capturing voice waves…",
];

const SPEAKING_MESSAGES = [
  "Articulating…",
  "Sharing wisdom…",
  "Vocalizing answer…",
  "Transmitting thoughts…",
];

export default function HomeScreen() {
  const [connection, setConnection] = useState<ConnectionState>(CHECKING);
  const [sheetOpen, setSheetOpen] = useState(false);
  const [activeVoiceState, setActiveVoiceState] = useState<OrbState | null>(null);
  const [thinkingIndex, setThinkingIndex] = useState(0);
  const [listeningIndex, setListeningIndex] = useState(0);
  const [speakingIndex, setSpeakingIndex] = useState(0);

  useEffect(() => {
    let cancelled = false;
    void loadConnection().then((next) => {
      if (!cancelled) setConnection(next);
    });
    return () => {
      cancelled = true;
    };
  }, []);

  // Effective state: user state override > connection status
  const currentOrbState: OrbState =
    activeVoiceState ??
    (connection.kind === "checking"
      ? "connecting"
      : connection.kind === "offline"
        ? "error"
        : "idle");

  // Cycle thinking messages dynamically
  useEffect(() => {
    if (currentOrbState !== "thinking") return;
    const interval = setInterval(() => {
      setThinkingIndex((prev) => (prev + 1) % CLAUDE_THINKING_MESSAGES.length);
    }, 2200);
    return () => clearInterval(interval);
  }, [currentOrbState]);

  // Cycle listening messages dynamically
  useEffect(() => {
    if (currentOrbState !== "listening") return;
    const interval = setInterval(() => {
      setListeningIndex((prev) => (prev + 1) % LISTENING_MESSAGES.length);
    }, 3000);
    return () => clearInterval(interval);
  }, [currentOrbState]);

  // Cycle speaking messages dynamically
  useEffect(() => {
    if (currentOrbState !== "speaking") return;
    const interval = setInterval(() => {
      setSpeakingIndex((prev) => (prev + 1) % SPEAKING_MESSAGES.length);
    }, 2500);
    return () => clearInterval(interval);
  }, [currentOrbState]);

  const dotColor =
    connection.kind === "checking"
      ? "text.disabled"
      : connection.kind === "offline"
        ? "error.main"
        : connection.healthy
          ? "success.main"
          : "warning.main";

  const handleOrbClick = () => {
    if (connection.kind === "offline") return;
    setActiveVoiceState((prev) => {
      if (!prev || prev === "idle") return "listening";
      if (prev === "listening") return "thinking";
      if (prev === "thinking") return "speaking";
      return "idle";
    });
  };

  const getStatusText = () => {
    switch (currentOrbState) {
      case "connecting":
        return "Connecting to server…";
      case "listening":
        return `${LISTENING_MESSAGES[listeningIndex]} (Tap to process)`;
      case "thinking":
        return `${CLAUDE_THINKING_MESSAGES[thinkingIndex]}`;
      case "speaking":
        return `${SPEAKING_MESSAGES[speakingIndex]} (Tap to stop)`;
      case "error":
        return "Server unreachable";
      case "idle":
      default:
        return "Tap to talk";
    }
  };

  return (
    <Box
      sx={{
        height: "100%",
        display: "grid",
        gridTemplateRows: "auto 1fr auto",
        justifyItems: "center",
        WebkitTapHighlightColor: "transparent",
        userSelect: "none",
        WebkitUserSelect: "none",
        px: 2,
      }}
    >
      {/* Header */}
      <Box
        sx={{
          width: "100%",
          display: "flex",
          justifyContent: "space-between",
          alignItems: "center",
          pt: 1.5,
          px: 0.5,
        }}
      >
        <Box sx={{ display: "flex", alignItems: "center", gap: 1 }}>
          <Box
            component="img"
            src="/logo.png"
            alt="Assistant Logo"
            sx={{ width: 28, height: 28, borderRadius: "50%", objectFit: "cover" }}
          />
          <Typography variant="h6" color="text.primary" sx={{ fontSize: "1rem", fontWeight: 700 }}>
            Assistant
          </Typography>
        </Box>
        <Tooltip title={connection.detail}>
          <Box
            aria-label={`Server ${connection.kind}`}
            sx={{ width: 8, height: 8, borderRadius: "50%", bgcolor: dotColor }}
          />
        </Tooltip>
      </Box>

      {/* Main Voice Orb Container */}
      <Box
        sx={{
          alignSelf: "center",
          display: "flex",
          flexDirection: "column",
          alignItems: "center",
          gap: 3.5,
          width: "100%",
        }}
      >
        <Box
          onClick={handleOrbClick}
          sx={{
            width: 250,
            height: 250,
            borderRadius: "50%",
            cursor: connection.kind === "offline" ? "not-allowed" : "pointer",
            position: "relative",
            display: "flex",
            justifyContent: "center",
            alignItems: "center",
            WebkitTapHighlightColor: "transparent",
            outline: "none",
            border: "none",
            transition: "all 0.3s cubic-bezier(0.4, 0, 0.2, 1)",
            boxShadow:
              currentOrbState === "listening"
                ? "0 0 60px rgba(59, 130, 246, 0.45), 0 0 0 20px rgba(59, 130, 246, 0.12)"
                : currentOrbState === "thinking"
                  ? "0 0 65px rgba(147, 51, 234, 0.45), 0 0 0 22px rgba(147, 51, 234, 0.14)"
                  : currentOrbState === "speaking"
                    ? "0 0 60px rgba(16, 185, 129, 0.45), 0 0 0 20px rgba(16, 185, 129, 0.12)"
                    : "0 0 40px rgba(59, 130, 246, 0.12)",
            "&:hover": {
              transform: "scale(1.03)",
            },
            "&:active": {
              transform: "scale(0.96)",
              WebkitTapHighlightColor: "transparent",
            },
          }}
        >
          <Box
            sx={{
              width: "100%",
              height: "100%",
              display: "flex",
              justifyContent: "center",
              alignItems: "center",
              borderRadius: "50%",
              overflow: "hidden",
            }}
          >
            <Orb theme="cloud" size={240} state={currentOrbState} />
          </Box>
        </Box>

        <Box sx={{ textAlign: "center", minHeight: 48, display: "flex", alignItems: "center" }}>
          <Typography
            variant="body1"
            color={
              currentOrbState === "thinking"
                ? "primary.main"
                : currentOrbState === "listening"
                  ? "info.main"
                  : currentOrbState === "speaking"
                    ? "success.main"
                    : "text.secondary"
            }
            sx={{
              fontWeight: 600,
              fontSize: "0.95rem",
              letterSpacing: "0.01em",
              transition: "all 0.25s ease-in-out",
            }}
          >
            {getStatusText()}
          </Typography>
        </Box>
      </Box>

      {/* Footer Drawer Button */}
      <IconButton
        aria-label="Everything else"
        onClick={() => setSheetOpen(true)}
        sx={{
          mb: 3,
          color: "text.secondary",
          WebkitTapHighlightColor: "transparent",
          outline: "none",
        }}
      >
        <MoreHorizRoundedIcon />
      </IconButton>

      <MoreSheet
        open={sheetOpen}
        onClose={() => setSheetOpen(false)}
        connection={connection}
      />
    </Box>
  );
}
