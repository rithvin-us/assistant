//! Google Classroom, normalised.
//!
//! This is the only file in the server that knows what Classroom's JSON looks
//! like. Everything above it sees `Course`, `CourseworkItem` and `Announcement`
//! from `assistant-protocol`, so a second course provider would mean writing
//! another normaliser rather than changing the wire contract or the UI.
//!
//! Read-only on purpose. Milestone 6 is about understanding academic
//! information, not administering it: there is no submission, grading or
//! roster call here, and the scopes requested do not permit one.

use assistant_protocol::{Announcement, Course, CourseworkItem, MaterialRef};
use assistant_tools::{ClassroomProvider, ToolError};
use async_trait::async_trait;
use serde::Deserialize;
use time::{Date, Month, OffsetDateTime, Time};
use uuid::Uuid;

use super::{GoogleClient, api_error};

const API_BASE: &str = "https://classroom.googleapis.com/v1";

/// Classroom's `Date`: a calendar date with every field optional.
#[derive(Debug, Deserialize)]
struct GoogleDate {
    year: Option<i32>,
    month: Option<u8>,
    day: Option<u8>,
}

/// Classroom's `TimeOfDay`, documented as UTC. Fields are optional and default
/// to zero, which is how "23:59" arrives with no seconds.
#[derive(Debug, Deserialize, Default)]
struct GoogleTimeOfDay {
    hours: Option<u8>,
    minutes: Option<u8>,
    seconds: Option<u8>,
}

/// Combines Classroom's split due date into one instant.
///
/// Returns `None` unless a complete date is present. Classroom documents
/// `dueDate` and `dueTime` as optional but required together; a partial value
/// means the assignment has no usable deadline, and guessing one would put a
/// fabricated deadline in front of the user and into the scheduler. The time is
/// UTC per Google's documentation, so no local-zone conversion happens here.
fn due_at(
    date: Option<&GoogleDate>,
    time_of_day: Option<&GoogleTimeOfDay>,
) -> Option<OffsetDateTime> {
    let date = date?;
    let (y, m, d) = (date.year?, date.month?, date.day?);
    let month = Month::try_from(m).ok()?;
    let day = Date::from_calendar_date(y, month, d).ok()?;

    // A missing dueTime is not a missing deadline: Classroom treats a
    // date-only assignment as due at the end of that day, and midnight UTC
    // would move it a day earlier for most of the world. Default to 23:59:59.
    let default = GoogleTimeOfDay::default();
    let tod = time_of_day.unwrap_or(&default);
    let has_time = tod.hours.is_some() || tod.minutes.is_some() || tod.seconds.is_some();
    let time = if has_time {
        Time::from_hms(
            tod.hours.unwrap_or(0),
            tod.minutes.unwrap_or(0),
            tod.seconds.unwrap_or(0),
        )
        .ok()?
    } else {
        Time::from_hms(23, 59, 59).ok()?
    };

    Some(day.with_time(time).assume_utc())
}

/// Parses an RFC 3339 timestamp, discarding anything unparseable rather than
/// failing the whole listing for one malformed field.
fn parse_rfc3339(value: Option<&str>) -> Option<OffsetDateTime> {
    OffsetDateTime::parse(value?, &time::format_description::well_known::Rfc3339).ok()
}

#[derive(Debug, Deserialize)]
struct RawMaterial {
    #[serde(rename = "driveFile")]
    drive_file: Option<serde_json::Value>,
    link: Option<serde_json::Value>,
    #[serde(rename = "youtubeVideo")]
    youtube_video: Option<serde_json::Value>,
    form: Option<serde_json::Value>,
}

/// Reduces a material to a title and a link.
///
/// Materials are kept as metadata only: a title and a URL are enough to show
/// what is attached and to open it in Google's own UI. File bodies are never
/// pulled in here.
fn material_ref(raw: &RawMaterial) -> MaterialRef {
    let pick = |v: &serde_json::Value, title_key: &str, link_key: &str| MaterialRef {
        title: v
            .get(title_key)
            .and_then(|t| t.as_str())
            .map(str::to_string),
        link: v.get(link_key).and_then(|t| t.as_str()).map(str::to_string),
        kind: None,
    };

    if let Some(v) = raw.drive_file.as_ref() {
        // driveFile nests the file under another `driveFile` key.
        let inner = v.get("driveFile").unwrap_or(v);
        let mut m = pick(inner, "title", "alternateLink");
        m.kind = Some("drive_file".into());
        return m;
    }
    if let Some(v) = raw.link.as_ref() {
        let mut m = pick(v, "title", "url");
        m.kind = Some("link".into());
        return m;
    }
    if let Some(v) = raw.youtube_video.as_ref() {
        let mut m = pick(v, "title", "alternateLink");
        m.kind = Some("youtube_video".into());
        return m;
    }
    if let Some(v) = raw.form.as_ref() {
        let mut m = pick(v, "title", "formUrl");
        m.kind = Some("form".into());
        return m;
    }
    MaterialRef::default()
}

