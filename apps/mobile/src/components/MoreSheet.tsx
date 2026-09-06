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
import DescriptionOutlinedIcon from "@mui/icons-material/DescriptionOutlined";
import ChecklistRoundedIcon from "@mui/icons-material/ChecklistRounded";

import type { ConnectionState } from "../api/bridge";
import type { ScreenType } from "../App";

export default function MoreSheet({
  open,
  onClose,
  connection,
  onSelectScreen,
}: {
  open: boolean;
  onClose: () => void;
  connection: ConnectionState;
  onSelectScreen: (screen: ScreenType) => void;
}) {
  const FEATURES = [
    {
      icon: <ChecklistRoundedIcon sx={{ color: "#DC4C3E" }} />,
      label: "Tasks",
      note: "Standalone task management & priorities",
      enabled: true,
      screen: "tasks" as ScreenType,
    },
    {
      icon: <DescriptionOutlinedIcon sx={{ color: "#2563EB" }} />,
      label: "Notes",
      note: "Auto-saving notes & tags",
      enabled: true,
      screen: "notes" as ScreenType,
    },
  ];

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
      {/* Grab handle */}
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
          <ListItemButton
            key={feature.label}
            disabled={!feature.enabled}
            onClick={() => {
              if (feature.enabled && feature.screen) {
                onSelectScreen(feature.screen);
                onClose();
              }
            }}
            sx={{ borderRadius: 2 }}
          >
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

