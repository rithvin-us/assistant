//! Google ecosystem integrations and personal schedule foundation.
//!
//! Each provider gets its own normaliser. `client` owns OAuth, credential
//! decryption and token refresh; `classroom` and `drive` borrow the pool, the
//! HTTP client and an access token from it and translate Google's JSON into
//! the provider-neutral types in `assistant-protocol`.

pub mod classroom;
pub mod client;
pub mod drive;
pub mod free_time;

pub use client::GoogleClient;
pub use free_time::{calculate_free_slots, find_free_slots};

pub(crate) use client::url_encode;

use assistant_tools::ToolError;

/// Turns a failed Google response into an error a user can be shown.
///
/// Google's error bodies quote the request back, which for these APIs can
/// include a course, a file name or a search query. None of that belongs in a
/// message rendered on a phone or written to a log, so the status is mapped to
/// a sentence and the body is dropped rather than forwarded. See ADR-0032.
///
/// The status itself is kept because the four cases below need genuinely
/// different actions from the user: reconnect, ask an administrator, wait, or
/// nothing.
pub(super) fn api_error(api: &str, status: reqwest::StatusCode) -> ToolError {
    let message = match status.as_u16() {
        401 => format!("This Google account needs to be reconnected before {api} can be used."),
        403 => format!(
            "This Google account does not have access to {api}. If it is a school \
             or work account, an administrator may need to allow it, or the account \
             may need to be reconnected to grant the newer permissions."
        ),
        404 => format!("That {api} item no longer exists, or this account cannot see it."),
        429 => format!("{api} is rate limiting these requests. Try again shortly."),
        500..=599 => format!("{api} is temporarily unavailable."),
        _ => format!("{api} rejected the request."),
    };
    ToolError::Failed(message)
}