#[derive(Debug, Deserialize)]
struct RawCourse {
    id: String,
    name: Option<String>,
    section: Option<String>,
    description: Option<String>,
    room: Option<String>,
    #[serde(rename = "courseState")]
    course_state: Option<String>,
    #[serde(rename = "alternateLink")]
    alternate_link: Option<String>,
    #[serde(rename = "updateTime")]
    update_time: Option<String>,
}

#[derive(Debug, Deserialize)]
struct CoursesResponse {
    courses: Option<Vec<RawCourse>>,
    #[serde(rename = "nextPageToken")]
    next_page_token: Option<String>,
}

#[derive(Debug, Deserialize)]
struct RawCoursework {
    id: String,
    #[serde(rename = "courseId")]
    course_id: Option<String>,
    title: Option<String>,
    description: Option<String>,
    state: Option<String>,
    #[serde(rename = "alternateLink")]
    alternate_link: Option<String>,
    #[serde(rename = "dueDate")]
    due_date: Option<GoogleDate>,
    #[serde(rename = "dueTime")]
    due_time: Option<GoogleTimeOfDay>,
    #[serde(rename = "maxPoints")]
    max_points: Option<f64>,
    #[serde(rename = "workType")]
    work_type: Option<String>,
    #[serde(rename = "updateTime")]
    update_time: Option<String>,
    materials: Option<Vec<RawMaterial>>,
}

#[derive(Debug, Deserialize)]
struct CourseworkResponse {
    #[serde(rename = "courseWork")]
    course_work: Option<Vec<RawCoursework>>,
    #[serde(rename = "nextPageToken")]
    next_page_token: Option<String>,
}

#[derive(Debug, Deserialize)]
struct RawAnnouncement {
    id: String,
    #[serde(rename = "courseId")]
    course_id: Option<String>,
    text: Option<String>,
    #[serde(rename = "alternateLink")]
    alternate_link: Option<String>,
    #[serde(rename = "creationTime")]
    creation_time: Option<String>,
    #[serde(rename = "updateTime")]
    update_time: Option<String>,
    materials: Option<Vec<RawMaterial>>,
}

#[derive(Debug, Deserialize)]
struct AnnouncementsResponse {
    announcements: Option<Vec<RawAnnouncement>>,
}

/// Normalises one course. Public for the unit tests, which feed it recorded
/// Google payloads rather than calling the live API.
fn normalize_course(raw: RawCourse, account_id: Uuid, synced_at: OffsetDateTime) -> Course {
    Course {
        external_id: raw.id,
        account_id,
        // A course with no name is not worth hiding behind an empty string in
        // a list; say what it is.
        name: raw.name.unwrap_or_else(|| "Untitled course".into()),
        section: raw.section,
        description: raw.description,
        room: raw.room,
        // Classroom returns `ownerId`, a numeric user id, not a teacher name.
        // Resolving it needs a separate roster call and a roster scope this
        // milestone deliberately does not request, so the field stays empty
        // rather than being filled with an opaque id.
        teacher_name: None,
        state: raw.course_state.unwrap_or_else(|| "ACTIVE".into()),
        alternate_link: raw.alternate_link,
        source_updated_at: parse_rfc3339(raw.update_time.as_deref()),
        synced_at,
    }
}

fn normalize_coursework(
    raw: RawCoursework,
    course_external_id: &str,
    account_id: Uuid,
    synced_at: OffsetDateTime,
) -> CourseworkItem {
    CourseworkItem {
        external_id: raw.id,
        course_external_id: raw
            .course_id
            .unwrap_or_else(|| course_external_id.to_string()),
        account_id,
        title: raw.title.unwrap_or_else(|| "Untitled assignment".into()),
        description: raw.description,
        state: raw.state.unwrap_or_else(|| "PUBLISHED".into()),
        alternate_link: raw.alternate_link,
        due_at: due_at(raw.due_date.as_ref(), raw.due_time.as_ref()),
        max_points: raw.max_points,
        work_type: raw.work_type,
        materials: raw
            .materials
            .unwrap_or_default()
            .iter()
            .map(material_ref)
            .collect(),
        source_updated_at: parse_rfc3339(raw.update_time.as_deref()),
        synced_at,
    }
}

