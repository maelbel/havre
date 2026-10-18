//! The assistant (the AI tab and the Ctrl+K / ⌘K palette).
//!
//! You ask in plain language; the model answers using **tools**: small
//! functions it may call (search mail, read an email, list events, find a
//! free slot, look up people, find and read notes, list tasks). Read-only
//! tools run right away. Tools that would change something only make a
//! **proposal** (an event to create, an email to write, a task to add, a
//! note to write): the UI shows it with a button, and nothing happens until
//! you click.
//!
//! The loop: send the conversation + tool list → the model either answers
//! or asks for tool calls → we run them and send the results back → … (at
//! most [`MAX_ROUNDS`] times).

use std::collections::HashMap;

use chrono::{Duration, NaiveDate, NaiveDateTime, NaiveTime};
use serde::{Deserialize, Serialize};
use serde_json::json;

use super::{
    Ai, LocalTime, UNTRUSTED, claude_code, full_text,
    index::Hit,
    provider::{Client, Message, Request, Tool, ToolCall},
    weekday_name,
};
use crate::{
    Error, Result,
    agenda::Event,
    notes::{NoteQuery, NoteSummary, markdown::Block},
    tasks::{EventLink, PersonLink, Task, TaskQuery},
};

const MAX_ROUNDS: usize = 6;

/// One message of the conversation, as the UI keeps it.
#[derive(Debug, Clone, Deserialize)]
pub struct Turn {
    /// `user` or `assistant`.
    pub role: String,
    pub text: String,
}

/// Something the assistant suggests doing; the UI asks you first.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Proposal {
    Event {
        title: String,
        /// Unix seconds.
        start: i64,
        end: i64,
        location: String,
        guests: Vec<String>,
        notes: String,
    },
    Email {
        to: String,
        subject: String,
        body: String,
    },
    /// Move an existing event (found with list_events).
    Move {
        event: i64,
        title: String,
        /// Where it is now (unix seconds)…
        from_start: i64,
        from_end: i64,
        /// …and where it would go.
        start: i64,
        end: i64,
    },
    /// Add a task (to the default list, shown as `list`).
    Task {
        title: String,
        /// The day (midnight UTC), and the moment when it has a time.
        due: Option<i64>,
        due_at: Option<i64>,
        notes: String,
        list: String,
        person: Option<PersonLink>,
        event: Option<EventLink>,
        /// Made from a note's `- [ ]` line: confirming makes the task the
        /// way the note's own *Make a task* does (linked, ticked together).
        from_note: Option<NoteTodo>,
    },
    /// Write a new note.
    Note {
        title: String,
        /// Markdown.
        body: String,
        people: Vec<PersonLink>,
        event: Option<EventLink>,
    },
}

/// A note's to-do line a task would be made from.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct NoteTodo {
    pub note: i64,
    pub line: usize,
    /// The note's title, for the UI.
    pub title: String,
}

/// What the assistant did, for the UI ("Searched mail for “budget”").
#[derive(Debug, Clone, Serialize)]
pub struct Step {
    pub tool: String,
    pub label: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct Answer {
    /// May cite `[m3]` (a message), `[e2]` (an event), `[n1]` (a note) or
    /// `[t4]` (a task): see `messages`, `events`, `notes`, `tasks`.
    pub text: String,
    pub steps: Vec<Step>,
    pub proposals: Vec<Proposal>,
    pub messages: HashMap<String, Hit>,
    pub events: HashMap<String, Event>,
    pub notes: HashMap<String, NoteSummary>,
    pub tasks: HashMap<String, Task>,
}

/// Who is using the tools.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToolSet {
    /// Havre's own assistant: changes are *proposed* to the user.
    Assistant,
    /// An outside MCP client (Claude Code, Claude Desktop…): read tools,
    /// and drafts saved in Havre (never sent).
    External,
}

/// The tools for `set`.
pub fn tools(set: ToolSet) -> Vec<Tool> {
    let all = all_tools();
    let wanted: &[&str] = match set {
        ToolSet::Assistant => &[
            "search_mail",
            "read_email",
            "unread_mail",
            "list_events",
            "find_free_time",
            "search_people",
            "search_notes",
            "read_note",
            "list_tasks",
            "propose_event",
            "propose_move",
            "propose_email",
            "propose_task",
            "propose_note",
        ],
        ToolSet::External => &[
            "search_mail",
            "read_email",
            "unread_mail",
            "list_events",
            "find_free_time",
            "search_people",
            "search_notes",
            "read_note",
            "list_tasks",
            "today",
            "create_draft",
        ],
    };
    all.into_iter().filter(|t| wanted.contains(&t.name)).collect()
}

/// What Settings › AI lets the assistant read. Notes and Tasks also need
/// their app in the dock: without it, the model isn't even told they exist.
pub fn allowed(name: &str, s: &super::settings::AiSettings) -> bool {
    match name {
        "search_mail" | "read_email" | "unread_mail" | "create_draft" | "propose_email" => s.read_mail,
        "list_events" | "find_free_time" | "propose_event" | "propose_move" => s.read_agenda,
        "search_people" => s.read_people,
        "search_notes" | "read_note" | "propose_note" => s.notes_app && s.read_notes,
        "list_tasks" | "propose_task" => s.tasks_app && s.read_tasks,
        "today" => s.read_mail || s.read_agenda,
        _ => true,
    }
}

/// Tools that change nothing (MCP clients may run them without asking).
pub fn is_read_only(name: &str) -> bool {
    !matches!(
        name,
        "propose_event" | "propose_move" | "propose_email" | "propose_task" | "propose_note" | "create_draft"
    )
}

