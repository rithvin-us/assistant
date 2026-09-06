/**
 * Placeholder for a screen whose milestone has not been reached.
 *
 * It states plainly that the feature does not exist. Mock content here would
 * misrepresent progress, which is exactly what the project is meant to avoid.
 */

import Box from "@mui/material/Box";
import Typography from "@mui/material/Typography";

export default function NotBuiltYet({
  title,
  milestone,
}: {
  title: string;
  milestone: string;
}) {
  return (
    <Box sx={{ display: "flex", flexDirection: "column", gap: 1.5 }}>
      <Typography variant="h1">{title}</Typography>
      <Typography variant="body2" color="text.secondary">
        Not built yet. {milestone}
      </Typography>
    </Box>
  );
}
