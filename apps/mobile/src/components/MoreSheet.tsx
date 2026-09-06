/**
 * Navigation Drawer / More Sheet.
 *
 * Provides access to productivity tools and Google integrations.
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
import EventRoundedIcon from "@mui/icons-material/EventRounded";
import MailOutlineRoundedIcon from "@mui/icons-material/MailOutlineRounded";
import GoogleIcon from "@mui/icons-material/Google";

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
    {
      icon: <EventRoundedIcon sx={{ color: "#1A73E8" }} />,
      label: "Calendar",
      note: "Events & free time intervals",
      enabled: true,
      screen: "calendar" as ScreenType,
    },
    {
      icon: <MailOutlineRoundedIcon sx={{ color: "#EA4335" }} />,
      label: "Gmail",
      note: "Search & read messages",
      enabled: true,
      screen: "gmail" as ScreenType,
    },
    {
      icon: <GoogleIcon sx={{ color: "#4285F4" }} />,
      label: "Connected Accounts",
      note: "Multi-account Google integration",
      enabled: true,
      screen: "connections" as ScreenType,
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
            bgcolor: "#FFFFFF",
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
          bgcolor: "#E0E0E0",
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
        <Typography variant="subtitle1" sx={{ fontWeight: 700, color: "#202020" }}>
          Personal Assistant
        </Typography>
      </Box>

      <List sx={{ px: 1, pb: 1 }}>
        {FEATURES.map((feature) => (
          <ListItemButton
            key={feature.label}
            disabled={!feature.enabled}
            onClick={() => {
              onClose();
              onSelectScreen(feature.screen);
            }}
            sx={{
              borderRadius: 2,
              mb: 0.5,
              py: 1,
              "&:hover": { bgcolor: "#F5F5F5" },
            }}
          >
            <ListItemIcon sx={{ minWidth: 40 }}>{feature.icon}</ListItemIcon>
            <ListItemText
              primary={
                <Typography variant="body2" sx={{ fontWeight: 600, color: "#202020" }}>
                  {feature.label}
                </Typography>
              }
              secondary={
                <Typography variant="caption" sx={{ color: "#777777" }}>
                  {feature.note}
                </Typography>
              }
            />
          </ListItemButton>
        ))}
      </List>

      <Divider sx={{ my: 0.5, borderColor: "#F0F0F0" }} />

      {/* Backend Connection Status */}
      <Box
        sx={{
          px: 2.5,
          py: 1.5,
          display: "flex",
          alignItems: "center",
          justifyContent: "space-between",
        }}
      >
        <Typography variant="caption" sx={{ color: "#888888", fontWeight: 500 }}>
          {connection.detail}
        </Typography>
        <Chip
          size="small"
          label={connection.kind}
          color={connection.healthy ? "success" : "default"}
          variant="outlined"
          sx={{ height: 20, fontSize: "0.7rem", fontWeight: 600 }}
        />
      </Box>
    </Drawer>
  );
}
