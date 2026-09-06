/**
 * Standalone Tasks Page.
 *
 * Implements Task management with Todoist-inspired visual language
 * (white canvas, near-black ink, Todoist Red #DC4C3E, priority strokes).
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

import TodoistCheckbox from "../components/TodoistCheckbox";
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
  const [taskPriorityFilter, setTaskPriorityFilter] = useState<string>("ALL");

  const [taskModalOpen, setTaskModalOpen] = useState(false);
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
        priority: taskPriorityFilter === "ALL" ? undefined : taskPriorityFilter,
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
            priority: taskPriorityFilter === "ALL" ? undefined : taskPriorityFilter,
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
  }, [taskStatusFilter, taskPriorityFilter, searchQuery]);

  const handleSaveTask = async () => {
    if (!taskTitle.trim()) return;
    try {
      if (editingTask) {
        await updateTask(editingTask.id, {
          title: taskTitle,
          description: taskDesc,
          priority: taskPriority,
          due_at: taskDueDate ? new Date(taskDueDate).toISOString() : null,
          project: taskProject,
        });
      } else {
        await createTask({
          title: taskTitle,
          description: taskDesc,
          priority: taskPriority,
          due_at: taskDueDate ? new Date(taskDueDate).toISOString() : undefined,
          project: taskProject,
        });
      }
      setTaskModalOpen(false);
      resetTaskForm();
      await loadData();
    } catch (err) {
      console.error(err);
    }
  };

  const resetTaskForm = () => {
    setEditingTask(null);
    setTaskTitle("");
    setTaskDesc("");
    setTaskPriority("P4");
    setTaskDueDate("");
    setTaskProject("Inbox");
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
      {/* Top Header */}
      <Box
        sx={{
          px: 2,
          pt: 1.5,
          pb: 1.5,
          display: "flex",
          alignItems: "center",
          gap: 1,
          borderBottom: "1px solid #EEEEEE",
        }}
      >
        {onBack && (
          <IconButton size="small" onClick={onBack} sx={{ color: "#202020" }}>
            <ArrowBackRoundedIcon />
          </IconButton>
        )}
        <Typography variant="h6" sx={{ fontWeight: 700, fontSize: "1.15rem", flexGrow: 1 }}>
          Tasks
        </Typography>
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
                  <SearchRoundedIcon sx={{ fontSize: 20, color: "#808080" }} />
                </InputAdornment>
              ),
            },
          }}
          sx={{
            "& .MuiOutlinedInput-root": {
              borderRadius: 3,
              bgcolor: "#FAFAFA",
            },
          }}
        />

        <Box sx={{ display: "flex", gap: 0.75, flexWrap: "wrap" }}>
          {["todo", "completed", "ALL"].map((st) => (
            <Chip
              key={st}
              size="small"
              label={st === "todo" ? "Active" : st === "completed" ? "Completed" : "All"}
              color={taskStatusFilter === st ? "primary" : "default"}
              onClick={() => setTaskStatusFilter(st)}
              sx={{ fontWeight: 500, fontSize: "0.78rem" }}
            />
          ))}
          {["P1", "P2", "P3", "P4"].map((p) => (
            <Chip
              key={p}
              size="small"
              label={p}
              variant={taskPriorityFilter === p ? "filled" : "outlined"}
              onClick={() => setTaskPriorityFilter(taskPriorityFilter === p ? "ALL" : p)}
              sx={{
                fontWeight: 600,
                fontSize: "0.75rem",
                borderColor: PRIORITY_COLORS[p as keyof typeof PRIORITY_COLORS],
                color: taskPriorityFilter === p ? "#FFF" : PRIORITY_COLORS[p as keyof typeof PRIORITY_COLORS],
                bgcolor: taskPriorityFilter === p ? PRIORITY_COLORS[p as keyof typeof PRIORITY_COLORS] : "transparent",
              }}
            />
          ))}
        </Box>
      </Box>

      {/* Main Content Area */}
      <Box sx={{ flexGrow: 1, overflowY: "auto", px: 2, pb: 10 }}>
        <Box sx={{ display: "flex", flexDirection: "column" }}>
          {tasks.length === 0 ? (
            <Typography color="text.secondary" sx={{ py: 6, textAlign: "center", fontSize: "0.9rem" }}>
              No tasks found. Tap + to add a task.
            </Typography>
          ) : (
            tasks.map((t) => (
              <Box
                key={t.id}
                sx={{
                  minHeight: 52,
                  display: "flex",
                  alignItems: "center",
                  py: 1,
                  px: 1,
                  borderBottom: "1px solid #EEEEEE",
                  gap: 1,
                  "&:hover": { bgcolor: "#FAFAFA" },
                }}
              >
                <TodoistCheckbox
                  priority={t.priority}
                  checked={t.status === "completed"}
                  onChange={() => void handleToggleTaskComplete(t)}
                />
                <Box
                  sx={{ flexGrow: 1, cursor: "pointer" }}
                  onClick={() => {
                    setEditingTask(t);
                    setTaskTitle(t.title);
                    setTaskDesc(t.description);
                    setTaskPriority(t.priority);
                    setTaskDueDate(t.due_at ? t.due_at.slice(0, 16) : "");
                    setTaskProject(t.project);
                    setTaskModalOpen(true);
                  }}
                >
                  <Typography
                    variant="body1"
                    sx={{
                      fontSize: "0.95rem",
                      fontWeight: 500,
                      textDecoration: t.status === "completed" ? "line-through" : "none",
                      color: t.status === "completed" ? "text.secondary" : "text.primary",
                    }}
                  >
                    {t.title}
                  </Typography>
                  {t.description && (
                    <Typography variant="body2" color="text.secondary" sx={{ fontSize: "0.8rem" }}>
                      {t.description}
                    </Typography>
                  )}
                  <Box sx={{ display: "flex", alignItems: "center", gap: 1, mt: 0.25 }}>
                    {t.project && (
                      <Typography
                        variant="caption"
                        sx={{ color: "primary.main", fontWeight: 600, fontSize: "0.72rem" }}
                      >
                        #{t.project}
                      </Typography>
                    )}
                    {t.due_at && (
                      <Typography
                        variant="caption"
                        sx={{
                          color: new Date(t.due_at) < new Date() ? "error.main" : "text.secondary",
                          fontSize: "0.72rem",
                          display: "flex",
                          alignItems: "center",
                          gap: 0.25,
                        }}
                      >
                        <EventNoteRoundedIcon sx={{ fontSize: 12 }} />
                        {new Date(t.due_at).toLocaleDateString()}
                      </Typography>
                    )}
                  </Box>
                </Box>
                <IconButton size="small" onClick={() => void handleDeleteTask(t.id)}>
                  <DeleteOutlineRoundedIcon sx={{ fontSize: 18, color: "#808080" }} />
                </IconButton>
              </Box>
            ))
          )}
        </Box>
      </Box>

      {/* Floating Action Button */}
      <Fab
        color="primary"
        aria-label="Add task"
        onClick={() => {
          resetTaskForm();
          setTaskModalOpen(true);
        }}
        sx={{
          position: "fixed",
          right: 20,
          bottom: `calc(24px + env(safe-area-inset-bottom))`,
          bgcolor: "#DC4C3E",
          "&:hover": { bgcolor: "#B9382B" },
        }}
      >
        <AddRoundedIcon />
      </Fab>

      {/* Task Modal */}
      <Dialog
        open={taskModalOpen}
        onClose={() => setTaskModalOpen(false)}
        fullWidth
        maxWidth="xs"
      >
        <DialogTitle sx={{ fontWeight: 700 }}>
          {editingTask ? "Edit Task" : "Add Task"}
        </DialogTitle>
        <DialogContent sx={{ display: "flex", flexDirection: "column", gap: 2, pt: 1 }}>
          <TextField
            autoFocus
            label="Task Title"
            value={taskTitle}
            onChange={(e) => setTaskTitle(e.target.value)}
            fullWidth
            required
            variant="outlined"
          />
          <TextField
            label="Description (optional)"
            value={taskDesc}
            onChange={(e) => setTaskDesc(e.target.value)}
            fullWidth
            multiline
            rows={2}
          />
          <Box sx={{ display: "flex", gap: 2 }}>
            <FormControl fullWidth size="small">
              <InputLabel>Priority</InputLabel>
              <Select
                value={taskPriority}
                label="Priority"
                onChange={(e) => setTaskPriority(e.target.value)}
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
            />
          </Box>
          <TextField
            type="datetime-local"
            label="Due Date & Time"
            value={taskDueDate}
            onChange={(e) => setTaskDueDate(e.target.value)}
            slotProps={{ inputLabel: { shrink: true } }}
            fullWidth
            size="small"
          />
        </DialogContent>
        <DialogActions sx={{ p: 2 }}>
          <Button onClick={() => setTaskModalOpen(false)}>Cancel</Button>
          <Button
            variant="contained"
            onClick={() => void handleSaveTask()}
            disabled={!taskTitle.trim()}
            sx={{ bgcolor: "#DC4C3E", "&:hover": { bgcolor: "#B9382B" } }}
          >
            Save Task
          </Button>
        </DialogActions>
      </Dialog>
    </Box>
  );
}
