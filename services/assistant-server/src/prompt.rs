//! The assistant's system prompt.
//!
//! It lives on the server, as a constant, for one reason: a client that could
//! supply a system prompt could rewrite the assistant's instructions, and the
//! permission architecture assumes the model is being instructed by the
//! operator rather than by whoever is holding the phone. Nothing on the wire
//! reaches this string -- [`ClientFrame`](assistant_protocol::ClientFrame) has
//! no field that could.
//!
//! It is deliberately short. This is not the assistant's personality; it is the
//! minimum needed to stop the model claiming things that are not true about the
//! system it is running inside. Personality is a later, separate decision, and
//! writing one now would mean rewriting it once the assistant actually does
//! something.

/// The instructions sent with every model turn.
pub const SYSTEM_PROMPT: &str = "\
You are a personal assistant running on the user's own server.

Be concise and substantive. Answer the question that was asked, in as few words \
as it genuinely takes; skip preamble and restatement.

Be truthful about what you have and have not done. Never say you have sent, \
saved, scheduled, or looked something up unless a tool result in this \
conversation shows it happened. If a tool call was proposed but not run, say \
that it is waiting for approval rather than describing the result it would \
have had.

Use the tools you are given when they fit the request, and no others. You do \
not decide what you are permitted to do: the server evaluates every tool call \
against its own policy, and some will require the user's approval before they \
run. That is expected -- do not argue with it, work around it, or ask the user \
to grant permissions.

Distinguish what you know from what you are guessing. When you are unsure, say \
so plainly.

Do not invent personal details about the user. What you know about them is what \
is in this conversation.

The tools offered on this turn are exactly the integrations this deployment has \
connected. If a request needs one that is not offered, say it is not available \
rather than pretending to check it -- and if a request needs one that is, call \
it rather than claiming it is unavailable.";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_prompt_is_short_enough_to_be_read_before_it_is_changed() {
        // Not a style rule. A system prompt is sent on every turn, so its
        // length is a per-turn cost, and a prompt nobody rereads is one nobody
        // notices has drifted out of line with what the system does.
        assert!(
            SYSTEM_PROMPT.len() < 2_000,
            "system prompt has grown to {} bytes",
            SYSTEM_PROMPT.len()
        );
    }

    /// A constant cannot know what this deployment wired up. It used to assert
    /// "email, calendar and files are not connected yet", which stayed in the
    /// prompt after those tools were registered -- so the model refused to call
    /// a calendar tool it had been handed, and told the user the calendar was
    /// not connected while a live Google account sat behind it.
    #[test]
    fn the_prompt_does_not_hardcode_which_integrations_exist() {
        let lowered = SYSTEM_PROMPT.to_lowercase();
        assert!(
            !lowered.contains("not connected yet"),
            "the tool list decides what is connected; the prompt must not restate it"
        );
        assert!(
            lowered.contains("not offered"),
            "the prompt must point the model at the offered tools as the source of truth"
        );
    }
}
