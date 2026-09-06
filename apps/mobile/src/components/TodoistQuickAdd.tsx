/**
 * Todoist Quick Add Sheet / Bottom Bar Component — Authentic Todoist Light Theme.
 *
 * Features:
 * - Floating bottom sheet above keyboard with clean pure white canvas
 * - "Task name" input with red accent cursor
 * - Natural language shorthand typing:
 *     - "p1", "p2", "p3", "p4" -> automatically assigns priority
 *     - "tod", "today" -> automatically sets due date to Today
 *     - "tom", "tomorrow" -> automatically sets due date to Tomorrow
 *     - Automatically parses and attaches flags & date badges
 * - Quick action chips: [+] [Inbox / Project] [Date Flag] [Priority] [Attachment]
 * - Signature Todoist Red submit button with send icon
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

  // Natural language & shorthand parser for task input:
  // e.g. typing "p1", "p2", "p3", "tod", "tom"
  const handleTitleChange = (val: string) => {
    let newTitle = val;

    // Detect priority shorthands: p1, p2, p3, p4 (case-insensitive)
    const prioMatch = newTitle.match(/\b([pP][1-4])\b/);
    if (prioMatch) {
      const matched = prioMatch[1].toUpperCase();
      setPriority(matched);
      newTitle = newTitle.replace(/\b[pP][1-4]\b/, "").replace(/\s{2,}/g, " ");
    }

    // Detect today shorthands: tod, today
    const todayMatch = newTitle.match(/\b(tod|today)\b/i);
    if (todayMatch) {
      setDueDateLabel("Today");
      setDueAtIso(new Date().toISOString());
      newTitle = newTitle.replace(/\b(tod|today)\b/i, "").replace(/\s{2,}/g, " ");
    }

    // Detect tomorrow shorthands: tom, tomorrow
    const tomMatch = newTitle.match(/\b(tom|tomorrow)\b/i);
    if (tomMatch) {
      const tomorrow = new Date();
      tomorrow.setDate(tomorrow.getDate() + 1);
      setDueDateLabel("Tomorrow");
      setDueAtIso(tomorrow.toISOString());
      newTitle = newTitle.replace(/\b(tom|tomorrow)\b/i, "").replace(/\s{2,}/g, " ");
    }

    setTitle(newTitle);
  };

  const handleSubmit = async () => {
    let taskTitle = title.trim();
    if (!taskTitle) return;

    let taskPrio = priority;
    let taskDue = dueAtIso || undefined;

    // Final pass for shorthand cleanup if typed at the very end
    const prioMatch = taskTitle.match(/\b([pP][1-4])\b/);
    if (prioMatch) {
      taskPrio = prioMatch[1].toUpperCase();
      taskTitle = taskTitle.replace(/\b[pP][1-4]\b/, "").trim();
    }
    const todayMatch = taskTitle.match(/\b(tod|today)\b/i);
    if (todayMatch) {
      taskDue = new Date().toISOString();
      taskTitle = taskTitle.replace(/\b(tod|today)\b/i, "").trim();
    }
    const tomMatch = taskTitle.match(/\b(tom|tomorrow)\b/i);
    if (tomMatch) {
      const tomorrow = new Date();
      tomorrow.setDate(tomorrow.getDate() + 1);
      taskDue = tomorrow.toISOString();
      taskTitle = taskTitle.replace(/\b(tom|tomorrow)\b/i, "").trim();
    }

    if (!taskTitle) return;

    const taskDesc = description.trim();
    const taskProj = project;

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
        top: 0,
        left: 0,
        right: 0,
        bottom: 0,
        zIndex: 1400,
        display: "flex",
        flexDirection: "column",
        justifyContent: "flex-end",
        bgcolor: "rgba(0, 0, 0, 0.4)",
        backdropFilter: "blur(2px)",
        animation: "fadeIn 0.15s ease-out",
      }}
      onClick={(e) => {
        if (e.target === e.currentTarget) handleClose();
      }}
    >
      {/* Quick Add Card anchored above keyboard — Pure Light Canvas Overlay */}
      <Box
        sx={{
          bgcolor: "#FFFFFF",
          color: "#202020",
          borderTopLeftRadius: 24,
          borderTopRightRadius: 24,
          borderTop: "1px solid #EEEEEE",
          p: 2,
          pb: `calc(16px + env(safe-area-inset-bottom))`,
          boxShadow: "0 -10px 40px rgba(0,0,0,0.2)",
          display: "flex",
          flexDirection: "column",
          gap: 1.5,
          position: "relative",
          zIndex: 1401,
        }}
      >
        {/* Task Title Input */}
        <InputBase
          inputRef={inputRef}
          placeholder="Task name (e.g. Call Arun p1 tod)"
          value={title}
          onChange={(e) => handleTitleChange(e.target.value)}
          onKeyDown={(e) => {
            if (e.key === "Enter" && !e.shiftKey) {
              e.preventDefault();
              void handleSubmit();
            }
          }}
          multiline
          maxRows={3}
          sx={{
            color: "#202020",
            fontSize: "1.05rem",
            fontWeight: 500,
            "& .MuiInputBase-input": {
              caretColor: "#DC4C3E",
              p: 0.5,
            },
            "& .MuiInputBase-input::placeholder": {
              color: "#888888",
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
              color: "#555555",
              fontSize: "0.9rem",
              px: 0.5,
              "& .MuiInputBase-input::placeholder": {
                color: "#999999",
                opacity: 1,
              },
            }}
          />
        )}

        {/* Action Row: Left scrollable chips + Pinned right submit button */}
        <Box
          sx={{
            display: "flex",
            alignItems: "center",
            justifyContent: "space-between",
            gap: 1,
            pt: 0.5,
          }}
        >
          {/* Scrollable metadata chips */}
          <Box
            sx={{
              display: "flex",
              alignItems: "center",
              gap: 1,
              overflowX: "auto",
              flexGrow: 1,
              minWidth: 0,
              "::-webkit-scrollbar": { display: "none" },
            }}
          >
            {/* [+] Description Toggle */}
            <IconButton
              size="small"
              onClick={() => setShowDesc(!showDesc)}
              sx={{
                bgcolor: showDesc ? "rgba(220, 76, 62, 0.12)" : "#F3F3F3",
                color: showDesc ? "#DC4C3E" : "#555555",
                border: "1px solid #E5E5E5",
                borderRadius: "10px",
                width: 36,
                height: 36,
                flexShrink: 0,
                "&:hover": { bgcolor: "#EAEAEA" },
              }}
            >
              <AddRoundedIcon sx={{ fontSize: 18 }} />
            </IconButton>

            {/* [Inbox] Project Chip */}
            <Box
              onClick={(e) => setProjectAnchor(e.currentTarget)}
              sx={{
                display: "flex",
                alignItems: "center",
                gap: 0.75,
                bgcolor: "#F3F3F3",
                color: "#333333",
                border: "1px solid #E5E5E5",
                px: 1.5,
                py: 0.75,
                borderRadius: "10px",
                cursor: "pointer",
                fontSize: "0.85rem",
                fontWeight: 500,
                userSelect: "none",
                flexShrink: 0,
                "&:hover": { bgcolor: "#EAEAEA" },
              }}
            >
              <InboxRoundedIcon sx={{ fontSize: 16, color: "#666666" }} />
              <Typography variant="body2" sx={{ fontSize: "0.85rem", fontWeight: 500 }}>
                {project}
              </Typography>
            </Box>

            {/* [Date] Chip */}
            <Box
              onClick={(e) => setDateAnchor(e.currentTarget)}
              sx={{
                display: "flex",
                alignItems: "center",
                gap: 0.75,
                bgcolor: dueDateLabel ? "rgba(220, 76, 62, 0.12)" : "#F3F3F3",
                color: dueDateLabel ? "#DC4C3E" : "#333333",
                border: dueDateLabel ? "1px solid rgba(220, 76, 62, 0.3)" : "1px solid #E5E5E5",
                px: 1.5,
                py: 0.75,
                borderRadius: "10px",
                cursor: "pointer",
                fontSize: "0.85rem",
                fontWeight: 500,
                userSelect: "none",
                flexShrink: 0,
                "&:hover": {
                  bgcolor: dueDateLabel ? "rgba(220, 76, 62, 0.18)" : "#EAEAEA",
                },
              }}
            >
              <CalendarTodayRoundedIcon
                sx={{
                  fontSize: 16,
                  color: dueDateLabel ? "#DC4C3E" : "#666666",
                }}
              />
              <Typography
                variant="body2"
                sx={{
                  fontSize: "0.85rem",
                  fontWeight: dueDateLabel ? 600 : 500,
                  color: dueDateLabel ? "#DC4C3E" : "#333333",
                }}
              >
                {dueDateLabel || "Date"}
              </Typography>
            </Box>

            {/* [Priority] Flag Button */}
            <IconButton
              size="small"
              onClick={(e) => setPriorityAnchor(e.currentTarget)}
              sx={{
                bgcolor: priority !== "P4" ? `${PRIORITY_COLORS[priority as keyof typeof PRIORITY_COLORS]}18` : "#F3F3F3",
                color: PRIORITY_COLORS[priority as keyof typeof PRIORITY_COLORS] || "#666666",
                border:
                  priority !== "P4"
                    ? `1px solid ${PRIORITY_COLORS[priority as keyof typeof PRIORITY_COLORS]}44`
                    : "1px solid #E5E5E5",
                borderRadius: "10px",
                width: 36,
                height: 36,
                flexShrink: 0,
                "&:hover": { bgcolor: "#EAEAEA" },
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
                bgcolor: "#F3F3F3",
                color: "#333333",
                border: "1px solid #E5E5E5",
                px: 1.5,
                py: 0.75,
                borderRadius: "10px",
                cursor: "pointer",
                fontSize: "0.85rem",
                fontWeight: 500,
                userSelect: "none",
                flexShrink: 0,
                "&:hover": { bgcolor: "#EAEAEA" },
              }}
            >
              <AttachFileRoundedIcon sx={{ fontSize: 16, color: "#666666" }} />
              <Typography variant="body2" sx={{ fontSize: "0.85rem", fontWeight: 500 }}>
                Attachment
              </Typography>
            </Box>
          </Box>

          {/* Todoist Red Submit Action Button (Always Pinned Right) */}
          <IconButton
            onClick={() => void handleSubmit()}
            disabled={!title.trim()}
            sx={{
              bgcolor: title.trim() ? "#DC4C3E" : "#F3F3F3",
              color: title.trim() ? "#FFFFFF" : "#AAAAAA",
              border: title.trim() ? "none" : "1px solid #E5E5E5",
              borderRadius: "14px",
              width: 42,
              height: 42,
              flexShrink: 0,
              boxShadow: title.trim() ? "0 4px 14px rgba(220, 76, 62, 0.4)" : "none",
              transition: "all 0.15s ease-in-out",
              "&:hover": {
                bgcolor: title.trim() ? "#B9382B" : "#EAEAEA",
              },
              "&.Mui-disabled": {
                bgcolor: "#F3F3F3",
                color: "#AAAAAA",
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
          slotProps={{
            paper: {
              sx: {
                bgcolor: "#FFFFFF",
                color: "#202020",
                border: "1px solid #E5E5E5",
                boxShadow: "0 6px 20px rgba(0,0,0,0.12)",
                borderRadius: 2,
              },
            },
          }}
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
          slotProps={{
            paper: {
              sx: {
                bgcolor: "#FFFFFF",
                color: "#202020",
                border: "1px solid #E5E5E5",
                boxShadow: "0 6px 20px rgba(0,0,0,0.12)",
                borderRadius: 2,
              },
            },
          }}
        >
          <MenuItem
            onClick={() => handleSetDateOption("Today", new Date().toISOString())}
            sx={{ color: "#DC4C3E", fontWeight: 600 }}
          >
            Today
          </MenuItem>
          <MenuItem
            onClick={() => {
              const tomorrow = new Date();
              tomorrow.setDate(tomorrow.getDate() + 1);
              handleSetDateOption("Tomorrow", tomorrow.toISOString());
            }}
            sx={{ color: "#E67E22", fontWeight: 500 }}
          >
            Tomorrow
          </MenuItem>
          <MenuItem
            onClick={() => {
              const nextWeek = new Date();
              nextWeek.setDate(nextWeek.getDate() + 7);
              handleSetDateOption("Next week", nextWeek.toISOString());
            }}
            sx={{ color: "#2980B9", fontWeight: 500 }}
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
          slotProps={{
            paper: {
              sx: {
                bgcolor: "#FFFFFF",
                color: "#202020",
                border: "1px solid #E5E5E5",
                boxShadow: "0 6px 20px rgba(0,0,0,0.12)",
                borderRadius: 2,
              },
            },
          }}
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
