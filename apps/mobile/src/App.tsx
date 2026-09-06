/**
 * Application shell.
 *
 * Navigation state is the only state React owns here. Every screen except Home
 * is an explicit placeholder: showing an empty labelled surface is honest,
 * whereas mock data would make an unbuilt feature look finished.
 */

import { useState } from "react";
import Box from "@mui/material/Box";
import BottomNavigation from "@mui/material/BottomNavigation";
import BottomNavigationAction from "@mui/material/BottomNavigationAction";
import Paper from "@mui/material/Paper";
import HomeRoundedIcon from "@mui/icons-material/HomeRounded";
import ChecklistRoundedIcon from "@mui/icons-material/ChecklistRounded";
import PsychologyRoundedIcon from "@mui/icons-material/PsychologyRounded";
import SettingsRoundedIcon from "@mui/icons-material/SettingsRounded";

import HomeScreen from "./screens/HomeScreen";
import NotBuiltYet from "./screens/NotBuiltYet";

type Tab = "home" | "tasks" | "memory" | "settings";

export default function App() {
  const [tab, setTab] = useState<Tab>("home");

  return (
    <Box
      sx={{
        display: "flex",
        flexDirection: "column",
        height: "100dvh",
        bgcolor: "background.default",
      }}
    >
      <Box component="main" sx={{ flex: 1, overflowY: "auto", px: 2, pt: 3, pb: 2 }}>
        {tab === "home" && <HomeScreen />}
        {tab === "tasks" && (
          <NotBuiltYet
            title="Tasks"
            milestone="Tasks, deadlines and reminders arrive with the schedule milestone."
          />
        )}
        {tab === "memory" && (
          <NotBuiltYet
            title="Memory"
            milestone="Important memories, projects and archive arrive with the memory milestone."
          />
        )}
        {tab === "settings" && (
          <NotBuiltYet
            title="Settings"
            milestone="Connected accounts, notification budget and voice settings arrive later."
          />
        )}
      </Box>

      <Paper
        square
        sx={{
          borderLeft: 0,
          borderRight: 0,
          borderBottom: 0,
          pb: "env(safe-area-inset-bottom)",
        }}
      >
        <BottomNavigation
          value={tab}
          onChange={(_event, next: Tab) => setTab(next)}
          showLabels
        >
          <BottomNavigationAction value="home" label="Home" icon={<HomeRoundedIcon />} />
          <BottomNavigationAction value="tasks" label="Tasks" icon={<ChecklistRoundedIcon />} />
          <BottomNavigationAction value="memory" label="Memory" icon={<PsychologyRoundedIcon />} />
          <BottomNavigationAction
            value="settings"
            label="Settings"
            icon={<SettingsRoundedIcon />}
          />
        </BottomNavigation>
      </Paper>
    </Box>
  );
}
