//! Conversations: the agent as a chat rather than a form.
//!
//! A conversation holds many turns; a turn is an `agent_runs` row. Keeping the run as
//! the unit of agent work means the timeline, the approvals, the typed outcome and
//! the workspace all keep working unchanged — what changed is that the user now types
//! a message instead of filling in an instrument, a timeframe, an iteration cap and a
//! time budget.
//!
//! # The agent keeps running when the user leaves
//!
//! A turn is a detached `tokio::spawn` writing to Postgres. Nothing about it depends
//! on anyone watching: close the tab, go and trade, come back tomorrow, and the
//! transcript is where the agent left it. The UI is a reader of that table, not the
//! thing driving the work — which is also why "is this conversation running" is a
//! query rather than a socket.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use uuid::Uuid;

/// States a run can be in while it is still going.
///
/// One list, used by the sidebar's running dot, the resume check and the
/// one-turn-at-a-time guard. Three copies of this would disagree within a month.
pub const ACTIVE_RUN_STATES: &[&str] =
    &["queued", "running", "waiting_backtest", "awaiting_approval"];

/// `(id, title, provider, model, created_at, last_activity_at, active_runs, turns)`
///
/// Named because the anonymous tuple is eight fields deep and read in two places; a
/// column reordered in one query and not the other would compile and be wrong.
type SummaryRow = (
    Uuid,
    Option<String>,
    String,
    String,
    DateTime<Utc>,
    DateTime<Utc>,
    Option<i64>,
    Option<i64>,
);

/// `(run_id, turn_index, status, prompt, summary, error, tokens_in, tokens_out,
/// created_at, finished_at)`
type TurnRow = (
    Uuid,
    i32,
    String,
    String,
    Option<String>,
    Option<String>,
    i64,
    i64,
    DateTime<Utc>,
    Option<DateTime<Utc>>,
);

#[derive(Debug, Clone, Serialize)]
pub struct ConversationSummary {
    pub conversation_id: Uuid,
    /// `None` until the summariser has run. The UI falls back to the first prompt.
    pub title: Option<String>,
    pub model: String,
    pub provider: String,
    pub created_at: DateTime<Utc>,
    pub last_activity_at: DateTime<Utc>,
    /// Whether an agent is working in this conversation right now.
    pub running: bool,
    pub turns: i64,
}

#[derive(Debug, Clone, Serialize)]
pub struct Turn {
    pub run_id: Uuid,
    pub turn_index: i32,
    pub status: String,
    pub prompt: String,
    pub summary: Option<String>,
    pub error: Option<String>,
    pub tokens_in: i64,
    pub tokens_out: i64,
    pub created_at: DateTime<Utc>,
    pub finished_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Deserialize)]
pub struct NewConversation {
    pub provider: Option<String>,
    pub model: Option<String>,
}

#[derive(Debug, thiserror::Error)]
pub enum ConversationError {
    #[error("conversation not found")]
    NotFound,
    #[error("{0}")]
    Invalid(String),
    #[error(transparent)]
    Db(#[from] sqlx::Error),
}

/// Creates an empty conversation. The "New chat" button.
///
/// Empty on purpose: the chat page opens with the composer focused and nothing else,
/// so the first thing a user does is type. Creating it lazily on the first message
/// would mean the page has no id to stream against until they send.
pub async fn create(
    pg: &PgPool,
    user_id: Uuid,
    provider: &str,
    model: &str,
) -> Result<Uuid, ConversationError> {
    let id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO agent_conversations (conversation_id, user_id, provider, model)
         VALUES ($1, $2, $3, $4)",
    )
    .bind(id)
    .bind(user_id)
    .bind(provider)
    .bind(model)
    .execute(pg)
    .await?;
    Ok(id)
}

