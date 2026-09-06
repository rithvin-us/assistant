/**
 * Standalone Productivity Layer Screen.
 *
 * Implements Tasks, Reminders, Notes, and Ideas management with Todoist-inspired
 * visual language (white canvas, near-black ink, Todoist Red #DC4C3E, priority strokes).
 *
 * Runs 100% deterministically without any AI API key or network connection requirement.
 */

import { useState, useEffect } from "react";
import Box from "@mui/material/Box";
import Typography from "@mui/material/Typography";
import Tabs from "@mui/material/Tabs";
import Tab from "@mui/material/Tab";
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
import Card from "@mui/material/Card";
import CardContent from "@mui/material/CardContent";
import CardActions from "@mui/material/CardActions";
import IconButton from "@mui/material/IconButton";
import Fab from "@mui/material/Fab";
import SearchRoundedIcon from "@mui/icons-material/SearchRounded";
import AddRoundedIcon from "@mui/icons-material/AddRounded";
import DeleteOutlineRoundedIcon from "@mui/icons-material/DeleteOutlineRounded";
import EditOutlinedIcon from "@mui/icons-material/EditOutlined";
import ArchiveOutlinedIcon from "@mui/icons-material/ArchiveOutlined";
import UnarchiveOutlinedIcon from "@mui/icons-material/UnarchiveOutlined";
import TransformRoundedIcon from "@mui/icons-material/TransformRounded";
import EventNoteRoundedIcon from "@mui/icons-material/EventNoteRounded";
import AlarmRoundedIcon from "@mui/icons-material/AlarmRounded";
import ArrowBackRoundedIcon from "@mui/icons-material/ArrowBackRounded";

import TodoistCheckbox from "../components/TodoistCheckbox";
import { PRIORITY_COLORS } from "../lib/priority";

import type { TaskItem, ReminderItem, NoteItem, IdeaItem } from "../api/types";
import {
  fetchTasks,
  createTask,
  updateTask,
  deleteTask,
  fetchReminders,
  createReminder,
  updateReminder,
  deleteReminder,
  fetchNotes,
  createNote,
  updateNote,
  deleteNote,
  fetchIdeas,
  createIdea,
  updateIdea,
  convertIdeaToTask,
  deleteIdea,
} from "../api/productivity";

export type ProductivityTab = "tasks" | "reminders" | "notes" | "ideas";

interface ProductivityScreenProps {
  initialTab?: ProductivityTab;
  onBack?: () => void;
}