fn normalize_announcement(
    raw: RawAnnouncement,
    course_external_id: &str,
    account_id: Uuid,
    synced_at: OffsetDateTime,
) -> Announcement {
    Announcement {
        external_id: raw.id,
        course_external_id: raw
            .course_id
            .unwrap_or_else(|| course_external_id.to_string()),
        account_id,
        text: raw.text.unwrap_or_default(),
        // Same reason as `teacher_name` above: Classroom gives `creatorUserId`,
        // not a name.
        author_name: None,
        alternate_link: raw.alternate_link,
        materials: raw
            .materials
            .unwrap_or_default()
            .iter()
            .map(material_ref)
            .collect(),
        source_created_at: parse_rfc3339(raw.creation_time.as_deref()),
        source_updated_at: parse_rfc3339(raw.update_time.as_deref()),
        synced_at,
    }
}

#[async_trait]
impl ClassroomProvider for GoogleClient {
    async fn courses(&self, account_id: Uuid, user_id: Uuid) -> Result<Vec<Course>, ToolError> {
        let token = self.get_access_token(user_id, account_id).await?;
        let synced_at = OffsetDateTime::now_utc();
        let mut out = Vec::new();
        let mut page_token: Option<String> = None;

        // Bounded rather than "follow every page": a student's course list is
        // small, and an unbounded loop against someone else's API is how a
        // quota gets exhausted by a bug.
        for _ in 0..5 {
            let mut url =
                format!("{API_BASE}/courses?studentId=me&courseStates=ACTIVE&pageSize=100");
            if let Some(ref t) = page_token {
                url.push_str(&format!("&pageToken={t}"));
            }

            let resp = self
                .http
                .get(&url)
                .bearer_auth(&token)
                .send()
                .await
                .map_err(|_| ToolError::Failed("Could not reach Google Classroom.".into()))?;

            if !resp.status().is_success() {
                return Err(api_error("Classroom", resp.status()));
            }

            let body: CoursesResponse = resp.json().await.map_err(|_| {
                ToolError::Failed("Classroom returned an unreadable response.".into())
            })?;

            for raw in body.courses.unwrap_or_default() {
                out.push(normalize_course(raw, account_id, synced_at));
            }

            match body.next_page_token {
                Some(t) if !t.is_empty() => page_token = Some(t),
                _ => break,
            }
        }

        Ok(out)
    }

    async fn coursework(
        &self,
        account_id: Uuid,
        user_id: Uuid,
        course_external_id: &str,
    ) -> Result<Vec<CourseworkItem>, ToolError> {
        let token = self.get_access_token(user_id, account_id).await?;
        let synced_at = OffsetDateTime::now_utc();
        let mut out = Vec::new();
        let mut page_token: Option<String> = None;

        for _ in 0..5 {
            let mut url = format!(
                "{API_BASE}/courses/{}/courseWork?courseWorkStates=PUBLISHED&pageSize=100",
                super::url_encode(course_external_id)
            );
            if let Some(ref t) = page_token {
                url.push_str(&format!("&pageToken={t}"));
            }

            let resp = self
                .http
                .get(&url)
                .bearer_auth(&token)
                .send()
                .await
                .map_err(|_| ToolError::Failed("Could not reach Google Classroom.".into()))?;

            if !resp.status().is_success() {
                return Err(api_error("Classroom", resp.status()));
            }

            let body: CourseworkResponse = resp.json().await.map_err(|_| {
                ToolError::Failed("Classroom returned an unreadable response.".into())
            })?;

            for raw in body.course_work.unwrap_or_default() {
                out.push(normalize_coursework(
                    raw,
                    course_external_id,
                    account_id,
                    synced_at,
                ));
            }

            match body.next_page_token {
                Some(t) if !t.is_empty() => page_token = Some(t),
                _ => break,
            }
        }

        Ok(out)
    }

