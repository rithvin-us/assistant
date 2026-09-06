/**
 * Standalone Tasks Page — Todoist Authentic Light Design.
 *
 * Matches the official Todoist Android interface:
 * - Clean white-first canvas (#FFFFFF) with warm near-black typography (#202020)
 * - Top header with "Inbox" / view title and options
 * - Priority stroke circles (P1 Red, P2 Orange, P3 Blue, P4 Grey)
 * - Red date pills beneath task titles (e.g. "Today", "Tomorrow", "Aug 28") with flag/calendar icons
 * - Todoist Quick Add bottom sheet anchored above keyboard
 * - Tinted Red FAB (+)
 */

import { useState, useEffect } from "react";
import Box from "@mui/material/Box";
import Typography from "@mui/material/Typography";
import TextField from "@mui/material/TextField";
import InputAdornment from "@mui/material/InputAdornment";
import Button from "@mui/material/Button";
import Chip from "@mui/material/Chip";
import Dialog from "@mui/material/Dialog";
import DialogTitle from "@mui/material/DialogTitle";
import DialogContent from "@mui/material/DialogContent";
import DialogActions from "@mui/material/DialogActions";
import MenuItem from "@mui/material/MenuItem";
import Select from "@mui/material/Select";
import FormControl from "@mui/material/FormControl";
import InputLabel from "@mui/material/InputLabel";
import IconButton from "@mui/material/IconButton";
import Fab from "@mui/material/Fab";
import SearchRoundedIcon from "@mui/icons-material/SearchRounded";
import AddRoundedIcon from "@mui/icons-material/AddRounded";
import DeleteOutlineRoundedIcon from "@mui/icons-material/DeleteOutlineRounded";
import EventNoteRoundedIcon from "@mui/icons-material/EventNoteRounded";
import ArrowBackRoundedIcon from "@mui/icons-material/ArrowBackRounded";
import FormatListBulletedRoundedIcon from "@mui/icons-material/FormatListBulletedRounded";
import MoreVertRoundedIcon from "@mui/icons-material/MoreVertRounded";

import TodoistCheckbox from "../components/TodoistCheckbox";
import TodoistQuickAdd from "../components/TodoistQuickAdd";
import { PRIORITY_COLORS } from "../lib/priority";
import type { TaskItem } from "../api/types";
import {
  fetchTasks,
  createTask,
  updateTask,
  deleteTask,
} from "../api/productivity";

interface TasksScreenProps {
  onBack?: () => void;
}

