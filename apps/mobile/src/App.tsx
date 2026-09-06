/**
 * Application shell.
 *
 * There is no navigation state, because there is no navigation. Home fills the
 * screen; everything else is a sheet opened from here or from Home.
 *
 * The text conversation is mounted here rather than inside Home because it is
 * not part of the voice interface: it is the surface that exists until the
 * microphone does. When voice works, this button is what goes away, and Home is
 * untouched by its removal.
 */

import { useState } from "react";
import Box from "@mui/material/Box";
import Fab from "@mui/material/Fab";
import ChatBubbleOutlineRoundedIcon from "@mui/icons-material/ChatBubbleOutlineRounded";

import HomeScreen from "./screens/HomeScreen";
import ConversationSheet from "./components/ConversationSheet";

export default function App() {
  const [chatOpen, setChatOpen] = useState(false);

  return (
    <Box
      sx={{
        height: "100dvh",
        bgcolor: "background.default",
        px: 3,
        pt: "env(safe-area-inset-top)",
        pb: "env(safe-area-inset-bottom)",
      }}
    >
      <HomeScreen />

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

      <ConversationSheet open={chatOpen} onClose={() => setChatOpen(false)} />
    </Box>
  );
}