fn all_tools() -> Vec<Tool> {
    let date = json!({ "type": "string", "description": "YYYY-MM-DD" });
    vec![
        Tool {
            name: "search_mail",
            description: "Search the user's emails (all accounts). Returns ids like m3 with sender, subject, date.",
            schema: json!({ "type": "object", "properties": {
                "query": { "type": "string", "description": "What to look for: words, a topic, a person" },
                "unread_only": { "type": "boolean" }
            }, "required": ["query"] }),
        },
        Tool {
            name: "read_email",
            description: "Read one email found with search_mail.",
            schema: json!({ "type": "object", "properties": {
                "id": { "type": "string", "description": "e.g. m3" }
            }, "required": ["id"] }),
        },
        Tool {
            name: "list_events",
            description: "Calendar events between two dates (inclusive). Returns ids like e2.",
            schema: json!({ "type": "object", "properties": { "from": date, "to": date }, "required": ["from", "to"] }),
        },
        Tool {
            name: "find_free_time",
            description: "Free slots on a day, between earliest and latest (local time).",
            schema: json!({ "type": "object", "properties": {
                "date": date,
                "duration_minutes": { "type": "integer" },
                "earliest": { "type": "string", "description": "HH:MM, default 09:00" },
                "latest": { "type": "string", "description": "HH:MM, default 18:00" }
            }, "required": ["date", "duration_minutes"] }),
        },
        Tool {
            name: "search_people",
            description: "People the user exchanged emails with or met: names and email addresses.",
            schema: json!({ "type": "object", "properties": { "query": { "type": "string" } }, "required": ["query"] }),
        },
        Tool {
            name: "unread_mail",
            description: "Unread emails in every inbox, newest first. Returns ids like m3.",
            schema: json!({ "type": "object", "properties": {} }),
        },
        Tool {
            name: "today",
            description: "The user's day: today's events, invitations waiting for an answer, unread emails.",
            schema: json!({ "type": "object", "properties": {} }),
        },
        Tool {
            name: "create_draft",
            description: "Save an email as a draft in Havre (Drafts). It is NOT sent: the user reviews and sends it.",
            schema: json!({ "type": "object", "properties": {
                "to": { "type": "string", "description": "Name <address>, comma-separated" },
                "subject": { "type": "string" },
                "body": { "type": "string", "description": "Plain text" },
                "from": { "type": "string", "description": "Optional: which of the user's addresses sends it" }
            }, "required": ["to", "subject", "body"] }),
        },
        Tool {
            name: "propose_event",
            description: "Propose a calendar event. The user sees it and confirms; it is NOT created by this call.",
            schema: json!({ "type": "object", "properties": {
                "title": { "type": "string" },
                "start": { "type": "string", "description": "YYYY-MM-DDTHH:MM local time" },
                "end": { "type": "string", "description": "YYYY-MM-DDTHH:MM local time" },
                "location": { "type": "string" },
                "guests": { "type": "array", "items": { "type": "string" }, "description": "email addresses" },
                "notes": { "type": "string" }
            }, "required": ["title", "start", "end"] }),
        },
        Tool {
            name: "propose_move",
            description: "Propose moving an existing event (an id like e2 from list_events) to a new time. The \
                          user confirms it; Microsoft and Google tell the guests. It is NOT moved by this call.",
            schema: json!({ "type": "object", "properties": {
                "id": { "type": "string", "description": "e.g. e2" },
                "start": { "type": "string", "description": "YYYY-MM-DDTHH:MM local time" },
                "end": { "type": "string", "description": "YYYY-MM-DDTHH:MM local time" }
            }, "required": ["id", "start", "end"] }),
        },
        Tool {
            name: "search_notes",
            description: "Search the user's own notes (Markdown, written in Havre): by words, or about a \
                          person. Returns ids like n1 with the title, the meeting and people it's about, \
                          open to-dos and the start of the text.",
            schema: json!({ "type": "object", "properties": {
                "query": { "type": "string", "description": "Words in the title or text, e.g. a meeting's name" },
                "person": { "type": "string", "description": "Optional: an email address the note is about" }
            } }),
        },
        Tool {
            name: "read_note",
            description: "Read one note found with search_notes: all its text, and its open to-dos with \
                          their line numbers (for propose_task).",
            schema: json!({ "type": "object", "properties": {
                "id": { "type": "string", "description": "e.g. n1" }
            }, "required": ["id"] }),
        },
        Tool {
            name: "list_tasks",
            description: "The user's tasks (to-dos): still to do by default. Returns ids like t1 with the due \
                          day, the list, and who or what each is about.",
            schema: json!({ "type": "object", "properties": {
                "query": { "type": "string", "description": "Optional: words in the title or notes" },
                "due_by": { "type": "string", "description": "Optional: only those due on or before YYYY-MM-DD" },
                "person": { "type": "string", "description": "Optional: an email address it's linked to" },
                "done": { "type": "boolean", "description": "true: those done in the last 30 days instead" }
            } }),
        },
        Tool {
            name: "propose_task",
            description: "Propose a task. The user sees it and confirms; it is NOT added by this call. To make \
                          a task of a note's open to-do, give note (n1) and line from read_note instead of \
                          a title.",
            schema: json!({ "type": "object", "properties": {
                "title": { "type": "string" },
                "due": date,
                "time": { "type": "string", "description": "Optional: HH:MM local time on the due day" },
                "notes": { "type": "string" },
                "person": { "type": "string", "description": "Optional: an email address it's about" },
                "event": { "type": "string", "description": "Optional: an event id like e2 it's for" },
                "note": { "type": "string", "description": "Optional: a note id like n1…" },
                "line": { "type": "integer", "description": "…and its to-do's line number" }
            } }),
        },
        Tool {
            name: "propose_note",
            description: "Propose a new note (Markdown: ## headings, - lists, - [ ] to-dos). The user confirms; \
                          it is NOT written by this call.",
            schema: json!({ "type": "object", "properties": {
                "title": { "type": "string" },
                "body": { "type": "string", "description": "Markdown" },
                "people": { "type": "array", "items": { "type": "string" }, "description": "email addresses it's about" },
                "event": { "type": "string", "description": "Optional: an event id like e2 it's for" }
            }, "required": ["title", "body"] }),
        },
        Tool {
            name: "propose_email",
            description: "Propose an email. The user sees it in the compose window and sends it themselves.",
            schema: json!({ "type": "object", "properties": {
                "to": { "type": "string", "description": "Name <address>, comma-separated" },
                "subject": { "type": "string" },
                "body": { "type": "string" }
            }, "required": ["to", "subject", "body"] }),
        },
    ]
}

/// What a conversation with the tools remembers: the ids it handed out
/// (`m3` → that message), the steps, the proposals.
pub struct ToolState {
    pub local: LocalTime,
    /// Who's asking, for the activity log (`assistant`, `mcp:<app>`).
    pub source: String,
    pub answer: Answer,
    /// m3 → where the message is.
    message_ids: HashMap<String, (i64, String, u32)>,
}

impl ToolState {
    pub fn new(local: LocalTime, source: &str) -> Self {
        Self {
            local,
            source: source.to_owned(),
            answer: Answer {
                text: String::new(),
                steps: Vec::new(),
                proposals: Vec::new(),
                messages: HashMap::new(),
                events: HashMap::new(),
                notes: HashMap::new(),
                tasks: HashMap::new(),
            },
            message_ids: HashMap::new(),
        }
    }
}