/// The sidebar.
pub async fn list(
    pg: &PgPool,
    user_id: Uuid,
    limit: i64,
) -> Result<Vec<ConversationSummary>, ConversationError> {
    // `running` and `turns` are computed in SQL rather than by loading every run:
    // the sidebar is polled, and N+1 queries behind a poll is a slow leak.
    let rows: Vec<SummaryRow> = sqlx::query_as(
        "SELECT c.conversation_id, c.title, c.provider, c.model, c.created_at,
                c.last_activity_at,
                COUNT(r.run_id) FILTER (WHERE r.status = ANY($3)) AS active,
                COUNT(r.run_id) AS turns
           FROM agent_conversations c
           LEFT JOIN agent_runs r ON r.conversation_id = c.conversation_id
          WHERE c.user_id = $1 AND c.archived_at IS NULL
          GROUP BY c.conversation_id
          ORDER BY c.last_activity_at DESC
          LIMIT $2",
    )
    .bind(user_id)
    .bind(limit)
    .bind(ACTIVE_RUN_STATES)
    .fetch_all(pg)
    .await?;

    Ok(rows
        .into_iter()
        .map(
            |(id, title, provider, model, created, last, active, turns)| ConversationSummary {
                conversation_id: id,
                title,
                provider,
                model,
                created_at: created,
                last_activity_at: last,
                running: active.unwrap_or(0) > 0,
                turns: turns.unwrap_or(0),
            },
        )
        .collect())
}

/// One conversation's header, checking ownership.
pub async fn get(
    pg: &PgPool,
    user_id: Uuid,
    conversation_id: Uuid,
) -> Result<ConversationSummary, ConversationError> {
    list_one(pg, user_id, conversation_id).await
}

async fn list_one(
    pg: &PgPool,
    user_id: Uuid,
    conversation_id: Uuid,
) -> Result<ConversationSummary, ConversationError> {
    let row: Option<SummaryRow> = sqlx::query_as(
        "SELECT c.conversation_id, c.title, c.provider, c.model, c.created_at,
                c.last_activity_at,
                COUNT(r.run_id) FILTER (WHERE r.status = ANY($3)) AS active,
                COUNT(r.run_id) AS turns
           FROM agent_conversations c
           LEFT JOIN agent_runs r ON r.conversation_id = c.conversation_id
          WHERE c.user_id = $1 AND c.conversation_id = $2
          GROUP BY c.conversation_id",
    )
    .bind(user_id)
    .bind(conversation_id)
    .bind(ACTIVE_RUN_STATES)
    .fetch_optional(pg)
    .await?;

    let (id, title, provider, model, created, last, active, turns) =
        row.ok_or(ConversationError::NotFound)?;
    Ok(ConversationSummary {
        conversation_id: id,
        title,
        provider,
        model,
        created_at: created,
        last_activity_at: last,
        running: active.unwrap_or(0) > 0,
        turns: turns.unwrap_or(0),
    })
}

/// Every turn, oldest first. The chat transcript's spine.
pub async fn turns(pg: &PgPool, conversation_id: Uuid) -> Result<Vec<Turn>, ConversationError> {
    let rows: Vec<TurnRow> = sqlx::query_as(
        "SELECT run_id, turn_index, status, goal, summary, error, tokens_in, tokens_out,
                created_at, finished_at
           FROM agent_runs
          WHERE conversation_id = $1
          ORDER BY turn_index ASC",
    )
    .bind(conversation_id)
    .fetch_all(pg)
    .await?;

    Ok(rows
        .into_iter()
        .map(
            |(run_id, turn_index, status, prompt, summary, error, ti, to, created, finished)| {
                Turn {
                    run_id,
                    turn_index,
                    status,
                    prompt,
                    summary,
                    error,
                    tokens_in: ti,
                    tokens_out: to,
                    created_at: created,
                    finished_at: finished,
                }
            },
        )
        .collect())
}

/// The next turn index, and whether one is already in flight.
///
/// One turn at a time per conversation. Two agents working the same thread would
/// interleave their tool calls and their notes in one workspace, and the transcript
/// would stop being a sequence anyone could read.
pub async fn next_turn(
    pg: &PgPool,
    conversation_id: Uuid,
) -> Result<(i32, bool), ConversationError> {
    let row: Option<(Option<i32>, Option<i64>)> = sqlx::query_as(
        "SELECT MAX(turn_index), COUNT(*) FILTER (WHERE status = ANY($2))
           FROM agent_runs WHERE conversation_id = $1",
    )
    .bind(conversation_id)
    .bind(ACTIVE_RUN_STATES)
    .fetch_optional(pg)
    .await?;
    let (max, active) = row.unwrap_or((None, Some(0)));
    Ok((max.unwrap_or(-1) + 1, active.unwrap_or(0) > 0))
}