    async fn announcements(
        &self,
        account_id: Uuid,
        user_id: Uuid,
        course_external_id: &str,
        limit: u32,
    ) -> Result<Vec<Announcement>, ToolError> {
        let token = self.get_access_token(user_id, account_id).await?;
        let synced_at = OffsetDateTime::now_utc();

        // One page only. Announcements are shown as "recent"; there is no
        // reason to walk a course's entire history to render a short list.
        let url = format!(
            "{API_BASE}/courses/{}/announcements?announcementStates=PUBLISHED&pageSize={}",
            super::url_encode(course_external_id),
            limit.clamp(1, 50)
        );

        let resp = self
            .http
            .get(&url)
            .bearer_auth(&token)
            .send()
            .await
            .map_err(|_| ToolError::Failed("Could not reach Google Classroom.".into()))?;

        if !resp.status().is_success() {
            return Err(api_error("Classroom", resp.status()));
        }

        let body: AnnouncementsResponse = resp
            .json()
            .await
            .map_err(|_| ToolError::Failed("Classroom returned an unreadable response.".into()))?;

        Ok(body
            .announcements
            .unwrap_or_default()
            .into_iter()
            .map(|raw| normalize_announcement(raw, course_external_id, account_id, synced_at))
            .collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ts() -> OffsetDateTime {
        OffsetDateTime::from_unix_timestamp(1_700_000_000).unwrap()
    }

    fn parse_coursework(json: &str) -> CourseworkItem {
        let raw: RawCoursework = serde_json::from_str(json).expect("payload parses");
        normalize_coursework(raw, "c1", Uuid::nil(), ts())
    }

    #[test]
    fn due_date_and_time_become_one_utc_instant() {
        let item = parse_coursework(
            r#"{"id":"w1","title":"Assignment 2",
                "dueDate":{"year":2026,"month":9,"day":11},
                "dueTime":{"hours":18,"minutes":29}}"#,
        );
        let due = item.due_at.expect("a complete due date yields a deadline");
        assert_eq!(due.year(), 2026);
        assert_eq!(due.month() as u8, 9);
        assert_eq!(due.day(), 11);
        assert_eq!(due.hour(), 18);
        assert_eq!(due.minute(), 29);
        assert_eq!(due.offset(), time::UtcOffset::UTC);
    }

    #[test]
    fn missing_due_date_is_no_deadline_not_a_guess() {
        let item = parse_coursework(r#"{"id":"w2","title":"Reading"}"#);
        assert!(item.due_at.is_none());
    }

    #[test]
    fn partial_due_date_is_refused_rather_than_completed() {
        // A year and month with no day cannot name an instant. Filling in the
        // 1st would invent a deadline the user never set.
        let item =
            parse_coursework(r#"{"id":"w3","title":"Vague","dueDate":{"year":2026,"month":9}}"#);
        assert!(item.due_at.is_none());
    }

    #[test]
    fn date_without_time_lands_at_end_of_day() {
        let item = parse_coursework(
            r#"{"id":"w4","title":"All day","dueDate":{"year":2026,"month":9,"day":11}}"#,
        );
        let due = item.due_at.expect("date-only still has a deadline");
        assert_eq!(due.hour(), 23);
        assert_eq!(due.minute(), 59);
        assert_eq!(due.day(), 11, "must not roll back to the previous day");
    }

    #[test]
    fn coursework_defaults_do_not_invent_content() {
        let item = parse_coursework(r#"{"id":"w5"}"#);
        assert_eq!(item.title, "Untitled assignment");
        assert_eq!(item.state, "PUBLISHED");
        assert!(item.description.is_none());
        assert!(item.materials.is_empty());
        assert!(item.max_points.is_none());
    }

    #[test]
    fn course_normalizes_and_keeps_provider_state() {
        let raw: RawCourse = serde_json::from_str(
            r#"{"id":"c9","name":"Compiler Design","section":"B",
                "room":"L302","courseState":"ARCHIVED",
                "alternateLink":"https://classroom.google.com/c/c9",
                "updateTime":"2026-09-01T10:00:00Z"}"#,
        )
        .unwrap();
        let course = normalize_course(raw, Uuid::nil(), ts());
        assert_eq!(course.name, "Compiler Design");
        assert_eq!(course.section.as_deref(), Some("B"));
        assert_eq!(course.state, "ARCHIVED");
        assert!(course.source_updated_at.is_some());
        // Not inferred from an opaque ownerId.
        assert!(course.teacher_name.is_none());
    }

    #[test]
    fn materials_are_metadata_only() {
        let item = parse_coursework(
            r#"{"id":"w6","title":"With material","materials":[
                {"driveFile":{"driveFile":{"title":"Notes.pdf",
                 "alternateLink":"https://drive.google.com/file/d/x"}}},
                {"link":{"url":"https://example.com","title":"Reference"}}]}"#,
        );
        assert_eq!(item.materials.len(), 2);
        assert_eq!(item.materials[0].title.as_deref(), Some("Notes.pdf"));
        assert_eq!(item.materials[0].kind.as_deref(), Some("drive_file"));
        assert_eq!(item.materials[1].kind.as_deref(), Some("link"));
    }

    #[test]
    fn announcement_normalizes_text_and_times() {
        let raw: RawAnnouncement = serde_json::from_str(
            r#"{"id":"a1","courseId":"c1","text":"Lab moved to Friday",
                "creationTime":"2026-09-02T08:30:00Z",
                "updateTime":"2026-09-02T09:00:00Z"}"#,
        )
        .unwrap();
        let a = normalize_announcement(raw, "c1", Uuid::nil(), ts());
        assert_eq!(a.text, "Lab moved to Friday");
        assert_eq!(a.course_external_id, "c1");
        assert!(a.source_created_at.is_some());
    }

    #[test]
    fn unparseable_timestamp_is_dropped_not_fatal() {
        let raw: RawCourse =
            serde_json::from_str(r#"{"id":"c2","name":"X","updateTime":"not-a-date"}"#).unwrap();
        let course = normalize_course(raw, Uuid::nil(), ts());
        assert!(course.source_updated_at.is_none());
        assert_eq!(course.name, "X");
    }
}
