/**
 * Home.
 *
 * One thing to do, one place to go. The microphone is the interface; everything
 * else is behind a single small button. Status is a four-pixel dot rather than a
 * card, because connection state is only interesting when it is wrong — and when
 * it is wrong, the dot turns red and the sheet explains why.
 *
 * Deliberately absent: cards, counters, lists, a nav bar. They arrive when there
 * is real information to put in them, not before.
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

export default function HomeScreen() {
  const [connection, setConnection] = useState<ConnectionState>(CHECKING);
  const [sheetOpen, setSheetOpen] = useState(false);
  const [activeVoiceState, setActiveVoiceState] = useState<OrbState | null>(null);

  useEffect(() => {
    let cancelled = false;
    void loadConnection().then((next) => {
      if (!cancelled) setConnection(next);
    });
    return () => {
      cancelled = true;
    };
  }, []);

  const dotColor =
    connection.kind === "checking"
      ? "text.disabled"
      : connection.kind === "offline"
        ? "error.main"
        : connection.healthy
          ? "success.main"
          : "warning.main";

  // Effective state: user state override > connection status
  const currentOrbState: OrbState =
    activeVoiceState ??
    (connection.kind === "checking"
      ? "connecting"
      : connection.kind === "offline"
        ? "error"
        : "idle");

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
        return "Listening… Tap to process";
      case "thinking":
        return "Thinking… Tap to answer";
      case "speaking":
        return "Speaking… Tap to stop";
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
            sx={{ width: 28, height: 28, borderRadius: "50%", objectFit: "contain" }}
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
          gap: 3,
        }}
      >
        <Box
          onClick={handleOrbClick}
          sx={{
            p: 2.5,
            borderRadius: "50%",
            cursor: connection.kind === "offline" ? "not-allowed" : "pointer",
            position: "relative",
            display: "flex",
            justifyContent: "center",
            alignItems: "center",
            transition: "transform 0.25s cubic-bezier(0.4, 0, 0.2, 1), box-shadow 0.3s ease",
            boxShadow:
              currentOrbState === "listening"
                ? "0 0 50px rgba(239, 68, 68, 0.4), 0 0 0 16px rgba(239, 68, 68, 0.12)"
                : currentOrbState === "thinking"
                  ? "0 0 45px rgba(220, 38, 38, 0.35), 0 0 0 12px rgba(220, 38, 38, 0.1)"
                  : currentOrbState === "speaking"
                    ? "0 0 45px rgba(248, 113, 113, 0.35), 0 0 0 14px rgba(248, 113, 113, 0.12)"
                    : "0 0 35px rgba(239, 68, 68, 0.15)",
            "&:hover": {
              transform: "scale(1.04)",
            },
            "&:active": {
              transform: "scale(0.97)",
            },
          }}
        >
          <Orb theme="cloud" size={200} state={currentOrbState} />
        </Box>

        <Box sx={{ textAlign: "center", display: "flex", flexDirection: "column", gap: 1 }}>
          <Typography
            variant="body2"
            color={currentOrbState === "idle" ? "text.secondary" : "error.main"}
            sx={{ fontWeight: 600 }}
          >
            {getStatusText()}
          </Typography>
        </Box>
      </Box>

      {/* Footer Drawer Button */}
      <IconButton
        aria-label="Everything else"
        onClick={() => setSheetOpen(true)}
        sx={{ mb: 3, color: "text.secondary" }}
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