pub async fn touch(pg: &PgPool, conversation_id: Uuid) {
    let _ = sqlx::query(
        "UPDATE agent_conversations SET last_activity_at = now() WHERE conversation_id = $1",
    )
    .bind(conversation_id)
    .execute(pg)
    .await;
}

/// Sets the title.
///
/// Unconditional, and called exactly twice for a conversation: once synchronously
/// with a fallback cut from the prompt, and once when the summariser answers.
///
/// The first call is the important one. Generating the title takes a model call, and
/// on a cold local model that is nearly three minutes — measured. A row that says
/// nothing for three minutes is a row the user cannot find, so the fallback goes in
/// first and the better title replaces it whenever it arrives.
///
/// Only turn 0 ever titles a conversation, so there is no later turn to race with.
pub async fn set_title(pg: &PgPool, conversation_id: Uuid, title: &str) {
    let trimmed = title.trim();
    if trimmed.is_empty() {
        return;
    }
    let _ = sqlx::query("UPDATE agent_conversations SET title = $2 WHERE conversation_id = $1")
        .bind(conversation_id)
        .bind(trimmed)
        .execute(pg)
        .await;
}

/// The title to show before the summariser has answered.
#[must_use]
pub fn provisional_title(prompt: &str) -> String {
    fallback(prompt)
}

pub async fn set_workspace(pg: &PgPool, conversation_id: Uuid, path: &str) {
    let _ = sqlx::query(
        "UPDATE agent_conversations SET workspace_path = $2 WHERE conversation_id = $1",
    )
    .bind(conversation_id)
    .bind(path)
    .execute(pg)
    .await;
}

pub async fn archive(
    pg: &PgPool,
    user_id: Uuid,
    conversation_id: Uuid,
) -> Result<bool, ConversationError> {
    let done = sqlx::query(
        "UPDATE agent_conversations SET archived_at = now()
          WHERE conversation_id = $1 AND user_id = $2 AND archived_at IS NULL",
    )
    .bind(conversation_id)
    .bind(user_id)
    .execute(pg)
    .await?;
    Ok(done.rows_affected() > 0)
}

/// What the agent should know about earlier turns.
///
/// Deliberately the *conclusions*, not the transcripts. Replaying every prior tool
/// call would refill the context window with work already done and summarised; the
/// detail is still in the agent's own workspace if it wants it back.
pub async fn prior_context(
    pg: &PgPool,
    conversation_id: Uuid,
    before_turn: i32,
) -> Result<String, ConversationError> {
    let rows: Vec<(i32, String, Option<String>, Option<String>)> = sqlx::query_as(
        "SELECT turn_index, goal, summary, error
           FROM agent_runs
          WHERE conversation_id = $1 AND turn_index < $2
          ORDER BY turn_index ASC",
    )
    .bind(conversation_id)
    .bind(before_turn)
    .fetch_all(pg)
    .await?;

    if rows.is_empty() {
        return Ok(String::new());
    }
    let mut out = String::from("Earlier in this conversation:\n");
    for (i, prompt, summary, error) in rows {
        out.push_str(&format!("\n[turn {i}] the user asked: {prompt}\n"));
        match (summary, error) {
            (Some(s), _) if !s.trim().is_empty() => {
                out.push_str(&format!("you concluded: {s}\n"));
            }
            (_, Some(e)) if !e.trim().is_empty() => {
                out.push_str(&format!("that turn did not finish: {e}\n"));
            }
            _ => out.push_str("that turn produced no conclusion.\n"),
        }
    }
    Ok(out)
}

