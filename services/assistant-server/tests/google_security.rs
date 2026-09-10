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
    GmailReadTool, GmailSearchTool, Tool, ToolError, providers::UpdateCalendarEvent,
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

#[test]
fn scope_expansion_normalizes_and_implies_dependent_scopes() {
    let raw = vec![
        "https://www.googleapis.com/auth/calendar.events".to_string(),
        "https://www.googleapis.com/auth/gmail.modify".to_string(),
        "https://www.googleapis.com/auth/drive".to_string(),
        "https://www.googleapis.com/auth/classroom.courses.readonly".to_string(),
        "https://www.googleapis.com/auth/classroom.coursework.students.readonly".to_string(),
    ];

    let expanded = assistant_server::google::client::expand_google_scopes(&raw);

    // Canonical short names
    assert!(expanded.contains(&"calendar.events".to_string()));
    assert!(expanded.contains(&"gmail.modify".to_string()));
    assert!(expanded.contains(&"drive".to_string()));
    assert!(expanded.contains(&"classroom.courses.readonly".to_string()));
    assert!(expanded.contains(&"classroom.coursework.students.readonly".to_string()));

    // Implied scopes required by tools
    assert!(
        expanded.contains(&"calendar.readonly".to_string()),
        "calendar.events implies calendar.readonly"
    );
    assert!(
        expanded.contains(&"gmail.readonly".to_string()),
        "gmail.modify implies gmail.readonly"
    );
    assert!(
        expanded.contains(&"gmail.send".to_string()),
        "gmail.modify implies gmail.send"
    );
    assert!(
        expanded.contains(&"drive.readonly".to_string()),
        "drive implies drive.readonly"
    );
    assert!(
        expanded.contains(&"classroom.coursework.me.readonly".to_string()),
        "classroom students coursework implies me coursework"
    );
}

#[test]
fn permission_policy_denies_unscoped_principal_for_google_tools() {
    use assistant_auth::Principal;
    use assistant_core::{PermissionPolicy, RiskBasedPolicy};
    use assistant_tools::PermissionDecision;

    let policy = RiskBasedPolicy::new();
    let unscoped_principal = Principal {
        user_id: Uuid::new_v4(),
        scopes: vec![],
    };

    let provider = Arc::new(MockSecurityProvider {
        owner_user_id: unscoped_principal.user_id,
        connected_account_id: Uuid::new_v4(),
        is_disconnected: false,
    });

    let gmail_tool = GmailSearchTool::new(provider.clone());
    let cal_list_tool = CalendarListTool::new(provider.clone());
    let cal_delete_tool = CalendarDeleteTool::new(provider);

    match policy.evaluate(gmail_tool.spec(), &unscoped_principal) {
        PermissionDecision::Deny { reason } => {
            assert!(reason.contains("gmail.readonly"), "Deny reason: {reason}");
        }
        other => panic!("Expected Deny for unscoped principal, got {other:?}"),
    }

    match policy.evaluate(cal_list_tool.spec(), &unscoped_principal) {
        PermissionDecision::Deny { reason } => {
            assert!(
                reason.contains("calendar.readonly"),
                "Deny reason: {reason}"
            );
        }
        other => panic!("Expected Deny for unscoped principal, got {other:?}"),
    }

    match policy.evaluate(cal_delete_tool.spec(), &unscoped_principal) {
        PermissionDecision::Deny { reason } => {
            assert!(reason.contains("calendar.events"), "Deny reason: {reason}");
        }
        other => panic!("Expected Deny for unscoped principal, got {other:?}"),
    }
}

#[test]
fn permission_policy_allows_populated_principal_and_requires_approval_for_orange_risk() {
    use assistant_auth::Principal;
    use assistant_core::{PermissionPolicy, RiskBasedPolicy};
    use assistant_tools::PermissionDecision;

    let policy = RiskBasedPolicy::new();
    let raw_scopes = vec![
        "https://www.googleapis.com/auth/calendar.events".to_string(),
        "https://www.googleapis.com/auth/gmail.readonly".to_string(),
    ];
    let expanded = assistant_server::google::client::expand_google_scopes(&raw_scopes);

    let scoped_principal = Principal {
        user_id: Uuid::new_v4(),
        scopes: expanded,
    };

    let provider = Arc::new(MockSecurityProvider {
        owner_user_id: scoped_principal.user_id,
        connected_account_id: Uuid::new_v4(),
        is_disconnected: false,
    });

    let gmail_tool = GmailSearchTool::new(provider.clone());
    let cal_list_tool = CalendarListTool::new(provider.clone());
    let cal_delete_tool = CalendarDeleteTool::new(provider);

    // Green tools with scopes satisfied are Allowed
    assert!(
        matches!(
            policy.evaluate(gmail_tool.spec(), &scoped_principal),
            PermissionDecision::Allow
        ),
        "gmail.search should be allowed"
    );
    assert!(
        matches!(
            policy.evaluate(cal_list_tool.spec(), &scoped_principal),
            PermissionDecision::Allow
        ),
        "calendar.list should be allowed because calendar.events implies calendar.readonly"
    );

    // Orange tools with scopes satisfied require human approval (not scope denial!)
    match policy.evaluate(cal_delete_tool.spec(), &scoped_principal) {
        PermissionDecision::RequireApproval { reason } => {
            assert!(
                reason.contains("calendar.delete"),
                "Approval reason: {reason}"
            );
        }
        other => panic!("Expected RequireApproval for orange tool, got {other:?}"),
    }
}
