/**
 * Google account selector.
 *
 * Accounts are never merged and never inferred. The user picks one, sees which
 * one is active, and can switch. Two accounts belonging to the same person are
 * two enrolments, not duplicates to be folded together.
 *
 * It also answers a question the user would otherwise have to guess at: an
 * account connected before this feature existed did not grant Classroom or
 * Drive permission, and every request against it will fail. Rather than
 * letting the screen show an unexplained error, the chip says the account
 * needs reconnecting.
 */

import Box from "@mui/material/Box";
import Chip from "@mui/material/Chip";
import Typography from "@mui/material/Typography";
import Alert from "@mui/material/Alert";
import Button from "@mui/material/Button";

import type { AccountSummary } from "../api/types";
import { hasScopesFor, type ScopedFeature } from "../api/scopes";

interface Props {
  accounts: AccountSummary[];
  selectedId: string | null;
  onSelect: (id: string) => void;
  /** When set, accounts lacking this feature's scopes are flagged. */
  feature?: ScopedFeature;
  onOpenConnections?: () => void;
}

export default function AccountPicker({
  accounts,
  selectedId,
  onSelect,
  feature,
  onOpenConnections,
}: Props) {
  const selected = accounts.find((a) => a.id === selectedId);
  const missingScopes = feature ? !hasScopesFor(selected, feature) : false;

  if (accounts.length === 0) {
    return (
      <Alert
        severity="info"
        sx={{ mx: 2, my: 1.5 }}
        action={
          onOpenConnections && (
            <Button size="small" onClick={onOpenConnections}>
              Connect
            </Button>
          )
        }
      >
        No Google account is connected yet.
      </Alert>
    );
  }

  return (
    <Box>
      <Box
        sx={{
          display: "flex",
          gap: 1,
          px: 2,
          py: 1.5,
          overflowX: "auto",
          // A horizontal strip keeps several accounts reachable with a thumb
          // without a dropdown that hides which one is active.
          "&::-webkit-scrollbar": { display: "none" },
        }}
      >
        {accounts.map((account) => (
          <Chip
            key={account.id}
            label={account.email}
            onClick={() => onSelect(account.id)}
            color={account.id === selectedId ? "primary" : "default"}
            variant={account.id === selectedId ? "filled" : "outlined"}
            size="small"
            sx={{ flexShrink: 0, fontWeight: account.id === selectedId ? 600 : 400 }}
          />
        ))}
      </Box>

      {selected && selected.status !== "active" && (
        <Alert severity="warning" sx={{ mx: 2, mb: 1.5 }}>
          This account is {selected.status}. Reconnect it to sync again —
          anything already imported stays where it is.
        </Alert>
      )}

      {selected && selected.status === "active" && missingScopes && (
        <Alert
          severity="warning"
          sx={{ mx: 2, mb: 1.5 }}
          action={
            onOpenConnections && (
              <Button size="small" onClick={onOpenConnections}>
                Reconnect
              </Button>
            )
          }
        >
          {selected.email} was connected before {feature === "drive" ? "Drive" : "Classroom"}{" "}
          access was added. Reconnect it to grant the new permission.
        </Alert>
      )}

      {selected && (
        <Typography
          variant="caption"
          sx={{ px: 2, color: "text.secondary", display: "block" }}
        >
          {selected.display_name ?? selected.email}
        </Typography>
      )}
    </Box>
  );
}