/// Runs tools for one conversation.
struct Session<'a> {
    ai: &'a Ai,
    st: &'a mut ToolState,
}

impl Ai {
    /// Answers the last user turn of `turns`.
    pub async fn ask(&self, turns: &[Turn], local: LocalTime) -> Result<Answer> {
        let client = self.client().await?;
        let accounts = self.db().accounts().await?;
        let me = accounts
            .iter()
            .map(|a| {
                if a.sender_name.is_empty() {
                    a.email.clone()
                } else {
                    format!("{} <{}>", a.sender_name, a.email)
                }
            })
            .collect::<Vec<_>>()
            .join(", ");
        let now = local.now();
        let settings = self.settings().await?;
        // Notes and tasks are only mentioned when the assistant may use them
        // (their app is in the dock, and Settings › AI lets it read them).
        let notes = allowed("search_notes", &settings);
        let tasks = allowed("list_tasks", &settings);
        let mut what = vec!["mail", "calendar"];
        let mut propose = vec!["propose_event / propose_move / propose_email"];
        let mut cite = vec!["[m3]", "[e2]"];
        if notes {
            what.push("notes");
            propose.push("propose_note");
            cite.push("[n1]");
        }
        if tasks {
            what.push("tasks");
            propose.push("propose_task");
            cite.push("[t4]");
        }
        let system = format!(
            "You are the assistant inside Havre, the user's {} app. {UNTRUSTED}\n\
             Now: {} {} (UTC{}). The user: {me}.\n\
             Use the tools to look things up instead of guessing. Dates and times are the user's local time.\n\
             To create, move or write something, use {}: the user confirms it themselves, so never say it is \
             done; say you prepared it.\n\
             Answer briefly, in the language of the user's last message. When you refer to something you \
             found, cite its id in brackets, like {}.",
            what.join(", "),
            weekday_name(now.date_naive()),
            now.format("%Y-%m-%d %H:%M"),
            now.format("%:z"),
            propose.join(", "),
            cite.join(" or "),
        );
        let mut messages: Vec<Message> = turns
            .iter()
            .filter(|t| !t.text.trim().is_empty())
            .map(|t| match t.role.as_str() {
                "assistant" => Message::Assistant { text: t.text.clone(), calls: Vec::new() },
                _ => Message::User(t.text.clone()),
            })
            .collect();
        if !matches!(messages.last(), Some(Message::User(_))) {
            return Err(Error::Config("Ask something first.".into()));
        }
        // Claude Code runs its own tool loop, with Havre's tools over MCP.
        if client.is_claude_code() {
            return self.ask_claude_code(&client, &system, turns, local).await;
        }
        let mut state = ToolState::new(local, "assistant");
        let mut session = Session { ai: self, st: &mut state };
        for _ in 0..MAX_ROUNDS {
            let request = Request {
                system: system.clone(),
                messages: messages.clone(),
                tools: tools(ToolSet::Assistant).into_iter().filter(|t| allowed(t.name, &settings)).collect(),
                max_tokens: 1500,
            };
            let reply = client.complete(&request).await?;
            if reply.calls.is_empty() {
                session.st.answer.text = reply.text;
                break;
            }
            let mut results = Vec::new();
            for call in &reply.calls {
                let result = match session.run(call).await {
                    Ok(text) => text,
                    // The model can read the error and try differently.
                    Err(err) => format!("Error: {err}"),
                };
                results.push((call.id.clone(), result));
            }
            messages.push(Message::Assistant { text: reply.text, calls: reply.calls });
            messages.push(Message::ToolResults(results));
        }
        if session.st.answer.text.is_empty() {
            session.st.answer.text =
                "I couldn't finish: the question needed too many steps. Try asking something more specific."
                    .into();
        }
        Ok(state.answer)
    }

    /// The assistant with Claude Code: it gets Havre's tools over MCP (a
    /// one-time token lets it call back into this server), runs its own
    /// tool loop, and we collect what the tools did.
    async fn ask_claude_code(
        &self,
        client: &Client,
        system: &str,
        turns: &[Turn],
        local: LocalTime,
    ) -> Result<Answer> {
        let claude = client.claude_code().expect("checked by the caller");
        let url = self
            .self_url()
            .ok_or_else(|| Error::Ai("Havre doesn't know its own address for Claude Code.".into()))?;
        let state = std::sync::Arc::new(tokio::sync::Mutex::new(ToolState::new(local, "assistant")));
        let secret = self.mcp.start_run(std::sync::Arc::clone(&state));
        let config = json!({ "mcpServers": { "havre": {
            "type": "http",
            "url": format!("{url}/mcp"),
            "headers": { "Authorization": format!("Bearer {secret}") },
        }}})
        .to_string();
        // Earlier turns as text, then the question.
        let mut prompt = String::new();
        let (last, earlier) =
            turns.split_last().ok_or_else(|| Error::Config("Ask something first.".into()))?;
        if !earlier.is_empty() {
            prompt.push_str("Earlier in this conversation:\n");
            for t in earlier {
                let who = if t.role == "assistant" { "Assistant" } else { "User" };
                prompt.push_str(&format!("{who}: {}\n", t.text));
            }
            prompt.push('\n');
        }
        prompt.push_str(&last.text);
        let system = format!(
            "{system}\nThe tools are the mcp__havre__* ones (search_mail, read_email, list_events, propose_event…)."
        );
        let options = claude_code::Run {
            mcp_config: Some(&config),
            allowed_tools: Some("mcp__havre"),
            max_turns: Some(10),
        };
        let result = claude.run(&system, &prompt, options, None).await;
        self.mcp.end_run(&secret);
        let text = result?;
        let mut answer = state.lock().await.answer.clone();
        answer.text = text.trim().to_owned();
        Ok(answer)
    }

    /// Runs one tool call for an outside conversation (the MCP server).
    pub async fn run_tool(&self, state: &mut ToolState, set: ToolSet, call: &ToolCall) -> Result<String> {
        if !tools(set).iter().any(|t| t.name == call.name) {
            return Err(Error::NotFound(format!("tool {}", call.name)));
        }
        if !allowed(&call.name, &self.settings().await?) {
            return Err(Error::Config(format!(
                "Havre's settings don't let AI use “{}” (Settings › AI › What it can read).",
                call.name
            )));
        }
        Session { ai: self, st: state }.run(call).await
    }
}