/// Asks a model for a short title.
///
/// One cheap call with a low token cap. Falls back to a trimmed prompt rather than
/// failing: a conversation with no title is a row the user cannot find again, and
/// that is worse than a title that is merely the first few words.
pub async fn generate_title(client: &llm::LlmClient, model: &str, prompt: &str) -> String {
    let req = llm::ChatRequest {
        model: model.to_string(),
        system: Some(
            "You name conversations. Reply with a title of 3 to 6 words, in plain \
             words, describing what is being asked. No quotes, no punctuation at the \
             end, no preamble."
                .into(),
        ),
        messages: vec![llm::Message::User {
            content: format!("Title this request:\n\n{}", truncate(prompt, 600)),
        }],
        tools: Vec::new(),
        max_tokens: 24,
        temperature: Some(0.0),
        // Constrained so a chatty local model cannot answer "Sure! Here's a title:".
        schema: Some(serde_json::json!({
            "type": "object",
            "properties": {"title": {"type": "string"}},
            "required": ["title"]
        })),
        num_ctx: Some(2048),
        keep_alive: Some("30m".into()),
        tool_choice: None,
    };

    match client.chat(&req).await {
        Ok(resp) => {
            let raw = resp.content.unwrap_or_default();
            let parsed = serde_json::from_str::<serde_json::Value>(&raw)
                .ok()
                .and_then(|v| {
                    v.get("title")
                        .and_then(serde_json::Value::as_str)
                        .map(str::to_string)
                });
            clean(parsed.unwrap_or(raw), prompt)
        }
        Err(e) => {
            tracing::warn!(error = %e, "title generation failed; falling back to the prompt");
            fallback(prompt)
        }
    }
}

fn clean(candidate: String, prompt: &str) -> String {
    let t = candidate
        .trim()
        .trim_matches(['"', '\'', '.', ':'])
        .trim()
        .to_string();
    // A model that ignored the instruction and wrote a paragraph has not given us a
    // title, and a 200-character sidebar row is worse than the first few words.
    if t.is_empty() || t.len() > 80 || t.contains('\n') {
        return fallback(prompt);
    }
    t
}

fn fallback(prompt: &str) -> String {
    let words: Vec<&str> = prompt.split_whitespace().take(7).collect();
    let t = words.join(" ");
    truncate(&t, 60)
}

fn truncate(s: &str, max: usize) -> String {
    let s = s.trim();
    if s.len() <= max {
        return s.to_string();
    }
    let mut end = max;
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}…", &s[..end])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_usable_title_is_kept() {
        assert_eq!(
            clean("  \"Momentum on BTC\" ".into(), "x"),
            "Momentum on BTC"
        );
    }

    /// A model that ignored the instruction has not given us a title. A 200-character
    /// sidebar row is worse than the first few words of the prompt.
    #[test]
    fn a_model_that_wrote_a_paragraph_falls_back_to_the_prompt() {
        let prompt = "Find whether BTC-USD has momentum at the 1h timeframe over the last year";
        let rambling = "Certainly! Here is a title for your conversation:\nMomentum study";
        let t = clean(rambling.into(), prompt);
        assert!(t.starts_with("Find whether BTC-USD"), "{t}");
        assert!(t.len() <= 61, "{t}");
    }

    #[test]
    fn an_empty_answer_falls_back_rather_than_leaving_the_row_anonymous() {
        assert_eq!(
            clean("   ".into(), "Check ETH liquidity"),
            "Check ETH liquidity"
        );
    }

    #[test]
    fn the_fallback_is_short_enough_for_a_sidebar() {
        let long = "a ".repeat(200);
        assert!(fallback(&long).len() <= 61);
    }

    /// A conversation is never anonymous. Generating the real title needs a model
    /// call — nearly three minutes on a cold local model, measured — and a row that
    /// says nothing for three minutes is a row the user cannot find again.
    #[test]
    fn there_is_always_a_title_to_show_immediately() {
        for prompt in [
            "What instruments do we have market data for?",
            "hi",
            "   Find momentum on BTC-USD   ",
        ] {
            let t = provisional_title(prompt);
            assert!(!t.trim().is_empty(), "empty title for {prompt:?}");
            assert!(t.len() <= 61, "{t}");
        }
    }

    /// One list of active states, because the sidebar's dot, the resume check and the
    /// one-turn-at-a-time guard all read it.
    #[test]
    fn the_active_states_do_not_include_terminal_ones() {
        for dead in ["completed", "failed", "cancelled", "fenced", "refused"] {
            assert!(!ACTIVE_RUN_STATES.contains(&dead), "{dead}");
        }
        assert!(ACTIVE_RUN_STATES.contains(&"running"));
        assert!(ACTIVE_RUN_STATES.contains(&"awaiting_approval"));
    }
}
