/**
 * Material UI theme.
 *
 * The product is voice-first and read at a glance, often one-handed. That drives
 * three choices: a dark ground so the screen is usable at night without
 * flashing, generous touch targets, and flat surfaces instead of elevation
 * shadows so density does not turn into visual noise.
 */

import { createTheme } from "@mui/material/styles";

export const theme = createTheme({
  colorSchemes: { dark: true },
  defaultColorScheme: "dark",
  palette: {
    mode: "dark",
    background: { default: "#0e1013", paper: "#16191e" },
    primary: { main: "#7aa2f7" },
    // Reserved for the states the attention engine will drive.
    warning: { main: "#e0af68" },
    error: { main: "#f7768e" },
    success: { main: "#9ece6a" },
    divider: "rgba(255,255,255,0.08)",
  },
  shape: { borderRadius: 12 },
  typography: {
    fontFamily: [
      "Inter",
      "system-ui",
      "-apple-system",
      "Segoe UI",
      "Roboto",
      "sans-serif",
    ].join(","),
    h1: { fontSize: "1.5rem", fontWeight: 600, letterSpacing: "-0.01em" },
    h2: { fontSize: "1.125rem", fontWeight: 600 },
    body2: { lineHeight: 1.5 },
    button: { textTransform: "none", fontWeight: 600 },
  },
  components: {
    MuiPaper: {
      // Elevation overlays make dense lists muddy; a hairline border separates
      // surfaces more cleanly at this information density.
      defaultProps: { elevation: 0 },
      styleOverrides: {
        root: { backgroundImage: "none", border: "1px solid rgba(255,255,255,0.06)" },
      },
    },
    MuiButton: {
      defaultProps: { disableElevation: true },
      styleOverrides: { root: { minHeight: 44 } },
    },
    MuiListItemButton: { styleOverrides: { root: { minHeight: 48 } } },
  },
});
