/**
 * The one place everything that is not voice lives.
 *
 * It is a sheet rather than a nav bar on purpose: a permanent bar advertises
 * four destinations at all times, which is four decisions the user did not ask
 * to make. Here the surface is empty until it is summoned, and it closes again.
 *
 * Entries for unbuilt features are shown disabled with the reason. Hiding them
 * would misrepresent the roadmap; enabling them would misrepresent progress.
 */

import Box from "@mui/material/Box";
import Chip from "@mui/material/Chip";
import Divider from "@mui/material/Divider";
import Drawer from "@mui/material/Drawer";
import List from "@mui/material/List";
import ListItemButton from "@mui/material/ListItemButton";
import ListItemIcon from "@mui/material/ListItemIcon";
import ListItemText from "@mui/material/ListItemText";
import Typography from "@mui/material/Typography";
import ChecklistRoundedIcon from "@mui/icons-material/ChecklistRounded";
import PsychologyRoundedIcon from "@mui/icons-material/PsychologyRounded";
import EventRoundedIcon from "@mui/icons-material/EventRounded";
import NotificationsRoundedIcon from "@mui/icons-material/NotificationsRounded";
import LinkRoundedIcon from "@mui/icons-material/LinkRounded";
import SettingsRoundedIcon from "@mui/icons-material/SettingsRounded";

import type { ConnectionState } from "../api/bridge";

const FEATURES = [
  { icon: <ChecklistRoundedIcon />, label: "Tasks", note: "schedule milestone" },
  { icon: <EventRoundedIcon />, label: "Calendar", note: "schedule milestone" },
  { icon: <PsychologyRoundedIcon />, label: "Memory", note: "memory milestone" },
  { icon: <NotificationsRoundedIcon />, label: "Notifications", note: "attention milestone" },
  { icon: <LinkRoundedIcon />, label: "Connections", note: "integrations milestone" },
  { icon: <SettingsRoundedIcon />, label: "Settings", note: "not built yet" },
];

export default function MoreSheet({
  open,
  onClose,
  connection,
}: {
  open: boolean;
  onClose: () => void;
  connection: ConnectionState;
}) {
  return (
    <Drawer
      anchor="bottom"
      open={open}
      onClose={onClose}
      slotProps={{
        paper: {
          sx: {
            borderTopLeftRadius: 20,
            borderTopRightRadius: 20,
            pb: "env(safe-area-inset-bottom)",
          },
        },
      }}
    >
      {/* Grab handle: the only affordance needed to say "drag or tap away". */}
      <Box
        sx={{
          width: 36,
          height: 4,
          borderRadius: 2,
          bgcolor: "divider",
          mx: "auto",
          mt: 1.5,
          mb: 1.5,
        }}
      />

      <Box sx={{ px: 2, pb: 1, display: "flex", alignItems: "center", gap: 1.25 }}>
        <Box
          component="img"
          src="/logo.png"
          alt="Logo"
          sx={{ width: 28, height: 28, borderRadius: "50%", objectFit: "contain" }}
        />
        <Typography variant="subtitle1" color="text.primary" sx={{ fontWeight: 700 }}>
          Personal Assistant
        </Typography>
      </Box>

      <List sx={{ px: 1, pb: 1 }}>
        {FEATURES.map((feature) => (
          <ListItemButton key={feature.label} disabled sx={{ borderRadius: 2 }}>
            <ListItemIcon sx={{ minWidth: 40 }}>{feature.icon}</ListItemIcon>
            <ListItemText primary={feature.label} secondary={feature.note} />
          </ListItemButton>
        ))}
      </List>

      <Divider />

      <Box sx={{ px: 3, py: 2, display: "flex", alignItems: "center", gap: 1.5 }}>
        <Chip
          size="small"
          variant={connection.kind === "connected" ? "filled" : "outlined"}
          color={
            connection.kind === "connected"
              ? connection.healthy
                ? "success"
                : "warning"
              : "error"
          }
          label={connection.kind === "connected" ? "Connected" : "Offline"}
        />
        <Typography variant="caption" color="text.secondary" sx={{ minWidth: 0 }}>
          {connection.detail}
        </Typography>
      </Box>
    </Drawer>
  );
}
