/**
 * Todoist Quick Add Sheet / Bottom Bar Component.
 *
 * Recreates the exact Todoist quick task assignment interface:
 * - Floating bottom sheet above keyboard
 * - "Task name" input with red accent cursor
 * - Quick action chips: [+] [Inbox / Project] [Date] [Priority] [Attachment]
 * - Signature Todoist Red submit button with wave/send icon
 */

import { useState, useRef, useEffect } from "react";
import Box from "@mui/material/Box";
import InputBase from "@mui/material/InputBase";
import IconButton from "@mui/material/IconButton";
import Menu from "@mui/material/Menu";
import MenuItem from "@mui/material/MenuItem";
import Typography from "@mui/material/Typography";
import AddRoundedIcon from "@mui/icons-material/AddRounded";
import InboxRoundedIcon from "@mui/icons-material/InboxRounded";
import CalendarTodayRoundedIcon from "@mui/icons-material/CalendarTodayRounded";
import AttachFileRoundedIcon from "@mui/icons-material/AttachFileRounded";
import FlagRoundedIcon from "@mui/icons-material/FlagRounded";
import SendRoundedIcon from "@mui/icons-material/SendRounded";
import { PRIORITY_COLORS } from "../lib/priority";

interface TodoistQuickAddProps {
  open: boolean;
  onClose: () => void;
  onAddTask: (task: {
    title: string;
    description?: string;
    priority: string;
    due_at?: string;
    project: string;
  }) => Promise<void>;
  defaultProject?: string;
}