export default function ProductivityScreen({
  initialTab = "tasks",
  onBack,
}: ProductivityScreenProps) {
  const [activeTab, setActiveTab] = useState<ProductivityTab>(initialTab);
  const [searchQuery, setSearchQuery] = useState("");

  // Data state
  const [tasks, setTasks] = useState<TaskItem[]>([]);
  const [reminders, setReminders] = useState<ReminderItem[]>([]);
  const [notes, setNotes] = useState<NoteItem[]>([]);
  const [ideas, setIdeas] = useState<IdeaItem[]>([]);

  // Filters
  const [taskStatusFilter, setTaskStatusFilter] = useState<string>("todo");
  const [taskPriorityFilter, setTaskPriorityFilter] = useState<string>("ALL");
  const [reminderStatusFilter, setReminderStatusFilter] = useState<string>("pending");
  const [noteArchiveFilter, setNoteArchiveFilter] = useState<boolean>(false);
  const [ideaStatusFilter, setIdeaStatusFilter] = useState<string>("active");

  // Modals
  const [taskModalOpen, setTaskModalOpen] = useState(false);
  const [editingTask, setEditingTask] = useState<TaskItem | null>(null);
  const [taskTitle, setTaskTitle] = useState("");
  const [taskDesc, setTaskDesc] = useState("");
  const [taskPriority, setTaskPriority] = useState<string>("P4");
  const [taskDueDate, setTaskDueDate] = useState<string>("");
  const [taskProject, setTaskProject] = useState<string>("Inbox");

  const [reminderModalOpen, setReminderModalOpen] = useState(false);
  const [editingReminder, setEditingReminder] = useState<ReminderItem | null>(null);
  const [reminderTitle, setReminderTitle] = useState("");
  const [reminderTime, setReminderTime] = useState("");

  const [noteModalOpen, setNoteModalOpen] = useState(false);
  const [editingNote, setEditingNote] = useState<NoteItem | null>(null);
  const [noteTitle, setNoteTitle] = useState("");
  const [noteContent, setNoteContent] = useState("");
  const [noteTagsInput, setNoteTagsInput] = useState("");

  const [ideaModalOpen, setIdeaModalOpen] = useState(false);
  const [editingIdea, setEditingIdea] = useState<IdeaItem | null>(null);
  const [ideaTitle, setIdeaTitle] = useState("");
  const [ideaDesc, setIdeaDesc] = useState("");

  useEffect(() => {
    let cancelled = false;
    queueMicrotask(() => {
      if (cancelled) return;
      void (async () => {
        try {
          if (activeTab === "tasks") {
            const res = await fetchTasks({
              status: taskStatusFilter === "ALL" ? undefined : taskStatusFilter,
              priority: taskPriorityFilter === "ALL" ? undefined : taskPriorityFilter,
              q: searchQuery || undefined,
            });
            if (!cancelled) setTasks(res);
          } else if (activeTab === "reminders") {
            const res = await fetchReminders({
              status: reminderStatusFilter === "ALL" ? undefined : reminderStatusFilter,
              q: searchQuery || undefined,
            });
            if (!cancelled) setReminders(res);
          } else if (activeTab === "notes") {
            const res = await fetchNotes({
              is_archived: noteArchiveFilter,
              q: searchQuery || undefined,
            });
            if (!cancelled) setNotes(res);
          } else if (activeTab === "ideas") {
            const res = await fetchIdeas({
              status: ideaStatusFilter === "ALL" ? undefined : ideaStatusFilter,
              q: searchQuery || undefined,
            });
            if (!cancelled) setIdeas(res);
          }
        } catch (err) {
          console.warn("Failed to load productivity items:", err);
        }
      })();
    });
    return () => {
      cancelled = true;
    };
  }, [
    activeTab,
    taskStatusFilter,
    taskPriorityFilter,
    reminderStatusFilter,
    noteArchiveFilter,
    ideaStatusFilter,
    searchQuery,
  ]);

  const loadData = async () => {
    try {
      if (activeTab === "tasks") {
        const res = await fetchTasks({
          status: taskStatusFilter === "ALL" ? undefined : taskStatusFilter,
          priority: taskPriorityFilter === "ALL" ? undefined : taskPriorityFilter,
          q: searchQuery || undefined,
        });
        setTasks(res);
      } else if (activeTab === "reminders") {
        const res = await fetchReminders({
          status: reminderStatusFilter === "ALL" ? undefined : reminderStatusFilter,
          q: searchQuery || undefined,
        });
        setReminders(res);
      } else if (activeTab === "notes") {
        const res = await fetchNotes({
          is_archived: noteArchiveFilter,
          q: searchQuery || undefined,
        });
        setNotes(res);
      } else if (activeTab === "ideas") {
        const res = await fetchIdeas({
          status: ideaStatusFilter === "ALL" ? undefined : ideaStatusFilter,
          q: searchQuery || undefined,
        });
        setIdeas(res);
      }
    } catch (err) {
      console.warn("Failed to load productivity items:", err);
    }
  };


  // Handle Task Save
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

  // Handle Reminder Save
  const handleSaveReminder = async () => {
    if (!reminderTitle.trim() || !reminderTime) return;
    try {
      const remindAtIso = new Date(reminderTime).toISOString();
      if (editingReminder) {
        await updateReminder(editingReminder.id, {
          title: reminderTitle,
          remind_at: remindAtIso,
        });
      } else {
        await createReminder({
          title: reminderTitle,
          remind_at: remindAtIso,
        });
      }
      setReminderModalOpen(false);
      setEditingReminder(null);
      setReminderTitle("");
      setReminderTime("");
      await loadData();
    } catch (err) {
      console.error(err);
    }
  };

  const handleToggleReminderHandled = async (reminder: ReminderItem) => {
    const nextStatus = reminder.status === "handled" ? "pending" : "handled";
    try {
      setReminders((prev) =>
        prev.map((r) => (r.id === reminder.id ? { ...r, status: nextStatus } : r))
      );
      await updateReminder(reminder.id, { status: nextStatus });
    } catch (err) {
      console.error(err);
    }
  };

  const handleDeleteReminder = async (id: string) => {
    try {
      setReminders((prev) => prev.filter((r) => r.id !== id));
      await deleteReminder(id);
    } catch (err) {
      console.error(err);
    }
  };

  // Handle Note Save
  const handleSaveNote = async () => {
    if (!noteTitle.trim()) return;
    try {
      const tags = noteTagsInput
        .split(",")
        .map((s) => s.trim())
        .filter(Boolean);
      if (editingNote) {
        await updateNote(editingNote.id, {
          title: noteTitle,
          content: noteContent,
          tags,
        });
      } else {
        await createNote({
          title: noteTitle,
          content: noteContent,
          tags,
        });
      }
      setNoteModalOpen(false);
      setEditingNote(null);
      setNoteTitle("");
      setNoteContent("");
      setNoteTagsInput("");
      await loadData();
    } catch (err) {
      console.error(err);
    }
  };

  const handleToggleArchiveNote = async (note: NoteItem) => {
    try {
      await updateNote(note.id, { is_archived: !note.is_archived });
      await loadData();
    } catch (err) {
      console.error(err);
    }
  };

  const handleDeleteNote = async (id: string) => {
    try {
      setNotes((prev) => prev.filter((n) => n.id !== id));
      await deleteNote(id);
    } catch (err) {
      console.error(err);
    }
  };

  // Handle Idea Save
  const handleSaveIdea = async () => {
    if (!ideaTitle.trim()) return;
    try {
      if (editingIdea) {
        await updateIdea(editingIdea.id, {
          title: ideaTitle,
          description: ideaDesc,
        });
      } else {
        await createIdea({
          title: ideaTitle,
          description: ideaDesc,
        });
      }
      setIdeaModalOpen(false);
      setEditingIdea(null);
      setIdeaTitle("");
      setIdeaDesc("");
      await loadData();
    } catch (err) {
      console.error(err);
    }
  };

  const handleConvertIdea = async (idea: IdeaItem) => {
    try {
      await convertIdeaToTask(idea.id);
      await loadData();
    } catch (err) {
      console.error(err);
    }
  };

  const handleDeleteIdea = async (id: string) => {
    try {
      setIdeas((prev) => prev.filter((i) => i.id !== id));
      await deleteIdea(id);
    } catch (err) {
      console.error(err);
    }
  };

  // FAB Trigger
  const handleFabClick = () => {
    if (activeTab === "tasks") {
      resetTaskForm();
      setTaskModalOpen(true);
    } else if (activeTab === "reminders") {
      setEditingReminder(null);
      setReminderTitle("");
      const defaultTime = new Date(Date.now() + 3600 * 1000).toISOString().slice(0, 16);
      setReminderTime(defaultTime);
      setReminderModalOpen(true);
    } else if (activeTab === "notes") {
      setEditingNote(null);
      setNoteTitle("");
      setNoteContent("");
      setNoteTagsInput("");
      setNoteModalOpen(true);
    } else if (activeTab === "ideas") {
      setEditingIdea(null);
      setIdeaTitle("");
      setIdeaDesc("");
      setIdeaModalOpen(true);
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
      {/* Header Bar */}
      <Box
        sx={{
          px: 2,
          pt: 1.5,
          pb: 1,
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
          Productivity
        </Typography>
      </Box>

      {/* Tabs */}
      <Box sx={{ borderBottom: "1px solid #EEEEEE" }}>
        <Tabs
          value={activeTab}
          onChange={(_, val) => setActiveTab(val)}
          variant="fullWidth"
          textColor="primary"
          indicatorColor="primary"
          sx={{
            minHeight: 44,
            "& .MuiTab-root": {
              textTransform: "none",
              fontWeight: 600,
              fontSize: "0.9rem",
              minHeight: 44,
              py: 1,
            },
          }}
        >
          <Tab value="tasks" label="Tasks" />
          <Tab value="reminders" label="Reminders" />
          <Tab value="notes" label="Notes" />
          <Tab value="ideas" label="Ideas" />
        </Tabs>
      </Box>

      {/* Filter and Search Bar */}
      <Box sx={{ p: 2, pb: 1, display: "flex", flexDirection: "column", gap: 1.25 }}>
        <TextField
          size="small"
          placeholder={`Search ${activeTab}…`}
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

        {/* Tab Specific Filters */}
        {activeTab === "tasks" && (
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
        )}

        {activeTab === "reminders" && (
          <Box sx={{ display: "flex", gap: 0.75 }}>
            {["pending", "handled", "ALL"].map((st) => (
              <Chip
                key={st}
                size="small"
                label={st === "pending" ? "Upcoming" : st === "handled" ? "Handled" : "All"}
                color={reminderStatusFilter === st ? "primary" : "default"}
                onClick={() => setReminderStatusFilter(st)}
                sx={{ fontWeight: 500 }}
              />
            ))}
          </Box>
        )}

        {activeTab === "notes" && (
          <Box sx={{ display: "flex", gap: 0.75 }}>
            <Chip
              size="small"
              label="Active"
              color={!noteArchiveFilter ? "primary" : "default"}
              onClick={() => setNoteArchiveFilter(false)}
            />
            <Chip
              size="small"
              label="Archived"
              color={noteArchiveFilter ? "primary" : "default"}
              onClick={() => setNoteArchiveFilter(true)}
            />
          </Box>
        )}

        {activeTab === "ideas" && (
          <Box sx={{ display: "flex", gap: 0.75 }}>
            {["active", "converted", "ALL"].map((st) => (
              <Chip
                key={st}
                size="small"
                label={st === "active" ? "Active Ideas" : st === "converted" ? "Converted to Tasks" : "All"}
                color={ideaStatusFilter === st ? "primary" : "default"}
                onClick={() => setIdeaStatusFilter(st)}
              />
            ))}
          </Box>
        )}
      </Box>

      {/* Main Content Area */}
      <Box sx={{ flexGrow: 1, overflowY: "auto", px: 2, pb: 10 }}>
        {/* TASKS LIST */}
        {activeTab === "tasks" && (
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
        )}

        {/* REMINDERS LIST */}
        {activeTab === "reminders" && (
          <Box sx={{ display: "flex", flexDirection: "column", gap: 1, pt: 1 }}>
            {reminders.length === 0 ? (
              <Typography color="text.secondary" sx={{ py: 6, textAlign: "center", fontSize: "0.9rem" }}>
                No reminders scheduled. Tap + to set a reminder.
              </Typography>
            ) : (
              reminders.map((r) => (
                <Card key={r.id} variant="outlined" sx={{ borderRadius: 2.5, borderColor: "#EEEEEE" }}>
                  <CardContent sx={{ py: 1.5, px: 2, "&:last-child": { pb: 1.5 } }}>
                    <Box sx={{ display: "flex", justifyContent: "space-between", alignItems: "flex-start" }}>
                      <Box>
                        <Typography
                          variant="subtitle1"
                          sx={{
                            fontWeight: 600,
                            fontSize: "0.95rem",
                            textDecoration: r.status === "handled" ? "line-through" : "none",
                            color: r.status === "handled" ? "text.secondary" : "text.primary",
                          }}
                        >
                          {r.title}
                        </Typography>
                        <Typography
                          variant="caption"
                          sx={{
                            color: "primary.main",
                            fontWeight: 600,
                            display: "flex",
                            alignItems: "center",
                            gap: 0.5,
                            mt: 0.5,
                          }}
                        >
                          <AlarmRoundedIcon sx={{ fontSize: 14 }} />
                          {new Date(r.remind_at).toLocaleString()}
                        </Typography>
                      </Box>
                      <Box sx={{ display: "flex", alignItems: "center", gap: 0.5 }}>
                        <Button
                          size="small"
                          variant={r.status === "handled" ? "outlined" : "contained"}
                          onClick={() => void handleToggleReminderHandled(r)}
                          sx={{ textTransform: "none", fontSize: "0.75rem", px: 1.5, py: 0.25 }}
                        >
                          {r.status === "handled" ? "Reopen" : "Done"}
                        </Button>
                        <IconButton size="small" onClick={() => void handleDeleteReminder(r.id)}>
                          <DeleteOutlineRoundedIcon sx={{ fontSize: 18, color: "#808080" }} />
                        </IconButton>
                      </Box>
                    </Box>
                  </CardContent>
                </Card>
              ))
            )}
          </Box>
        )}

        {/* NOTES LIST */}
        {activeTab === "notes" && (
          <Box sx={{ display: "grid", gridTemplateColumns: "1fr", gap: 1.5, pt: 1 }}>
            {notes.length === 0 ? (
              <Typography color="text.secondary" sx={{ py: 6, textAlign: "center", fontSize: "0.9rem" }}>
                No notes found. Tap + to write a note.
              </Typography>
            ) : (
              notes.map((n) => (
                <Card
                  key={n.id}
                  variant="outlined"
                  sx={{
                    borderRadius: 2.5,
                    borderColor: "#EEEEEE",
                    cursor: "pointer",
                    "&:hover": { bgcolor: "#FAFAFA" },
                  }}
                  onClick={() => {
                    setEditingNote(n);
                    setNoteTitle(n.title);
                    setNoteContent(n.content);
                    setNoteTagsInput(n.tags ? n.tags.join(", ") : "");
                    setNoteModalOpen(true);
                  }}
                >
                  <CardContent sx={{ py: 1.5, px: 2, "&:last-child": { pb: 1.5 } }}>
                    <Box sx={{ display: "flex", justifyContent: "space-between", alignItems: "center" }}>
                      <Typography variant="subtitle1" sx={{ fontWeight: 600, fontSize: "0.95rem" }}>
                        {n.title}
                      </Typography>
                      <Box sx={{ display: "flex", gap: 0.5 }}>
                        <IconButton
                          size="small"
                          onClick={(e) => {
                            e.stopPropagation();
                            void handleToggleArchiveNote(n);
                          }}
                        >
                          {n.is_archived ? (
                            <UnarchiveOutlinedIcon sx={{ fontSize: 18 }} />
                          ) : (
                            <ArchiveOutlinedIcon sx={{ fontSize: 18 }} />
                          )}
                        </IconButton>
                        <IconButton
                          size="small"
                          onClick={(e) => {
                            e.stopPropagation();
                            void handleDeleteNote(n.id);
                          }}
                        >
                          <DeleteOutlineRoundedIcon sx={{ fontSize: 18, color: "#808080" }} />
                        </IconButton>
                      </Box>
                    </Box>
                    <Typography
                      variant="body2"
                      color="text.secondary"
                      sx={{
                        mt: 0.5,
                        fontSize: "0.85rem",
                        overflow: "hidden",
                        textOverflow: "ellipsis",
                        display: "-webkit-box",
                        WebkitLineClamp: 3,
                        WebkitBoxOrient: "vertical",
                      }}
                    >
                      {n.content || "(Empty note)"}
                    </Typography>
                    {n.tags && n.tags.length > 0 && (
                      <Box sx={{ display: "flex", gap: 0.5, mt: 1, flexWrap: "wrap" }}>
                        {n.tags.map((tag: string) => (

                          <Chip
                            key={tag}
                            label={`#${tag}`}
                            size="small"
                            sx={{ fontSize: "0.68rem", height: 20 }}
                          />
                        ))}
                      </Box>
                    )}
                  </CardContent>
                </Card>
              ))
            )}
          </Box>
        )}

        {/* IDEAS LIST */}
        {activeTab === "ideas" && (
          <Box sx={{ display: "flex", flexDirection: "column", gap: 1.5, pt: 1 }}>
            {ideas.length === 0 ? (
              <Typography color="text.secondary" sx={{ py: 6, textAlign: "center", fontSize: "0.9rem" }}>
                No ideas captured yet. Tap + to record an idea.
              </Typography>
            ) : (
              ideas.map((i) => (
                <Card key={i.id} variant="outlined" sx={{ borderRadius: 2.5, borderColor: "#EEEEEE" }}>
                  <CardContent sx={{ py: 1.5, px: 2 }}>
                    <Box sx={{ display: "flex", justifyContent: "space-between", alignItems: "flex-start" }}>
                      <Typography variant="subtitle1" sx={{ fontWeight: 600, fontSize: "0.95rem" }}>
                        {i.title}
                      </Typography>
                      <Chip
                        size="small"
                        label={i.status === "converted" ? "Converted to Task" : "Active Idea"}
                        color={i.status === "converted" ? "success" : "info"}
                        sx={{ fontSize: "0.7rem", height: 20 }}
                      />
                    </Box>
                    {i.description && (
                      <Typography variant="body2" color="text.secondary" sx={{ mt: 0.5, fontSize: "0.85rem" }}>
                        {i.description}
                      </Typography>
                    )}
                  </CardContent>
                  <CardActions sx={{ px: 2, pb: 1.5, pt: 0, justifyContent: "space-between" }}>
                    {i.status !== "converted" ? (
                      <Button
                        size="small"
                        variant="contained"
                        color="primary"
                        startIcon={<TransformRoundedIcon />}
                        onClick={() => void handleConvertIdea(i)}
                        sx={{ textTransform: "none", fontSize: "0.78rem", borderRadius: 2 }}
                      >
                        Convert to Task
                      </Button>
                    ) : (
                      <Typography variant="caption" color="text.secondary">
                        Already in tasks
                      </Typography>
                    )}
                    <Box>
                      <IconButton
                        size="small"
                        onClick={() => {
                          setEditingIdea(i);
                          setIdeaTitle(i.title);
                          setIdeaDesc(i.description);
                          setIdeaModalOpen(true);
                        }}
                      >
                        <EditOutlinedIcon sx={{ fontSize: 18 }} />
                      </IconButton>
                      <IconButton size="small" onClick={() => void handleDeleteIdea(i.id)}>
                        <DeleteOutlineRoundedIcon sx={{ fontSize: 18, color: "#808080" }} />
                      </IconButton>
                    </Box>
                  </CardActions>
                </Card>
              ))
            )}
          </Box>
        )}
      </Box>

      {/* FAB - Strict Todoist Red shadow */}
      <Fab
        color="primary"
        aria-label="Add item"
        onClick={handleFabClick}
        sx={{
          position: "fixed",
          right: 24,
          bottom: `calc(28px + env(safe-area-inset-bottom))`,
          boxShadow: "0 8px 24px rgba(220, 76, 62, 0.4)",
          bgcolor: "#DC4C3E",
          "&:hover": { bgcolor: "#B53C30" },
        }}
      >
        <AddRoundedIcon />
      </Fab>

      {/* TASK DIALOG */}
      <Dialog open={taskModalOpen} onClose={() => setTaskModalOpen(false)} fullWidth maxWidth="xs">
        <DialogTitle>{editingTask ? "Edit Task" : "New Task"}</DialogTitle>
        <DialogContent sx={{ display: "flex", flexDirection: "column", gap: 2, pt: 1 }}>
          <TextField
            autoFocus
            label="Task Name"
            fullWidth
            value={taskTitle}
            onChange={(e) => setTaskTitle(e.target.value)}
          />
          <TextField
            label="Description (optional)"
            fullWidth
            multiline
            rows={2}
            value={taskDesc}
            onChange={(e) => setTaskDesc(e.target.value)}
          />
          <FormControl fullWidth size="small">
            <InputLabel>Priority</InputLabel>
            <Select
              value={taskPriority}
              label="Priority"
              onChange={(e) => setTaskPriority(e.target.value)}
            >
              <MenuItem value="P1">P1 - High (Red)</MenuItem>
              <MenuItem value="P2">P2 - Medium (Orange)</MenuItem>
              <MenuItem value="P3">P3 - Low (Blue)</MenuItem>
              <MenuItem value="P4">P4 - None (Gray)</MenuItem>
            </Select>
          </FormControl>
          <TextField
            label="Due Date"
            type="datetime-local"
            fullWidth
            slotProps={{ inputLabel: { shrink: true } }}
            value={taskDueDate}
            onChange={(e) => setTaskDueDate(e.target.value)}
          />
          <TextField
            label="Project"
            fullWidth
            value={taskProject}
            onChange={(e) => setTaskProject(e.target.value)}
          />
        </DialogContent>
        <DialogActions>
          <Button onClick={() => setTaskModalOpen(false)}>Cancel</Button>
          <Button variant="contained" onClick={() => void handleSaveTask()}>
            Save
          </Button>
        </DialogActions>
      </Dialog>

      {/* REMINDER DIALOG */}
      <Dialog open={reminderModalOpen} onClose={() => setReminderModalOpen(false)} fullWidth maxWidth="xs">
        <DialogTitle>{editingReminder ? "Edit Reminder" : "New Reminder"}</DialogTitle>
        <DialogContent sx={{ display: "flex", flexDirection: "column", gap: 2, pt: 1 }}>
          <TextField
            autoFocus
            label="Reminder Title"
            fullWidth
            value={reminderTitle}
            onChange={(e) => setReminderTitle(e.target.value)}
          />
          <TextField
            label="Date & Time"
            type="datetime-local"
            fullWidth
            slotProps={{ inputLabel: { shrink: true } }}
            value={reminderTime}
            onChange={(e) => setReminderTime(e.target.value)}
          />
        </DialogContent>
        <DialogActions>
          <Button onClick={() => setReminderModalOpen(false)}>Cancel</Button>
          <Button variant="contained" onClick={() => void handleSaveReminder()}>
            Save
          </Button>
        </DialogActions>
      </Dialog>

      {/* NOTE DIALOG */}
      <Dialog open={noteModalOpen} onClose={() => setNoteModalOpen(false)} fullWidth maxWidth="xs">
        <DialogTitle>{editingNote ? "Edit Note" : "New Note"}</DialogTitle>
        <DialogContent sx={{ display: "flex", flexDirection: "column", gap: 2, pt: 1 }}>
          <TextField
            autoFocus
            label="Title"
            fullWidth
            value={noteTitle}
            onChange={(e) => setNoteTitle(e.target.value)}
          />
          <TextField
            label="Content"
            fullWidth
            multiline
            rows={5}
            value={noteContent}
            onChange={(e) => setNoteContent(e.target.value)}
          />
          <TextField
            label="Tags (comma separated)"
            fullWidth
            value={noteTagsInput}
            onChange={(e) => setNoteTagsInput(e.target.value)}
          />
        </DialogContent>
        <DialogActions>
          <Button onClick={() => setNoteModalOpen(false)}>Cancel</Button>
          <Button variant="contained" onClick={() => void handleSaveNote()}>
            Save
          </Button>
        </DialogActions>
      </Dialog>

      {/* IDEA DIALOG */}
      <Dialog open={ideaModalOpen} onClose={() => setIdeaModalOpen(false)} fullWidth maxWidth="xs">
        <DialogTitle>{editingIdea ? "Edit Idea" : "Capture Idea"}</DialogTitle>
        <DialogContent sx={{ display: "flex", flexDirection: "column", gap: 2, pt: 1 }}>
          <TextField
            autoFocus
            label="Idea Title"
            fullWidth
            value={ideaTitle}
            onChange={(e) => setIdeaTitle(e.target.value)}
          />
          <TextField
            label="Details (optional)"
            fullWidth
            multiline
            rows={3}
            value={ideaDesc}
            onChange={(e) => setIdeaDesc(e.target.value)}
          />
        </DialogContent>
        <DialogActions>
          <Button onClick={() => setIdeaModalOpen(false)}>Cancel</Button>
          <Button variant="contained" onClick={() => void handleSaveIdea()}>
            Save
          </Button>
        </DialogActions>
      </Dialog>
    </Box>
  );
}
