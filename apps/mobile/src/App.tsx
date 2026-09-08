/**
 * Application shell.
 *
 * Minimalist container that displays the primary assistant home,
 * productivity screens, and Google ecosystem features.
 */

import { useCallback, useState } from "react";
import Box from "@mui/material/Box";
import Fab from "@mui/material/Fab";
import ChatBubbleOutlineRoundedIcon from "@mui/icons-material/ChatBubbleOutlineRounded";

import ScreenTransition from "./components/ScreenTransition";
import { useAndroidBack } from "./lib/useAndroidBack";

import HomeScreen from "./screens/HomeScreen";
import TasksScreen from "./screens/TasksScreen";
import RemindersScreen from "./screens/RemindersScreen";
import NotesScreen from "./screens/NotesScreen";
import IdeasScreen from "./screens/IdeasScreen";
import ConnectionsScreen from "./screens/ConnectionsScreen";
import CalendarScreen from "./screens/CalendarScreen";
import GmailScreen from "./screens/GmailScreen";
import ClassroomScreen from "./screens/ClassroomScreen";
import DriveScreen from "./screens/DriveScreen";
import AcademicScreen from "./screens/AcademicScreen";
import MemoryScreen from "./screens/MemoryScreen";
import DocumentsScreen from "./screens/DocumentsScreen";
import { PlanningScreen } from "./screens/PlanningScreen";
import ConversationSheet from "./components/ConversationSheet";

export type ScreenType =
  | "home"
  | "tasks"
  | "reminders"
  | "notes"
  | "ideas"
  | "connections"
  | "calendar"
  | "gmail"
  | "classroom"
  | "drive"
  | "academic"
  | "memory"
  | "documents"
  | "planning";

export default function App() {
  const [chatOpen, setChatOpen] = useState(false);
  const [currentScreen, setCurrentScreen] = useState<ScreenType>("home");

  const goHome = useCallback(() => setCurrentScreen("home"), []);

  // Android's back button and back gesture reach us as one event. On a
  // sub-screen they used to leave the app, which reads as a crash; now they
  // return to the home screen. From home, back still exits, because that is
  // what the user means there.
  useAndroidBack(currentScreen !== "home", goHome);

  // A sheet is the shallowest thing on screen, so back should close it before
  // it touches navigation.
  useAndroidBack(chatOpen, () => setChatOpen(false));

  return (
    <Box
      sx={{
        height: "100dvh",
        width: "100%",
        bgcolor: "background.default",
        px: 0,
        pt: "env(safe-area-inset-top)",
        pb: "env(safe-area-inset-bottom)",
        overflowX: "hidden",
      }}
    >
      <ScreenTransition screenKey={currentScreen} back={currentScreen === "home"}>
      {currentScreen === "home" && (
        <HomeScreen onOpenScreen={(screen) => setCurrentScreen(screen)} />
      )}
      {currentScreen === "tasks" && (
        <TasksScreen onBack={() => setCurrentScreen("home")} />
      )}
      {currentScreen === "reminders" && (
        <RemindersScreen onBack={() => setCurrentScreen("home")} />
      )}
      {currentScreen === "notes" && (
        <NotesScreen onBack={() => setCurrentScreen("home")} />
      )}
      {currentScreen === "ideas" && (
        <IdeasScreen onBack={() => setCurrentScreen("home")} />
      )}
      {currentScreen === "connections" && (
        <ConnectionsScreen onBack={() => setCurrentScreen("home")} />
      )}
      {currentScreen === "calendar" && (
        <CalendarScreen onBack={() => setCurrentScreen("home")} />
      )}
      {currentScreen === "gmail" && (
        <GmailScreen onBack={() => setCurrentScreen("home")} />
      )}
      {currentScreen === "classroom" && (
        <ClassroomScreen
          onBack={() => setCurrentScreen("home")}
          onOpenConnections={() => setCurrentScreen("connections")}
        />
      )}
      {currentScreen === "drive" && (
        <DriveScreen
          onBack={() => setCurrentScreen("home")}
          onOpenConnections={() => setCurrentScreen("connections")}
        />
      )}
      {currentScreen === "academic" && (
        <AcademicScreen
          onBack={() => setCurrentScreen("home")}
          onOpenClassroom={() => setCurrentScreen("classroom")}
        />
      )}
      {currentScreen === "memory" && (
        <MemoryScreen onBack={() => setCurrentScreen("home")} />
      )}
      {currentScreen === "documents" && (
        <DocumentsScreen onBack={() => setCurrentScreen("home")} />
      )}
      {currentScreen === "planning" && <PlanningScreen />}
      </ScreenTransition>

      {currentScreen === "home" && (
        <Fab
          aria-label="Type a message"
          size="medium"
          onClick={() => setChatOpen(true)}
          sx={{
            position: "fixed",
            right: 20,
            bottom: `calc(24px + env(safe-area-inset-bottom))`,
            bgcolor: "background.paper",
            color: "text.secondary",
            boxShadow: 3,
          }}
        >
          <ChatBubbleOutlineRoundedIcon />
        </Fab>
      )}

      <ConversationSheet open={chatOpen} onClose={() => setChatOpen(false)} />
    </Box>
  );
}