export default function TodoistQuickAdd({
  open,
  onClose,
  onAddTask,
  defaultProject = "Inbox",
}: TodoistQuickAddProps) {
  const [title, setTitle] = useState("");
  const [description, setDescription] = useState("");
  const [showDesc, setShowDesc] = useState(false);
  const [project, setProject] = useState(defaultProject);
  const [priority, setPriority] = useState("P4");
  const [dueDateLabel, setDueDateLabel] = useState<string | null>(null);
  const [dueAtIso, setDueAtIso] = useState<string | null>(null);

  const [projectAnchor, setProjectAnchor] = useState<null | HTMLElement>(null);
  const [dateAnchor, setDateAnchor] = useState<null | HTMLElement>(null);
  const [priorityAnchor, setPriorityAnchor] = useState<null | HTMLElement>(null);

  const inputRef = useRef<HTMLInputElement>(null);

  useEffect(() => {
    let cancelled = false;
    if (open) {
      const timer = setTimeout(() => {
        if (!cancelled) inputRef.current?.focus();
      }, 100);
      return () => {
        cancelled = true;
        clearTimeout(timer);
      };
    }
  }, [open]);

  if (!open) return null;

  const resetForm = () => {
    setTitle("");
    setDescription("");
    setShowDesc(false);
    setProject(defaultProject);
    setPriority("P4");
    setDueDateLabel(null);
    setDueAtIso(null);
  };

  const handleClose = () => {
    resetForm();
    onClose();
  };

  const handleSubmit = async () => {
    if (!title.trim()) return;
    const taskTitle = title.trim();
    const taskDesc = description.trim();
    const taskProj = project;
    const taskPrio = priority;
    const taskDue = dueAtIso || undefined;

    // Reset fields for rapid consecutive entry
    setTitle("");
    setDescription("");
    setShowDesc(false);

    await onAddTask({
      title: taskTitle,
      description: taskDesc || undefined,
      priority: taskPrio,
      due_at: taskDue,
      project: taskProj,
    });
  };

  const handleSetDateOption = (label: string | null, iso: string | null) => {
    setDueDateLabel(label);
    setDueAtIso(iso);
    setDateAnchor(null);
  };

  return (
    <Box
      sx={{
        position: "fixed",
        inset: 0,
        zIndex: 1300,
        display: "flex",
        flexDirection: "column",
        justifyContent: "flex-end",
        bgcolor: "rgba(0, 0, 0, 0.4)",
        animation: "fadeIn 0.15s ease-out",
      }}
      onClick={(e) => {
        if (e.target === e.currentTarget) handleClose();
      }}
    >
      {/* Quick Add Card anchored above keyboard */}
      <Box
        sx={{
          bgcolor: "#212121",
          color: "#E8E8E8",
          borderTopLeftRadius: 20,
          borderTopRightRadius: 20,
          p: 2,
          pb: `calc(12px + env(safe-area-inset-bottom))`,
          boxShadow: "0 -8px 32px rgba(0,0,0,0.5)",
          display: "flex",
          flexDirection: "column",
          gap: 1.5,
        }}
      >
        {/* Task Title Input */}
        <InputBase
          inputRef={inputRef}
          placeholder="Task name"
          value={title}
          onChange={(e) => setTitle(e.target.value)}
          onKeyDown={(e) => {
            if (e.key === "Enter" && !e.shiftKey) {
              e.preventDefault();
              void handleSubmit();
            }
          }}
          multiline
          maxRows={3}
          sx={{
            color: "#FFFFFF",
            fontSize: "1.1rem",
            fontWeight: 500,
            "& .MuiInputBase-input": {
              caretColor: "#DC4C3E",
              p: 0.5,
            },
            "& .MuiInputBase-input::placeholder": {
              color: "#808080",
              opacity: 1,
            },
          }}
        />

        {/* Optional Description Input */}
        {showDesc && (
          <InputBase
            placeholder="Description"
            value={description}
            onChange={(e) => setDescription(e.target.value)}
            multiline
            maxRows={2}
            sx={{
              color: "#A0A0A0",
              fontSize: "0.9rem",
              px: 0.5,
              "& .MuiInputBase-input::placeholder": {
                color: "#606060",
                opacity: 1,
              },
            }}
          />
        )}

        {/* Quick Action Buttons Row */}
        <Box
          sx={{
            display: "flex",
            alignItems: "center",
            gap: 1,
            overflowX: "auto",
            pt: 0.5,
            "::-webkit-scrollbar": { display: "none" },
          }}
        >
          {/* [+] Description Toggle */}
          <IconButton
            size="small"
            onClick={() => setShowDesc(!showDesc)}
            sx={{
              bgcolor: showDesc ? "rgba(220, 76, 62, 0.2)" : "#333333",
              color: showDesc ? "#DC4C3E" : "#A0A0A0",
              borderRadius: "10px",
              width: 36,
              height: 36,
              "&:hover": { bgcolor: "#444444" },
            }}
          >
            <AddRoundedIcon sx={{ fontSize: 20 }} />
          </IconButton>

          {/* [Inbox] Project Selector Chip */}
          <Box
            onClick={(e) => setProjectAnchor(e.currentTarget)}
            sx={{
              display: "flex",
              alignItems: "center",
              gap: 0.75,
              bgcolor: "#333333",
              color: "#E8E8E8",
              px: 1.5,
              py: 0.75,
              borderRadius: "10px",
              cursor: "pointer",
              fontSize: "0.85rem",
              fontWeight: 500,
              userSelect: "none",
              "&:hover": { bgcolor: "#444444" },
            }}
          >
            <InboxRoundedIcon sx={{ fontSize: 18, color: "#A0A0A0" }} />
            <Typography variant="body2" sx={{ fontSize: "0.85rem", fontWeight: 500 }}>
              {project}
            </Typography>
          </Box>

          {/* [Date] Calendar Selector Chip */}
          <Box
            onClick={(e) => setDateAnchor(e.currentTarget)}
            sx={{
              display: "flex",
              alignItems: "center",
              gap: 0.75,
              bgcolor: "#333333",
              color: dueDateLabel ? "#DC4C3E" : "#E8E8E8",
              px: 1.5,
              py: 0.75,
              borderRadius: "10px",
              cursor: "pointer",
              fontSize: "0.85rem",
              fontWeight: 500,
              userSelect: "none",
              "&:hover": { bgcolor: "#444444" },
            }}
          >
            <CalendarTodayRoundedIcon sx={{ fontSize: 16, color: dueDateLabel ? "#DC4C3E" : "#A0A0A0" }} />
            <Typography variant="body2" sx={{ fontSize: "0.85rem", fontWeight: 500 }}>
              {dueDateLabel || "Date"}
            </Typography>
          </Box>

          {/* [Priority] Flag Chip */}
          <IconButton
            size="small"
            onClick={(e) => setPriorityAnchor(e.currentTarget)}
            sx={{
              bgcolor: "#333333",
              color: PRIORITY_COLORS[priority as keyof typeof PRIORITY_COLORS] || "#A0A0A0",
              borderRadius: "10px",
              width: 36,
              height: 36,
              "&:hover": { bgcolor: "#444444" },
            }}
          >
            <FlagRoundedIcon sx={{ fontSize: 18 }} />
          </IconButton>

          {/* [Attachment] Chip */}
          <Box
            sx={{
              display: "flex",
              alignItems: "center",
              gap: 0.75,
              bgcolor: "#333333",
              color: "#E8E8E8",
              px: 1.5,
              py: 0.75,
              borderRadius: "10px",
              cursor: "pointer",
              fontSize: "0.85rem",
              fontWeight: 500,
              userSelect: "none",
              "&:hover": { bgcolor: "#444444" },
            }}
          >
            <AttachFileRoundedIcon sx={{ fontSize: 16, color: "#A0A0A0" }} />
            <Typography variant="body2" sx={{ fontSize: "0.85rem", fontWeight: 500 }}>
              Attachment
            </Typography>
          </Box>

          {/* Spacer */}
          <Box sx={{ flexGrow: 1 }} />

          {/* Todoist Red Submit Action Button */}
          <IconButton
            onClick={() => void handleSubmit()}
            disabled={!title.trim()}
            sx={{
              bgcolor: title.trim() ? "#DC4C3E" : "#444444",
              color: "#FFFFFF",
              borderRadius: "14px",
              width: 42,
              height: 42,
              boxShadow: title.trim() ? "0 4px 14px rgba(220, 76, 62, 0.4)" : "none",
              transition: "all 0.15s ease-in-out",
              "&:hover": {
                bgcolor: title.trim() ? "#B9382B" : "#444444",
              },
              "&.Mui-disabled": {
                bgcolor: "#333333",
                color: "#606060",
              },
            }}
          >
            <SendRoundedIcon sx={{ fontSize: 20 }} />
          </IconButton>
        </Box>

        {/* Project Selector Menu */}
        <Menu
          anchorEl={projectAnchor}
          open={Boolean(projectAnchor)}
          onClose={() => setProjectAnchor(null)}
          slotProps={{ paper: { sx: { bgcolor: "#2A2A2A", color: "#FFF", borderRadius: 2 } } }}
        >
          {["Inbox", "Personal", "Work", "Ideas"].map((p) => (
            <MenuItem
              key={p}
              selected={project === p}
              onClick={() => {
                setProject(p);
                setProjectAnchor(null);
              }}
            >
              #{p}
            </MenuItem>
          ))}
        </Menu>

        {/* Date Selector Menu */}
        <Menu
          anchorEl={dateAnchor}
          open={Boolean(dateAnchor)}
          onClose={() => setDateAnchor(null)}
          slotProps={{ paper: { sx: { bgcolor: "#2A2A2A", color: "#FFF", borderRadius: 2 } } }}
        >
          <MenuItem
            onClick={() =>
              handleSetDateOption("Today", new Date().toISOString())
            }
          >
            Today
          </MenuItem>
          <MenuItem
            onClick={() => {
              const tomorrow = new Date();
              tomorrow.setDate(tomorrow.getDate() + 1);
              handleSetDateOption("Tomorrow", tomorrow.toISOString());
            }}
          >
            Tomorrow
          </MenuItem>
          <MenuItem
            onClick={() => {
              const nextWeek = new Date();
              nextWeek.setDate(nextWeek.getDate() + 7);
              handleSetDateOption("Next week", nextWeek.toISOString());
            }}
          >
            Next week
          </MenuItem>
          <MenuItem onClick={() => handleSetDateOption(null, null)}>
            No due date
          </MenuItem>
        </Menu>

        {/* Priority Selector Menu */}
        <Menu
          anchorEl={priorityAnchor}
          open={Boolean(priorityAnchor)}
          onClose={() => setPriorityAnchor(null)}
          slotProps={{ paper: { sx: { bgcolor: "#2A2A2A", color: "#FFF", borderRadius: 2 } } }}
        >
          <MenuItem
            onClick={() => {
              setPriority("P1");
              setPriorityAnchor(null);
            }}
            sx={{ color: PRIORITY_COLORS.P1, fontWeight: 700 }}
          >
            P1 — Priority 1 (Red)
          </MenuItem>
          <MenuItem
            onClick={() => {
              setPriority("P2");
              setPriorityAnchor(null);
            }}
            sx={{ color: PRIORITY_COLORS.P2, fontWeight: 700 }}
          >
            P2 — Priority 2 (Orange)
          </MenuItem>
          <MenuItem
            onClick={() => {
              setPriority("P3");
              setPriorityAnchor(null);
            }}
            sx={{ color: PRIORITY_COLORS.P3, fontWeight: 700 }}
          >
            P3 — Priority 3 (Blue)
          </MenuItem>
          <MenuItem
            onClick={() => {
              setPriority("P4");
              setPriorityAnchor(null);
            }}
            sx={{ color: PRIORITY_COLORS.P4, fontWeight: 700 }}
          >
            P4 — Priority 4 (Grey)
          </MenuItem>
        </Menu>
      </Box>
    </Box>
  );
}