impl Session<'_> {
    async fn run(&mut self, call: &ToolCall) -> Result<String> {
        let input = &call.input;
        let text = |key: &str| input[key].as_str().unwrap_or_default().trim().to_owned();
        match call.name.as_str() {
            "search_mail" => {
                let query = text("query");
                self.step("search_mail", format!("Searched mail for “{query}”"));
                self.search_mail(&query, input["unread_only"].as_bool().unwrap_or(false)).await
            }
            "read_email" => {
                let id = text("id");
                let (account, folder, uid) = self
                    .st
                    .message_ids
                    .get(&id)
                    .cloned()
                    .ok_or_else(|| Error::NotFound(format!("email {id}")))?;
                let subject = self.st.answer.messages.get(&id).map(|h| h.subject.clone()).unwrap_or_default();
                self.step("read_email", format!("Read “{subject}”"));
                let raw = self.ai.accounts().engine(account).await?.raw(&folder, uid).await?;
                Ok(format!("<email id=\"{id}\">\n{}\n</email>", full_text(&raw, 6_000)))
            }
            "list_events" => {
                let (from, to) = (date(&text("from"))?, date(&text("to"))?);
                self.step(
                    "list_events",
                    format!("Looked at the calendar ({} – {})", from.format("%d %b"), to.format("%d %b")),
                );
                self.list_events(from, to).await
            }
            "find_free_time" => {
                let day = date(&text("date"))?;
                let minutes = input["duration_minutes"].as_i64().unwrap_or(30).clamp(5, 12 * 60);
                let earliest = time(input["earliest"].as_str().unwrap_or("09:00"))?;
                let latest = time(input["latest"].as_str().unwrap_or("18:00"))?;
                self.step(
                    "find_free_time",
                    format!("Looked for {minutes} free minutes on {}", day.format("%a %d %b")),
                );
                self.free_time(day, minutes, earliest, latest).await
            }
            "search_people" => {
                let query = text("query");
                self.step("search_people", format!("Looked up “{query}” in People"));
                let people = self.ai.db().people().search(&query, 8).await?;
                if people.is_empty() {
                    return Ok("Nobody found.".into());
                }
                Ok(people
                    .iter()
                    .map(|p| {
                        format!("{} <{}> ({} emails, {} meetings)", p.name, p.email, p.messages, p.meetings)
                    })
                    .collect::<Vec<_>>()
                    .join("\n"))
            }
            "unread_mail" => {
                self.step("unread_mail", "Looked at unread mail".into());
                self.unread_mail().await
            }
            "today" => {
                self.step("today", "Looked at today".into());
                self.ai.day_data(self.st.local).await
            }
            "create_draft" => {
                let subject = text("subject");
                self.step("create_draft", format!("Saved the draft “{subject}”"));
                self.create_draft(&text("from"), &text("to"), &subject, &text("body")).await
            }
            "propose_event" => {
                let start = self.local_ts(&text("start"))?;
                let end = self.local_ts(&text("end"))?;
                if end <= start {
                    return Err(Error::Config("the end must be after the start".into()));
                }
                let title = text("title");
                self.step("propose_event", format!("Prepared the event “{title}”"));
                self.st.answer.proposals.push(Proposal::Event {
                    title,
                    start,
                    end,
                    location: text("location"),
                    guests: input["guests"]
                        .as_array()
                        .into_iter()
                        .flatten()
                        .filter_map(|g| g.as_str().map(str::to_owned))
                        .collect(),
                    notes: text("notes"),
                });
                Ok("Shown to the user with a Create button. It is not created yet.".into())
            }
            "propose_move" => {
                let id = text("id");
                let event = self
                    .st
                    .answer
                    .events
                    .get(&id)
                    .cloned()
                    .ok_or_else(|| Error::NotFound(format!("event {id} (use list_events first)")))?;
                if event.read_only {
                    return Err(Error::Config(format!(
                        "“{}” can't be moved from Havre (someone else's meeting or a read-only calendar).",
                        event.title
                    )));
                }
                let start = self.local_ts(&text("start"))?;
                let end = self.local_ts(&text("end"))?;
                if end <= start {
                    return Err(Error::Config("the end must be after the start".into()));
                }
                self.step("propose_move", format!("Prepared moving “{}”", event.title));
                self.st.answer.proposals.push(Proposal::Move {
                    event: event.id,
                    title: event.title,
                    from_start: event.start,
                    from_end: event.end,
                    start,
                    end,
                });
                Ok("Shown to the user with a Confirm button. It is not moved yet.".into())
            }
            "search_notes" => {
                let (query, person) = (text("query"), text("person"));
                let label = if query.is_empty() {
                    format!("Looked for notes about {person}")
                } else {
                    format!("Searched notes for “{query}”")
                };
                self.step("search_notes", label);
                self.search_notes(&query, &person).await
            }
            "read_note" => {
                let id = text("id");
                let note = self.note_id(&id)?;
                let view = self.ai.db().notes().view(note).await?;
                self.step("read_note", format!("Read the note “{}”", view.note.title));
                let mut out =
                    format!("<note id=\"{id}\" title=\"{}\">\n{}\n</note>", view.note.title, view.note.body);
                let open: Vec<String> = view
                    .blocks
                    .iter()
                    .filter_map(|b| match b {
                        Block::Todo { line, done: false, text, task, .. } if !text.is_empty() => {
                            Some(format!(
                                "line {line}: {text}{}",
                                if task.is_some() { " (already a task)" } else { "" }
                            ))
                        }
                        _ => None,
                    })
                    .collect();
                if !open.is_empty() {
                    out.push_str(&format!("\nOpen to-dos:\n{}", open.join("\n")));
                }
                Ok(out)
            }
            "list_tasks" => {
                let done = input["done"].as_bool().unwrap_or(false);
                let due_by = match text("due_by") {
                    d if d.is_empty() => None,
                    d => Some(date(&d)?),
                };
                self.step(
                    "list_tasks",
                    match (done, due_by) {
                        (true, _) => "Looked at your recent done tasks".to_owned(),
                        (false, Some(d)) => format!("Looked at your tasks due by {}", d.format("%a %d %b")),
                        (false, None) => "Looked at your tasks".to_owned(),
                    },
                );
                self.list_tasks(&text("query"), &text("person"), due_by, done).await
            }
            "propose_task" => self.propose_task(input).await,
            "propose_note" => {
                let title = text("title");
                if title.is_empty() {
                    return Err(Error::Config("a note needs a title".into()));
                }
                let event = self.event_link(&text("event"))?;
                let people = self.people_links(input["people"].as_array().into_iter().flatten()).await?;
                self.step("propose_note", format!("Prepared the note “{title}”"));
                self.st.answer.proposals.push(Proposal::Note { title, body: text("body"), people, event });
                Ok("Shown to the user with a Confirm button. It is not written yet.".into())
            }
            "propose_email" => {
                let subject = text("subject");
                self.step("propose_email", format!("Prepared an email “{subject}”"));
                self.st.answer.proposals.push(Proposal::Email {
                    to: text("to"),
                    subject,
                    body: text("body"),
                });
                Ok("Shown to the user with an Open in compose button. It is not sent.".into())
            }
            other => Err(Error::NotFound(format!("tool {other}"))),
        }
    }

    fn step(&mut self, tool: &str, label: String) {
        self.st.answer.steps.push(Step { tool: tool.into(), label: label.clone() });
        // Every read and proposal goes to the activity log (spawned: the
        // log must never slow an answer down).
        let pool = self.ai.db().pool();
        let source = self.st.source.clone();
        let tool = tool.to_owned();
        tokio::spawn(async move { super::activity::log(&pool, &source, &tool, &label).await });
    }

    /// Gives `m1`, `m2`… to messages, the same id for the same message.
    fn remember(&mut self, hit: Hit) -> String {
        let key = (hit.account, hit.folder.clone(), hit.uid);
        if let Some((id, _)) = self.st.message_ids.iter().find(|(_, v)| **v == key) {
            return id.clone();
        }
        let id = format!("m{}", self.st.message_ids.len() + 1);
        self.st.message_ids.insert(id.clone(), key);
        self.st.answer.messages.insert(id.clone(), hit);
        id
    }

    async fn search_mail(&mut self, query: &str, unread_only: bool) -> Result<String> {
        // By meaning when the index is set up, else by words.
        let hits = match self.ai.search(query, 12).await {
            Ok(hits) if !hits.is_empty() => hits,
            _ => self.ai.db().search_all(query, 12).await?,
        };
        let accounts = self.ai.db().accounts().await?;
        let mut lines = Vec::new();
        for hit in hits {
            let unread: bool = sqlx::query_scalar(
                "SELECT unread FROM messages WHERE account_id = ? AND folder = ? AND uid = ?",
            )
            .bind(hit.account)
            .bind(&hit.folder)
            .bind(hit.uid as i64)
            .fetch_optional(&self.ai.db().pool())
            .await?
            .unwrap_or(false);
            if unread_only && !unread {
                continue;
            }
            let account =
                accounts.iter().find(|a| a.id == hit.account).map(|a| a.email.clone()).unwrap_or_default();
            let from = hit
                .from
                .as_ref()
                .map(|p| match (&p.name, &p.address) {
                    (Some(n), Some(a)) => format!("{n} <{a}>"),
                    (n, a) => n.clone().or(a.clone()).unwrap_or_default(),
                })
                .unwrap_or_default();
            let date = hit.date.clone().unwrap_or_default();
            let line = format!(
                "{} | {} | From: {from} | Subject: {} | folder {} of {account}{}",
                "{id}",
                date.get(..16).unwrap_or(&date),
                hit.subject,
                hit.folder,
                if unread { " | unread" } else { "" }
            );
            let id = self.remember(hit);
            lines.push(line.replacen("{id}", &id, 1));
            if lines.len() >= 8 {
                break;
            }
        }
        Ok(if lines.is_empty() { "No email found.".into() } else { lines.join("\n") })
    }

    async fn unread_mail(&mut self) -> Result<String> {
        let unread = self.ai.db().unread_inbox(15).await?;
        let mut lines = Vec::new();
        for m in unread.messages {
            let hit = Hit {
                account: m.account,
                folder: m.folder,
                uid: m.uid,
                subject: m.subject,
                from: m.from,
                date: m.date,
                score: 0.0,
            };
            let from =
                hit.from.as_ref().and_then(|p| p.name.clone().or(p.address.clone())).unwrap_or_default();
            let line =
                format!("From: {from} | Subject: {} | {}", hit.subject, hit.date.clone().unwrap_or_default());
            let id = self.remember(hit);
            lines.push(format!("{id} | {line}"));
        }
        if lines.is_empty() {
            return Ok("No unread email.".into());
        }
        lines.push(format!("({} unread in all)", unread.total));
        Ok(lines.join("\n"))
    }

    async fn create_draft(&mut self, from: &str, to: &str, subject: &str, body: &str) -> Result<String> {
        let accounts = self.ai.db().accounts().await?;
        let account = accounts
            .iter()
            .filter(|a| a.can_send)
            .find(|a| from.is_empty() || from.to_lowercase().contains(&a.email.to_lowercase()))
            .or_else(|| accounts.iter().find(|a| a.can_send))
            .ok_or_else(|| Error::Config("No account can send mail yet.".into()))?;
        let draft = self
            .ai
            .accounts()
            .new_draft(crate::compose::NewDraft {
                account: account.id,
                kind: crate::compose::DraftKind::New,
                folder: None,
                uid: None,
                to: to.to_owned(),
                subject: subject.to_owned(),
            })
            .await?;
        self.ai
            .db()
            .drafts()
            .update(
                draft.id,
                &crate::compose::DraftUpdate { body: Some(body.to_owned()), ..Default::default() },
            )
            .await?;
        Ok(format!(
            "Saved in Havre's Drafts (from {}). It is not sent: the user opens it in Havre to review and send.",
            account.email
        ))
    }

    // ---- Notes and tasks ------------------------------------------------------------

    /// `n1` → the note's id in the database.
    fn note_id(&self, id: &str) -> Result<i64> {
        self.st
            .answer
            .notes
            .get(id)
            .map(|n| n.id)
            .ok_or_else(|| Error::NotFound(format!("note {id} (use search_notes first)")))
    }

    /// Gives `n1`, `n2`… to notes, the same id for the same note.
    fn remember_note(&mut self, note: NoteSummary) -> String {
        if let Some((id, _)) = self.st.answer.notes.iter().find(|(_, n)| n.id == note.id) {
            return id.clone();
        }
        let id = format!("n{}", self.st.answer.notes.len() + 1);
        self.st.answer.notes.insert(id.clone(), note);
        id
    }

    async fn search_notes(&mut self, query: &str, person: &str) -> Result<String> {
        let store = self.ai.db().notes();
        let base = NoteQuery {
            person: (!person.is_empty()).then(|| person.to_owned()),
            limit: Some(8),
            ..NoteQuery::default()
        };
        let mut found = store.notes(&NoteQuery { q: Some(query.to_owned()), ..base.clone() }).await?;
        // Every word must match; models often add words the note doesn't
        // have ("notes about the Lyon kick-off"). Then: the notes with the
        // most of the words.
        let words: Vec<&str> = query.split_whitespace().filter(|w| w.chars().count() >= 3).collect();
        if found.is_empty() && words.len() > 1 {
            let mut scored: Vec<(usize, NoteSummary)> = Vec::new();
            for word in words {
                for note in store.notes(&NoteQuery { q: Some(word.to_owned()), ..base.clone() }).await? {
                    match scored.iter_mut().find(|(_, n)| n.id == note.id) {
                        Some((score, _)) => *score += 1,
                        None => scored.push((1, note)),
                    }
                }
            }
            // Highest score first (`Reverse` flips the order); the sort is
            // stable, so equals keep their order: the latest edited first.
            scored.sort_by_key(|(score, _)| std::cmp::Reverse(*score));
            found = scored.into_iter().map(|(_, n)| n).take(8).collect();
        }
        if found.is_empty() {
            return Ok("No note found.".into());
        }
        let mut lines = Vec::new();
        for note in found {
            let mut line =
                format!("{} | edited {}", note.title, self.st.local.at(note.updated_at).format("%Y-%m-%d"));
            if let Some(e) = &note.event {
                line.push_str(&format!(
                    " | meeting: {} ({})",
                    e.title,
                    self.st.local.at(e.start).format("%a %d %b %H:%M")
                ));
            }
            if !note.people.is_empty() {
                let names: Vec<String> =
                    note.people.iter().map(|p| p.name.clone().unwrap_or_else(|| p.email.clone())).collect();
                line.push_str(&format!(" | about {}", names.join(", ")));
            }
            if note.open_todos > 0 {
                line.push_str(&format!(" | {} open to-dos", note.open_todos));
            }
            line.push_str(&format!(" | {}", note.snippet));
            let id = self.remember_note(note);
            lines.push(format!("{id} | {line}"));
        }
        Ok(lines.join("\n"))
    }

    async fn list_tasks(
        &mut self,
        query: &str,
        person: &str,
        due_by: Option<NaiveDate>,
        done: bool,
    ) -> Result<String> {
        let store = self.ai.db().tasks();
        let lists = store.lists().await?;
        let tasks = store
            .tasks(&TaskQuery {
                done: Some(done),
                q: (!query.is_empty()).then(|| query.to_owned()),
                person: (!person.is_empty()).then(|| person.to_lowercase()),
                // A task's `due` is midnight UTC of its day: "due by Friday"
                // is before Saturday's midnight UTC.
                to: due_by.map(|d| utc_midnight(d + Duration::days(1))),
                limit: Some(20),
                ..TaskQuery::default()
            })
            .await?;
        if tasks.is_empty() {
            return Ok(if done { "No task done lately." } else { "No task to do." }.into());
        }
        let mut lines = Vec::new();
        for task in tasks {
            let mut line = format!("{} | {}", task.title, self.due_label(&task));
            if let Some(list) = lists.iter().find(|l| l.id == task.list) {
                line.push_str(&format!(" | list {} · {}", list.owner, list.name));
            }
            if let Some(p) = &task.links.person {
                line.push_str(&format!(" | with {}", p.name.clone().unwrap_or_else(|| p.email.clone())));
            }
            if let Some(e) = &task.links.event {
                line.push_str(&format!(" | for {}", e.title));
            }
            if let Some(m) = &task.links.mail {
                line.push_str(&format!(" | from the email “{}”", m.subject));
            }
            if task.done_at.is_some() {
                line.push_str(" | done");
            }
            if !task.notes.trim().is_empty() {
                line.push_str(&format!(
                    " | notes: {}",
                    task.notes.trim().chars().take(160).collect::<String>()
                ));
            }
            let id = format!("t{}", self.st.answer.tasks.len() + 1);
            lines.push(format!("{id} | {line}"));
            self.st.answer.tasks.insert(id, task);
        }
        Ok(lines.join("\n"))
    }

    /// "due Fri 18 Oct", "due Fri 18 Oct 10:00", "no date".
    fn due_label(&self, task: &Task) -> String {
        match (task.due_at, task.due) {
            (Some(at), _) => format!("due {}", self.st.local.at(at).format("%a %d %b %H:%M")),
            (None, Some(day)) => match chrono::DateTime::from_timestamp(day, 0) {
                Some(d) => format!("due {}", d.format("%a %d %b")),
                None => "no date".into(),
            },
            (None, None) => "no date".into(),
        }
    }

    async fn propose_task(&mut self, input: &serde_json::Value) -> Result<String> {
        let text = |key: &str| input[key].as_str().unwrap_or_default().trim().to_owned();
        let store = self.ai.db().tasks();
        let lists = store.lists().await?;
        // New tasks go to the default list (the one quick add uses).
        let list = lists
            .iter()
            .filter(|l| !l.read_only)
            .find(|l| l.is_default)
            .or_else(|| lists.iter().find(|l| !l.read_only))
            .ok_or_else(|| Error::Config("There's no task list to add it to yet.".into()))?;
        let list_name = format!("{} · {}", list.owner, list.name);

        // From a note's to-do: the task is the line, read like quick add
        // ("… fri" is due Friday), linked to the note's meeting and person.
        let note_key = text("note");
        if !note_key.is_empty() {
            if !allowed("read_note", &self.ai.settings().await?) {
                return Err(Error::Config("notes aren't available".into()));
            }
            let note = self.note_id(&note_key)?;
            let line = input["line"]
                .as_u64()
                .ok_or_else(|| Error::Config("give the to-do's line number from read_note".into()))?
                as usize;
            let view = self.ai.db().notes().view(note).await?;
            // An open to-do on that line, not already a task.
            let todo = view
                .blocks
                .iter()
                .find_map(|b| match b {
                    Block::Todo { line: l, done: false, text, task: None, .. }
                        if *l == line && !text.is_empty() =>
                    {
                        Some(text.clone())
                    }
                    _ => None,
                })
                .ok_or_else(|| {
                    Error::Config(format!("line {line} isn't an open to-do without a task (see read_note)"))
                })?;
            let full = view.note;
            let writable: Vec<(i64, String)> =
                lists.iter().filter(|l| !l.read_only).map(|l| (l.id, l.name.clone())).collect();
            let js_offset = -self.st.local.offset.local_minus_utc() / 60;
            let parsed =
                crate::tasks::parse::parse(&todo, chrono::Utc::now().timestamp(), js_offset, &writable);
            self.step("propose_task", format!("Prepared the task “{}”", parsed.title));
            self.st.answer.proposals.push(Proposal::Task {
                title: parsed.title,
                due: parsed.due,
                due_at: parsed.due_at,
                notes: String::new(),
                list: list_name,
                person: full.people.into_iter().next(),
                event: full.event,
                from_note: Some(NoteTodo { note, line, title: full.title }),
            });
            return Ok("Shown to the user with a Confirm button. It is not added yet.".into());
        }

        let title = text("title");
        if title.is_empty() {
            return Err(Error::Config("a task needs a title".into()));
        }
        let (due, due_at) = match text("due") {
            d if d.is_empty() => (None, None),
            d => {
                let day = date(&d)?;
                let at = match text("time") {
                    t if t.is_empty() => None,
                    t => Some(
                        self.st.local.midnight(day)
                            + time(&t)?.signed_duration_since(NaiveTime::MIN).num_seconds(),
                    ),
                };
                (Some(utc_midnight(day)), at)
            }
        };
        let person = match text("person") {
            p if p.is_empty() => None,
            p => self.people_links([&serde_json::Value::String(p)].into_iter()).await?.into_iter().next(),
        };
        let event = self.event_link(&text("event"))?;
        self.step("propose_task", format!("Prepared the task “{title}”"));
        self.st.answer.proposals.push(Proposal::Task {
            title,
            due,
            due_at,
            notes: text("notes"),
            list: list_name,
            person,
            event,
            from_note: None,
        });
        Ok("Shown to the user with a Confirm button. It is not added yet.".into())
    }

    /// `e2` (from list_events) → what a task or note keeps of the meeting.
    fn event_link(&self, id: &str) -> Result<Option<EventLink>> {
        if id.is_empty() {
            return Ok(None);
        }
        let event = self
            .st
            .answer
            .events
            .get(id)
            .ok_or_else(|| Error::NotFound(format!("event {id} (use list_events first)")))?;
        // Without a UID (rare), the link couldn't be found again: leave it out.
        Ok(event.uid.clone().map(|uid| EventLink { uid, title: event.title.clone(), start: event.start }))
    }

    /// Email addresses → people, with their name when People knows them.
    async fn people_links<'v>(
        &self,
        values: impl Iterator<Item = &'v serde_json::Value>,
    ) -> Result<Vec<PersonLink>> {
        let mut out: Vec<PersonLink> = Vec::new();
        for v in values {
            let Some(raw) = v.as_str() else { continue };
            // "Claire Martin <claire@x.fr>" or just the address.
            let email = raw.rsplit('<').next().unwrap_or(raw).trim_end_matches('>').trim().to_lowercase();
            if !email.contains('@') || out.iter().any(|p| p.email == email) {
                continue;
            }
            let name = self
                .ai
                .db()
                .people()
                .search(&email, 1)
                .await?
                .into_iter()
                .find(|p| p.email.eq_ignore_ascii_case(&email))
                .map(|p| p.name)
                .filter(|n| !n.is_empty());
            out.push(PersonLink { email, name });
        }
        Ok(out)
    }

    async fn day_events(&self, from: NaiveDate, to: NaiveDate) -> Result<Vec<Event>> {
        let start = self.st.local.midnight(from);
        let end = self.st.local.midnight(to + Duration::days(1));
        Ok(self
            .ai
            .agenda()
            .events(start, end)
            .await?
            .into_iter()
            .filter(|e| e.my_response.as_deref() != Some("declined"))
            .collect())
    }

    async fn list_events(&mut self, from: NaiveDate, to: NaiveDate) -> Result<String> {
        if (to - from).num_days() > 62 {
            return Err(Error::Config("ask for two months at most".into()));
        }
        let events = self.day_events(from, to).await?;
        if events.is_empty() {
            return Ok("No events.".into());
        }
        let mut lines = Vec::new();
        for e in events {
            let id = format!("e{}", self.st.answer.events.len() + 1);
            let mut line = format!("{id} | {} | {}", self.st.local.event_time(&e), e.title);
            if !e.location.is_empty() {
                line.push_str(&format!(" | at {}", e.location));
            }
            if let Some(o) = &e.organizer {
                line.push_str(&format!(" | organizer {o}"));
            }
            if let Some(r) = &e.my_response {
                line.push_str(&format!(" | your answer: {r}"));
            }
            lines.push(line);
            self.st.answer.events.insert(id, e);
        }
        Ok(lines.join("\n"))
    }

    async fn free_time(
        &self,
        day: NaiveDate,
        minutes: i64,
        earliest: NaiveTime,
        latest: NaiveTime,
    ) -> Result<String> {
        let at = |t: NaiveTime| {
            self.st.local.midnight(day) + t.signed_duration_since(NaiveTime::MIN).num_seconds()
        };
        let (open, close) = (at(earliest), at(latest));
        let mut busy: Vec<(i64, i64)> = self
            .day_events(day, day)
            .await?
            .into_iter()
            .filter(|e| !e.all_day)
            .map(|e| (e.start.max(open), e.end.min(close)))
            .filter(|(s, e)| s < e)
            .collect();
        busy.sort();
        let mut free = Vec::new();
        let mut cursor = open;
        for (s, e) in busy {
            if s - cursor >= minutes * 60 {
                free.push((cursor, s));
            }
            cursor = cursor.max(e);
        }
        if close - cursor >= minutes * 60 {
            free.push((cursor, close));
        }
        if free.is_empty() {
            return Ok(format!("No free {minutes} minutes on {}.", weekday_name(day)));
        }
        Ok(free
            .iter()
            .map(|(s, e)| {
                format!(
                    "free {}–{}",
                    self.st.local.at(*s).format("%H:%M"),
                    self.st.local.at(*e).format("%H:%M")
                )
            })
            .collect::<Vec<_>>()
            .join("\n"))
    }

    /// `2026-10-16T14:30` (local) → unix seconds.
    fn local_ts(&self, s: &str) -> Result<i64> {
        let s = s.trim().trim_end_matches('Z');
        let t = NaiveDateTime::parse_from_str(s, "%Y-%m-%dT%H:%M")
            .or_else(|_| NaiveDateTime::parse_from_str(s, "%Y-%m-%dT%H:%M:%S"))
            .or_else(|_| NaiveDateTime::parse_from_str(s, "%Y-%m-%d %H:%M"))
            .map_err(|_| Error::Config(format!("“{s}” isn't YYYY-MM-DDTHH:MM")))?;
        Ok(self.st.local.midnight(t.date()) + t.time().signed_duration_since(NaiveTime::MIN).num_seconds())
    }
}

