/**
 * Application shell.
 *
 * There is no navigation state, because there is no navigation. Home fills the
 * screen; everything else is a sheet Home opens. When a second real destination
 * exists, add it to the sheet rather than reintroducing a permanent bar.
 */

import Box from "@mui/material/Box";

import HomeScreen from "./screens/HomeScreen";

export default function App() {
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
    </Box>
  );
}
