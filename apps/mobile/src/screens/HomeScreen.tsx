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

import { useEffect, useState } from "react";
import Box from "@mui/material/Box";
import Fab from "@mui/material/Fab";
import IconButton from "@mui/material/IconButton";
import Tooltip from "@mui/material/Tooltip";
import Typography from "@mui/material/Typography";
import MicRoundedIcon from "@mui/icons-material/MicRounded";
import MoreHorizRoundedIcon from "@mui/icons-material/MoreHorizRounded";

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

  // State is written only from the promise callback, never synchronously in the
  // effect body; the flag stops a late response reaching an unmounted component.
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

  return (
    <Box
      sx={{
        height: "100%",
        display: "grid",
        // Three rows: a near-empty header, the voice button centred in whatever
        // space is left, and a single small control at the bottom.
        gridTemplateRows: "auto 1fr auto",
        justifyItems: "center",
      }}
    >
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

      <Box
        sx={{
          alignSelf: "center",
          display: "flex",
          flexDirection: "column",
          alignItems: "center",
          gap: 2.5,
        }}
      >
        <Fab
          color="primary"
          aria-label="Hold to talk"
          disabled
          sx={{
            width: 132,
            height: 132,
            // A wide, very soft ring instead of a shadow: it reads as presence
            // rather than as elevation, and survives the dark ground.
            boxShadow: (t) => `0 0 0 12px ${t.palette.primary.main}14`,
            "& svg": { fontSize: 48 },
          }}
        >
          <MicRoundedIcon />
        </Fab>

        <Typography variant="body2" color="text.secondary">
          Voice is not built yet
        </Typography>
      </Box>

      <IconButton
        aria-label="Everything else"
        onClick={() => setSheetOpen(true)}
        sx={{ mb: 3, color: "text.disabled" }}
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
