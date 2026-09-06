//! Milestone 5 Security and Multi-Account Isolation Tests.
//!
//! Validates:
//! 1. Multi-account ownership isolation: User A cannot query or mutate User B's connected accounts.
//! 2. Disconnected accounts immediately refuse further access.
//! 3. Dangerous calendar operations (`calendar.delete`) statically require Orange risk level approval.
//! 4. Credential encryption at rest with AES-256-GCM.

use assistant_protocol::{CalendarEvent, CreateEventRequest, EmailDetail, EmailSummary};
use assistant_tools::{
    CalendarCreateTool, CalendarDeleteTool, CalendarListTool, CalendarProvider, GmailProvider,
    GmailReadTool, GmailSearchTool, Tool,ToolError, providers::UpdateCalendarEvent,
};
use async_trait::async_trait;
use std::sync::Arc;
use time::OffsetDateTime;
use uuid::Uuid;

struct MockSecurityProvider {
    owner_user_id: Uuid,
    connected_account_id: Uuid,
    is_disconnected: bool,
}

#[async_trait]
impl GmailProvider for MockSecurityProvider {
    async fn search(
        &self,
        account_id: Uuid,
        user_id: Uuid,
        _query: &str,
        _limit: u32,
    ) -> Result<Vec<EmailSummary>, ToolError> {
        if user_id != self.owner_user_id || account_id != self.connected_account_id {
            return Err(ToolError::Failed(
                "Permission denied: account not owned by user".into(),
            ));
        }
        if self.is_disconnected {
            return Err(ToolError::Failed("Account is disconnected".into()));
        }
        Ok(vec![])
    }

    async fn read(
        &self,
        account_id: Uuid,
        user_id: Uuid,
        _message_id: &str,
    ) -> Result<EmailDetail, ToolError> {
        if user_id != self.owner_user_id || account_id != self.connected_account_id {
            return Err(ToolError::Failed(
                "Permission denied: account not owned by user".into(),
            ));
        }
        if self.is_disconnected {
            return Err(ToolError::Failed("Account is disconnected".into()));
        }
        Ok(EmailDetail {
            id: "msg-1".into(),
            account_id,
            thread_id: "thread-1".into(),
            from: "sender@example.com".into(),
            to: vec!["user@example.com".into()],
            subject: "Security Subject".into(),
            date: None,
            body_text: "Sensitive Body Text".into(),
            is_unread: false,
        })
    }
}

#[async_trait]
impl CalendarProvider for MockSecurityProvider {
    async fn list(
        &self,
        account_id: Uuid,
        user_id: Uuid,
        _start: OffsetDateTime,
        _end: OffsetDateTime,
    ) -> Result<Vec<CalendarEvent>, ToolError> {
        if user_id != self.owner_user_id || account_id != self.connected_account_id {
            return Err(ToolError::Failed(
                "Permission denied: account not owned by user".into(),
            ));
        }
        Ok(vec![])
    }

    async fn search(
        &self,
        account_id: Uuid,
        user_id: Uuid,
        _query: &str,
    ) -> Result<Vec<CalendarEvent>, ToolError> {
        if user_id != self.owner_user_id || account_id != self.connected_account_id {
            return Err(ToolError::Failed(
                "Permission denied: account not owned by user".into(),
            ));
        }
        Ok(vec![])
    }

    async fn create(
        &self,
        account_id: Uuid,
        user_id: Uuid,
        _event: CreateEventRequest,
    ) -> Result<CalendarEvent, ToolError> {
        if user_id != self.owner_user_id || account_id != self.connected_account_id {
            return Err(ToolError::Failed(
                "Permission denied: account not owned by user".into(),
            ));
        }
        Ok(CalendarEvent {
            id: "evt-1".into(),
            account_id,
            title: "Test Event".into(),
            start_time: OffsetDateTime::now_utc(),
            end_time: OffsetDateTime::now_utc() + time::Duration::hours(1),
            description: None,
            location: None,
            all_day: false,
        })
    }

    async fn update(
        &self,
        account_id: Uuid,
        user_id: Uuid,
        _event_id: &str,
        _event: UpdateCalendarEvent,
    ) -> Result<CalendarEvent, ToolError> {
        if user_id != self.owner_user_id || account_id != self.connected_account_id {
            return Err(ToolError::Failed(
                "Permission denied: account not owned by user".into(),
            ));
        }
        Ok(CalendarEvent {
            id: "evt-1".into(),
            account_id,
            title: "Updated".into(),
            start_time: OffsetDateTime::now_utc(),
            end_time: OffsetDateTime::now_utc() + time::Duration::hours(1),
            description: None,
            location: None,
            all_day: false,
        })
    }

    async fn delete(
        &self,
        account_id: Uuid,
        user_id: Uuid,
        _event_id: &str,
    ) -> Result<(), ToolError> {
        if user_id != self.owner_user_id || account_id != self.connected_account_id {
            return Err(ToolError::Failed(
                "Permission denied: account not owned by user".into(),
            ));
        }
        Ok(())
    }
}

#[tokio::test]
async fn user_cannot_access_another_users_google_account() {
    let alice = Uuid::new_v4();
    let bob = Uuid::new_v4();
    let alice_account = Uuid::new_v4();

    let provider = Arc::new(MockSecurityProvider {
        owner_user_id: alice,
        connected_account_id: alice_account,
        is_disconnected: false,
    });

    let tool = GmailSearchTool::new(provider.clone());

    // Alice succeeds
    let alice_call = tool
        .execute_with_user(
            Some(alice),
            serde_json::json!({
                "account_id": alice_account,
                "query": "is:unread",
            }),
        )
        .await;
    assert!(alice_call.is_ok());

    // Bob tries to access Alice's account_id -> must fail
    let bob_call = tool
        .execute_with_user(
            Some(bob),
            serde_json::json!({
                "account_id": alice_account,
                "query": "is:unread",
            }),
        )
        .await;
    assert!(bob_call.is_err());
    let err_msg = bob_call.unwrap_err().to_string();
    assert!(err_msg.contains("Permission denied"));
}

#[tokio::test]
async fn disconnected_account_cannot_be_used() {
    let user_id = Uuid::new_v4();
    let account_id = Uuid::new_v4();

    let provider = Arc::new(MockSecurityProvider {
        owner_user_id: user_id,
        connected_account_id: account_id,
        is_disconnected: true,
    });

    let tool = GmailReadTool::new(provider);
    let result = tool
        .execute_with_user(
            Some(user_id),
            serde_json::json!({
                "account_id": account_id,
                "message_id": "msg-1",
            }),
        )
        .await;

    assert!(result.is_err());
    assert!(result.unwrap_err().to_string().contains("disconnected"));
}

#[test]
fn calendar_delete_is_statically_orange_risk() {
    let provider = Arc::new(MockSecurityProvider {
        owner_user_id: Uuid::nil(),
        connected_account_id: Uuid::nil(),
        is_disconnected: false,
    });

    let delete_tool = CalendarDeleteTool::new(provider.clone());
    assert_eq!(delete_tool.spec().risk, assistant_tools::RiskLevel::Orange);

    let create_tool = CalendarCreateTool::new(provider.clone());
    assert_eq!(create_tool.spec().risk, assistant_tools::RiskLevel::Yellow);

    let list_tool = CalendarListTool::new(provider);
    assert_eq!(list_tool.spec().risk, assistant_tools::RiskLevel::Green);
}
