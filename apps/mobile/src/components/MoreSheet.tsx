/**
 * Navigation Drawer / More Sheet.
 *
 * Reduced to core requested tools: Voice Assistant, Notes, Tasks, Documents,
 * Classroom, Calendar, Gmail, and Connected Accounts.
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
import IconButton from "@mui/material/IconButton";
import DescriptionOutlinedIcon from "@mui/icons-material/DescriptionOutlined";
import ChecklistRoundedIcon from "@mui/icons-material/ChecklistRounded";
import EventRoundedIcon from "@mui/icons-material/EventRounded";
import MailOutlineRoundedIcon from "@mui/icons-material/MailOutlineRounded";
import GoogleIcon from "@mui/icons-material/Google";
import SchoolOutlinedIcon from "@mui/icons-material/SchoolOutlined";
import GraphicEqRoundedIcon from "@mui/icons-material/GraphicEqRounded";
import CloseRoundedIcon from "@mui/icons-material/CloseRounded";

import {
  getServerBaseUrl,
  type ConnectionState,
} from "../api/bridge";
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
      icon: <GraphicEqRoundedIcon sx={{ color: "#7C3AED" }} />,
      label: "Voice Assistant",
      note: "Primary voice interface & status",
      enabled: true,
      screen: "home" as ScreenType,
    },
    {
      icon: <DescriptionOutlinedIcon sx={{ color: "#2563EB" }} />,
      label: "Notes",
      note: "Apple Notes-inspired auto-saving workspace",
      enabled: true,
      screen: "notes" as ScreenType,
    },
    {
      icon: <ChecklistRoundedIcon sx={{ color: "#DC4C3E" }} />,
      label: "Tasks",
      note: "Standalone task management & priorities",
      enabled: true,
      screen: "tasks" as ScreenType,
    },
    {
      icon: <SchoolOutlinedIcon sx={{ color: "#0F9D58" }} />,
      label: "Classroom",
      note: "Courses, coursework & announcements",
      enabled: true,
      screen: "classroom" as ScreenType,
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
            maxHeight: "85vh",
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
          mb: 1,
        }}
      />

      {/* Header bar with title and close button */}
      <Box
        sx={{
          px: 2,
          pb: 1,
          display: "flex",
          alignItems: "center",
          justifyContent: "space-between",
        }}
      >
        <Box sx={{ display: "flex", alignItems: "center", gap: 1.25 }}>
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
        <IconButton size="small" onClick={onClose} aria-label="Close drawer">
          <CloseRoundedIcon sx={{ fontSize: 20, color: "#666666" }} />
        </IconButton>
      </Box>

      <List sx={{ px: 1, pb: 1, overflowY: "auto" }}>
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
              bgcolor: "transparent",
              "&:hover": {
                bgcolor: "#F5F5F5",
              },
            }}
          >
            <ListItemIcon sx={{ minWidth: 40 }}>{feature.icon}</ListItemIcon>
            <ListItemText
              primary={
                <Typography
                  variant="body2"
                  sx={{
                    fontWeight: 600,
                    color: "#202020",
                  }}
                >
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

      {/* Backend Connection Status & Server URL Config */}
      <Box sx={{ px: 2.5, py: 1.5 }}>
        <Box
          sx={{
            display: "flex",
            alignItems: "center",
            justifyContent: "space-between",
            mb: 0.5,
          }}
        >
          <Typography variant="caption" sx={{ color: "#888888", fontWeight: 600 }}>
            Server: {getServerBaseUrl()}
          </Typography>
          <Chip
            size="small"
            label={connection.kind}
            color={connection.healthy ? "success" : "default"}
            variant="outlined"
            sx={{ height: 20, fontSize: "0.7rem", fontWeight: 600 }}
          />
        </Box>

        <Typography variant="caption" sx={{ color: "#AAAAAA", fontSize: "0.7rem", display: "block", mt: 0.5 }}>
          {connection.detail}
        </Typography>
      </Box>
    </Drawer>
  );
}