export default function TasksScreen({ onBack }: TasksScreenProps) {
  const [searchQuery, setSearchQuery] = useState("");
  const [tasks, setTasks] = useState<TaskItem[]>([]);
  const [taskStatusFilter, setTaskStatusFilter] = useState<string>("todo");

  const [quickAddOpen, setQuickAddOpen] = useState(false);

  const [editModalOpen, setEditModalOpen] = useState(false);
  const [editingTask, setEditingTask] = useState<TaskItem | null>(null);
  const [taskTitle, setTaskTitle] = useState("");
  const [taskDesc, setTaskDesc] = useState("");
  const [taskPriority, setTaskPriority] = useState<string>("P4");
  const [taskDueDate, setTaskDueDate] = useState<string>("");
  const [taskProject, setTaskProject] = useState<string>("Inbox");

  const loadData = async () => {
    try {
      const res = await fetchTasks({
        status: taskStatusFilter === "ALL" ? undefined : taskStatusFilter,
        q: searchQuery || undefined,
      });
      setTasks(res);
    } catch (err) {
      console.warn("Failed to load tasks:", err);
    }
  };

  useEffect(() => {
    let cancelled = false;
    queueMicrotask(() => {
      if (cancelled) return;
      void (async () => {
        try {
          const res = await fetchTasks({
            status: taskStatusFilter === "ALL" ? undefined : taskStatusFilter,
            q: searchQuery || undefined,
          });
          if (!cancelled) setTasks(res);
        } catch (err) {
          console.warn("Failed to load tasks:", err);
        }
      })();
    });
    return () => {
      cancelled = true;
    };
  }, [taskStatusFilter, searchQuery]);

  const handleQuickAddTask = async (newTask: {
    title: string;
    description?: string;
    priority: string;
    due_at?: string;
    project: string;
  }) => {
    try {
      await createTask(newTask);
      await loadData();
    } catch (err) {
      console.error("Failed to add task:", err);
    }
  };

  const handleSaveEditTask = async () => {
    if (!editingTask || !taskTitle.trim()) return;

    let finalTitle = taskTitle.trim();
    let finalPrio = taskPriority;
    let finalDue = taskDueDate ? new Date(taskDueDate).toISOString() : null;

    // Check for shorthands in edit field too
    const prioMatch = finalTitle.match(/\b([pP][1-4])\b/);
    if (prioMatch) {
      finalPrio = prioMatch[1].toUpperCase();
      finalTitle = finalTitle.replace(/\b[pP][1-4]\b/, "").trim();
    }
    const todayMatch = finalTitle.match(/\b(tod|today)\b/i);
    if (todayMatch) {
      finalDue = new Date().toISOString();
      finalTitle = finalTitle.replace(/\b(tod|today)\b/i, "").trim();
    }
    const tomMatch = finalTitle.match(/\b(tom|tomorrow)\b/i);
    if (tomMatch) {
      const tomorrow = new Date();
      tomorrow.setDate(tomorrow.getDate() + 1);
      finalDue = tomorrow.toISOString();
      finalTitle = finalTitle.replace(/\b(tom|tomorrow)\b/i, "").trim();
    }

    try {
      await updateTask(editingTask.id, {
        title: finalTitle,
        description: taskDesc.trim() || undefined,
        priority: finalPrio,
        due_at: finalDue,
        project: taskProject,
      });
      setEditModalOpen(false);
      setEditingTask(null);
      await loadData();
    } catch (err) {
      console.error(err);
    }
  };

  const handleToggleTaskComplete = async (task: TaskItem) => {
    const nextStatus = task.status === "completed" ? "todo" : "completed";
    try {
      setTasks((prev) =>
        prev.map((t) => (t.id === task.id ? { ...t, status: nextStatus } : t))
      );
      await updateTask(task.id, { status: nextStatus });
      await loadData();
    } catch (err) {
      console.error(err);
    }
  };

  const handleDeleteTask = async (id: string) => {
    try {
      setTasks((prev) => prev.filter((t) => t.id !== id));
      await deleteTask(id);
    } catch (err) {
      console.error(err);
    }
  };

  // Format date display like Todoist (e.g. "Today", "Tomorrow", "Aug 28")
  const formatDateLabel = (isoDate: string) => {
    const d = new Date(isoDate);
    const today = new Date();
    const tomorrow = new Date();
    tomorrow.setDate(tomorrow.getDate() + 1);

    if (
      d.getDate() === today.getDate() &&
      d.getMonth() === today.getMonth() &&
      d.getFullYear() === today.getFullYear()
    ) {
      return "Today";
    }
    if (
      d.getDate() === tomorrow.getDate() &&
      d.getMonth() === tomorrow.getMonth() &&
      d.getFullYear() === tomorrow.getFullYear()
    ) {
      return "Tomorrow";
    }
    return d.toLocaleDateString("en-US", { month: "short", day: "numeric" });
  };

  return (
    <Box
      sx={{
        height: "100%",
        display: "flex",
        flexDirection: "column",
        bgcolor: "#FFFFFF",
        color: "#202020",
      }}
    >
      {/* Top Header — Todoist Inbox Style (Light Theme) */}
      <Box
        sx={{
          px: 2,
          pt: 1.5,
          pb: 1.5,
          display: "flex",
          alignItems: "center",
          gap: 1,
          borderBottom: "1px solid #EEEEEE",
          bgcolor: "#FFFFFF",
        }}
      >
        {onBack && (
          <IconButton size="small" onClick={onBack} sx={{ color: "#202020" }}>
            <ArrowBackRoundedIcon />
          </IconButton>
        )}
        <Typography variant="h5" sx={{ fontWeight: 700, fontSize: "1.35rem", flexGrow: 1, color: "#202020" }}>
          Inbox
        </Typography>

        <IconButton size="small" sx={{ color: "#666666" }}>
          <FormatListBulletedRoundedIcon />
        </IconButton>
        <IconButton size="small" sx={{ color: "#666666" }}>
          <MoreVertRoundedIcon />
        </IconButton>
      </Box>

      {/* Filter and Search Bar */}
      <Box sx={{ p: 2, pb: 1, display: "flex", flexDirection: "column", gap: 1.25 }}>
        <TextField
          size="small"
          placeholder="Search tasks…"
          value={searchQuery}
          onChange={(e) => setSearchQuery(e.target.value)}
          slotProps={{
            input: {
              startAdornment: (
                <InputAdornment position="start">
                  <SearchRoundedIcon sx={{ fontSize: 20, color: "#888888" }} />
                </InputAdornment>
              ),
            },
          }}
          sx={{
            "& .MuiOutlinedInput-root": {
              borderRadius: 3,
              bgcolor: "#F5F5F5",
              color: "#202020",
              "& fieldset": { borderColor: "#E5E5E5" },
              "&:hover fieldset": { borderColor: "#CCCCCC" },
            },
          }}
        />

        <Box sx={{ display: "flex", gap: 0.75, flexWrap: "wrap", alignItems: "center" }}>
          {["todo", "completed", "ALL"].map((st) => (
            <Chip
              key={st}
              size="small"
              label={st === "todo" ? "Active" : st === "completed" ? "Completed" : "All"}
              color={taskStatusFilter === st ? "error" : "default"}
              onClick={() => setTaskStatusFilter(st)}
              sx={{
                fontWeight: 500,
                fontSize: "0.78rem",
                bgcolor: taskStatusFilter === st ? "#DC4C3E" : "#F3F3F3",
                color: taskStatusFilter === st ? "#FFFFFF" : "#444444",
                "&:hover": {
                  bgcolor: taskStatusFilter === st ? "#B9382B" : "#EAEAEA",
                },
              }}
            />
          ))}
        </Box>
      </Box>

      {/* Main Task List */}
      <Box sx={{ flexGrow: 1, overflowY: quickAddOpen ? "hidden" : "auto", px: 2, pb: 12, position: "relative" }}>
        <Box sx={{ display: "flex", flexDirection: "column" }}>
          {tasks.length === 0 ? (
            <Typography color="#888888" sx={{ py: 6, textAlign: "center", fontSize: "0.95rem" }}>
              No tasks found. Tap + to add a task.
            </Typography>
          ) : (
            tasks.map((t) => (
              <Box
                key={t.id}
                sx={{
                  minHeight: 56,
                  display: "flex",
                  alignItems: "flex-start",
                  py: 1.25,
                  px: 0.5,
                  borderBottom: "1px solid #F0F0F0",
                  gap: 1.5,
                  "&:hover": { bgcolor: "#FAFAFA" },
                }}
              >
                {/* Priority Checkbox */}
                <TodoistCheckbox
                  priority={t.priority}
                  checked={t.status === "completed"}
                  onChange={() => void handleToggleTaskComplete(t)}
                />

                {/* Content */}
                <Box
                  sx={{ flexGrow: 1, cursor: "pointer", pt: 0.25 }}
                  onClick={() => {
                    setEditingTask(t);
                    setTaskTitle(t.title);
                    setTaskDesc(t.description);
                    setTaskPriority(t.priority);
                    setTaskDueDate(t.due_at ? t.due_at.slice(0, 16) : "");
                    setTaskProject(t.project);
                    setEditModalOpen(true);
                  }}
                >
                  <Typography
                    variant="body1"
                    sx={{
                      fontSize: "0.98rem",
                      fontWeight: 400,
                      textDecoration: t.status === "completed" ? "line-through" : "none",
                      color: t.status === "completed" ? "#999999" : "#202020",
                      lineHeight: 1.35,
                    }}
                  >
                    {t.title}
                  </Typography>

                  {t.description && (
                    <Typography variant="body2" sx={{ fontSize: "0.82rem", color: "#666666", mt: 0.25 }}>
                      {t.description}
                    </Typography>
                  )}

                  {/* Subtitle Details — Red Date Pill matching Todoist style */}
                  <Box sx={{ display: "flex", alignItems: "center", gap: 1.25, mt: 0.5 }}>
                    {t.due_at && (
                      <Typography
                        variant="caption"
                        sx={{
                          color: "#DC4C3E",
                          fontSize: "0.76rem",
                          fontWeight: 500,
                          display: "flex",
                          alignItems: "center",
                          gap: 0.5,
                        }}
                      >
                        <EventNoteRoundedIcon sx={{ fontSize: 13, color: "#DC4C3E" }} />
                        {formatDateLabel(t.due_at)}
                      </Typography>
                    )}
                    {t.project && t.project !== "Inbox" && (
                      <Typography
                        variant="caption"
                        sx={{ color: "#777777", fontSize: "0.75rem" }}
                      >
                        #{t.project}
                      </Typography>
                    )}
                  </Box>
                </Box>

                {/* Delete IconButton */}
                <IconButton size="small" onClick={() => void handleDeleteTask(t.id)}>
                  <DeleteOutlineRoundedIcon sx={{ fontSize: 18, color: "#999999", "&:hover": { color: "#DC4C3E" } }} />
                </IconButton>
              </Box>
            ))
          )}
        </Box>
      </Box>

      {/* Signature Todoist Red FAB (+) */}
      <Fab
        color="primary"
        aria-label="Add task"
        onClick={() => setQuickAddOpen(true)}
        sx={{
          position: "fixed",
          right: 20,
          bottom: `calc(24px + env(safe-area-inset-bottom))`,
          bgcolor: "#DC4C3E",
          boxShadow: "0 8px 24px rgba(220, 76, 62, 0.4)",
          "&:hover": { bgcolor: "#B9382B" },
        }}
      >
        <AddRoundedIcon sx={{ fontSize: 28 }} />
      </Fab>

      {/* Todoist Quick Add Bottom Sheet */}
      <TodoistQuickAdd
        open={quickAddOpen}
        onClose={() => setQuickAddOpen(false)}
        onAddTask={handleQuickAddTask}
        defaultProject="Inbox"
      />

      {/* Task Edit Dialog */}
      <Dialog
        open={editModalOpen}
        onClose={() => setEditModalOpen(false)}
        fullWidth
        maxWidth="xs"
        slotProps={{
          paper: {
            sx: {
              bgcolor: "#FFFFFF",
              color: "#202020",
              borderRadius: 3,
              border: "1px solid #EEEEEE",
            },
          },
        }}
      >
        <DialogTitle sx={{ fontWeight: 700, color: "#202020" }}>Edit Task</DialogTitle>
        <DialogContent sx={{ display: "flex", flexDirection: "column", gap: 2, pt: 1 }}>
          <TextField
            autoFocus
            label="Task Title"
            value={taskTitle}
            onChange={(e) => setTaskTitle(e.target.value)}
            fullWidth
            required
            variant="outlined"
            slotProps={{
              input: { sx: { color: "#202020" } },
              inputLabel: { sx: { color: "#666666" } },
            }}
          />
          <TextField
            label="Description (optional)"
            value={taskDesc}
            onChange={(e) => setTaskDesc(e.target.value)}
            fullWidth
            multiline
            rows={2}
            slotProps={{
              input: { sx: { color: "#202020" } },
              inputLabel: { sx: { color: "#666666" } },
            }}
          />
          <Box sx={{ display: "flex", gap: 2 }}>
            <FormControl fullWidth size="small">
              <InputLabel sx={{ color: "#666666" }}>Priority</InputLabel>
              <Select
                value={taskPriority}
                label="Priority"
                onChange={(e) => setTaskPriority(e.target.value)}
                sx={{ color: "#202020" }}
              >
                <MenuItem value="P1" sx={{ color: PRIORITY_COLORS.P1, fontWeight: 700 }}>P1 — Red</MenuItem>
                <MenuItem value="P2" sx={{ color: PRIORITY_COLORS.P2, fontWeight: 700 }}>P2 — Orange</MenuItem>
                <MenuItem value="P3" sx={{ color: PRIORITY_COLORS.P3, fontWeight: 700 }}>P3 — Blue</MenuItem>
                <MenuItem value="P4" sx={{ color: PRIORITY_COLORS.P4, fontWeight: 700 }}>P4 — Grey</MenuItem>
              </Select>
            </FormControl>
            <TextField
              fullWidth
              size="small"
              label="Project"
              value={taskProject}
              onChange={(e) => setTaskProject(e.target.value)}
              slotProps={{
                input: { sx: { color: "#202020" } },
                inputLabel: { sx: { color: "#666666" } },
              }}
            />
          </Box>
          <TextField
            type="datetime-local"
            label="Due Date & Time"
            value={taskDueDate}
            onChange={(e) => setTaskDueDate(e.target.value)}
            slotProps={{
              inputLabel: { shrink: true, sx: { color: "#666666" } },
              input: { sx: { color: "#202020" } },
            }}
            fullWidth
            size="small"
          />
        </DialogContent>
        <DialogActions sx={{ p: 2 }}>
          <Button onClick={() => setEditModalOpen(false)} sx={{ color: "#666666" }}>Cancel</Button>
          <Button
            variant="contained"
            onClick={() => void handleSaveEditTask()}
            disabled={!taskTitle.trim()}
            sx={{ bgcolor: "#DC4C3E", "&:hover": { bgcolor: "#B9382B" } }}
          >
            Save
          </Button>
        </DialogActions>
      </Dialog>
    </Box>
  );
}