/// A day as tasks keep it: midnight UTC of that date.
fn utc_midnight(day: NaiveDate) -> i64 {
    day.and_time(NaiveTime::MIN).and_utc().timestamp()
}

fn date(s: &str) -> Result<NaiveDate> {
    NaiveDate::parse_from_str(s.get(..10).unwrap_or(s), "%Y-%m-%d")
        .map_err(|_| Error::Config(format!("“{s}” isn't a YYYY-MM-DD date")))
}

fn time(s: &str) -> Result<NaiveTime> {
    NaiveTime::parse_from_str(s, "%H:%M").map_err(|_| Error::Config(format!("“{s}” isn't HH:MM")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{accounts::AccountManager, agenda::Agenda, notes::NoteInput, store::Database};
    use std::sync::Arc;

    /// An assistant over an empty database (no AI server: the tools are
    /// called directly, as an MCP client or the model would).
    async fn ai(db: &Database) -> Arc<Ai> {
        let accounts = AccountManager::start(db.clone()).await.unwrap();
        let agenda = Agenda::start(db.clone(), accounts.sender());
        Ai::start(accounts, agenda)
    }

    fn call(name: &str, input: serde_json::Value) -> ToolCall {
        ToolCall { id: "1".into(), name: name.into(), input }
    }

    async fn dock(db: &Database, app: &str, on: bool) {
        sqlx::query("INSERT INTO app_settings (id, enabled, position) VALUES (?, ?, 9) ON CONFLICT (id) DO UPDATE SET enabled = excluded.enabled")
            .bind(app)
            .bind(on)
            .execute(&db.pool())
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn notes_and_tasks_need_their_app_and_the_setting() {
        let db = Database::in_memory().await.unwrap();
        let ai = ai(&db).await;
        let mut st = ToolState::new(LocalTime::server(), "test");
        let search = call("search_notes", json!({ "query": "Lyon" }));
        // Not in the dock: refused (and not even listed for the model).
        assert!(ai.run_tool(&mut st, ToolSet::Assistant, &search).await.is_err());
        dock(&db, "notes", true).await;
        assert!(ai.run_tool(&mut st, ToolSet::Assistant, &search).await.is_ok());
        // In the dock, but Settings › AI says no.
        ai.update_settings(&super::super::settings::AiSettingsUpdate {
            read_notes: Some(false),
            ..Default::default()
        })
        .await
        .unwrap();
        assert!(ai.run_tool(&mut st, ToolSet::Assistant, &search).await.is_err());
    }

    #[tokio::test]
    async fn finds_reads_and_proposes_from_notes_and_tasks() {
        let db = Database::in_memory().await.unwrap();
        dock(&db, "notes", true).await;
        dock(&db, "tasks", true).await;
        let ai = ai(&db).await;
        db.notes()
            .create(&NoteInput {
                title: "Lyon kick-off — prep".into(),
                body: "Planning for three sites.\n\n## To-dos\n- [x] Book the train\n- [ ] Send Claire the draft planning"
                    .into(),
                people: vec![PersonLink { email: "claire@lyon.fr".into(), name: Some("Claire".into()) }],
                ..Default::default()
            })
            .await
            .unwrap();
        sqlx::query(
            "INSERT INTO tasks (list_id, remote_id, title, due, updated_at)
             VALUES ((SELECT id FROM task_lists LIMIT 1), 'r1', 'Prepare the slides', 1792281600, 0)",
        )
        .execute(&db.pool())
        .await
        .unwrap();
        let mut st = ToolState::new(LocalTime::server(), "test");
        // An async closure (Rust 2024): a small helper that can `.await`.
        let mut run =
            async |name: &str, input| ai.run_tool(&mut st, ToolSet::Assistant, &call(name, input)).await;

        // More words than the note has: the best match still comes back.
        let found = run("search_notes", json!({ "query": "notes about the Lyon kick-off" })).await.unwrap();
        assert!(found.starts_with("n1 | Lyon kick-off — prep"), "{found}");
        assert!(found.contains("1 open to-dos") && found.contains("about Claire"), "{found}");

        let note = run("read_note", json!({ "id": "n1" })).await.unwrap();
        assert!(note.contains("line 4: Send Claire the draft planning"), "{note}");
        assert!(!note.contains("Book the train (already"), "only open to-dos are listed");

        let tasks = run("list_tasks", json!({ "due_by": "2026-10-18" })).await.unwrap();
        assert!(tasks.starts_with("t1 | Prepare the slides | due Sun 18 Oct"), "{tasks}");
        assert_eq!(run("list_tasks", json!({ "due_by": "2026-10-17" })).await.unwrap(), "No task to do.");

        // A task from the note's to-do: its text, the note's person, and
        // nothing created yet.
        run("propose_task", json!({ "note": "n1", "line": 4 })).await.unwrap();
        run(
            "propose_note",
            json!({ "title": "Questions", "body": "- Badges?", "people": ["Claire <CLAIRE@lyon.fr>"] }),
        )
        .await
        .unwrap();
        assert!(run("propose_task", json!({ "note": "n1", "line": 3 })).await.is_err(), "a done line");
        let proposals = &st.answer.proposals;
        match &proposals[0] {
            Proposal::Task { title, person, from_note: Some(todo), list, .. } => {
                assert_eq!(title, "Send Claire the draft planning");
                assert_eq!(person.as_ref().unwrap().email, "claire@lyon.fr");
                assert_eq!((todo.line, todo.title.as_str()), (4, "Lyon kick-off — prep"));
                assert!(list.ends_with("Inbox"), "{list}");
            }
            other => panic!("{other:?}"),
        }
        match &proposals[1] {
            Proposal::Note { people, .. } => assert_eq!(people[0].email, "claire@lyon.fr"),
            other => panic!("{other:?}"),
        }
        let open: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM tasks").fetch_one(&db.pool()).await.unwrap();
        assert_eq!(open, 1, "proposing adds nothing");
    }

    #[test]
    fn a_day_is_midnight_utc() {
        assert_eq!(utc_midnight(NaiveDate::from_ymd_opt(2026, 10, 18).unwrap()), 1_792_281_600);
    }
}
