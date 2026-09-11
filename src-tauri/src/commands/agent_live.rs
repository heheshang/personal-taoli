//! Folding turn progress into the shared status (PORT-01-M).
//!
//! The session reports [`TurnProgress`] as it runs; this module turns that
//! stream into the shape the interface polls. It exists because the status is
//! shared state behind a mutex: the fold has to happen under that lock, and
//! keeping it in one place is what stops the interface and the record from
//! disagreeing about what happened.
//!
//! Two bounds are deliberate:
//!
//! * **Streaming text is capped.** A turn can produce megabytes of reasoning
//!   and command output. The polled status is serialised on every tick, so it
//!   keeps a tail of each rather than everything — the full text stays in the
//!   turn's own record and in the trace.
//! * **A finished turn clears the live view.** Once the turn ends, its record
//!   carries the authoritative tool calls and final message; leaving the live
//!   copy in place would render the same work twice.
//!
//! Locking uses `std::sync::Mutex` to match the surrounding command layer: this
//! tracker and [`super::agent`] guard the *same* `Arc<Mutex<AgentStatus>>`, so
//! the two must agree on the type. Poisoning is tolerated rather than unwrapped
//! — a panic while folding progress must not make the status unreadable for the
//! interface, which is how an operator would diagnose it.

use std::sync::{Arc, Mutex, PoisonError};

use personal_taoli_core::agent::progress::{ItemKind, ItemState, ProgressItem, TurnProgress};

use super::dto::{AgentLive, AgentLiveItem, AgentStatus, AgentTokens};

/// How much streaming text to keep per stream, in characters.
///
/// Enough to render a useful tail of reasoning or command output; far short of
/// the megabytes a long turn can produce.
const TEXT_TAIL: usize = 4_000;

/// How much output to keep per item.
const ITEM_OUTPUT_TAIL: usize = 4_000;

/// Keeps the tail of a string, appending `chunk`.
fn push_capped(target: &mut String, chunk: &str) {
    target.push_str(chunk);
    if target.chars().count() > TEXT_TAIL {
        let skip = target.chars().count() - TEXT_TAIL;
        *target = target.chars().skip(skip).collect();
    }
}

/// Same, for an item's own output buffer.
fn push_item_output(target: &mut String, chunk: &str) {
    target.push_str(chunk);
    if target.chars().count() > ITEM_OUTPUT_TAIL {
        let skip = target.chars().count() - ITEM_OUTPUT_TAIL;
        *target = target.chars().skip(skip).collect();
    }
}

fn kind_token(kind: ItemKind) -> &'static str {
    match kind {
        ItemKind::Reasoning => "reasoning",
        ItemKind::Command => "command",
        ItemKind::FileChange => "file_change",
        ItemKind::ToolCall => "tool_call",
        ItemKind::Message => "message",
        ItemKind::Other => "other",
    }
}

fn state_token(state: ItemState) -> &'static str {
    match state {
        ItemState::Running => "running",
        ItemState::Completed => "completed",
        ItemState::Failed => "failed",
        ItemState::Declined => "declined",
    }
}

fn live_item(item: ProgressItem) -> AgentLiveItem {
    AgentLiveItem {
        item_id: item.item_id,
        kind: kind_token(item.kind).to_string(),
        title: item.title,
        detail: item.detail,
        state: state_token(item.state).to_string(),
        output: item.output,
        exit_code: item.exit_code,
        duration_ms: item.duration_ms,
    }
}

/// Folds progress events into `status.live`.
pub struct LiveTracker {
    status: Arc<Mutex<AgentStatus>>,
    turn_seq: u64,
}

impl LiveTracker {
    pub fn new(status: Arc<Mutex<AgentStatus>>, turn_seq: u64) -> Self {
        Self { status, turn_seq }
    }

    /// The sink the session calls. Returned as a boxed closure so the call site
    /// passes it straight into `TurnContext`.
    pub fn sink(self: Arc<Self>) -> Arc<dyn Fn(TurnProgress) + Send + Sync> {
        Arc::new(move |event| self.apply(event))
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, AgentStatus> {
        self.status.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Mutates the live block for the turn this tracker belongs to.
    ///
    /// Ignores events for a different turn, so a straggling notification from a
    /// previous turn cannot repaint the current one.
    fn with_live(&self, mutate: impl FnOnce(&mut AgentLive)) {
        let mut status = self.lock();
        let Some(live) = status.live.as_mut() else {
            return;
        };
        if live.turn_seq != self.turn_seq {
            return;
        }
        mutate(live);
    }

    fn apply(&self, event: TurnProgress) {
        match event {
            TurnProgress::Started { .. } => {
                self.with_live(|live| live.stage = "thinking".to_string());
            }
            TurnProgress::ReasoningDelta { item_id, text } => {
                // 归到**它自己的 item** 上，而不是一个聚合字段。
                //
                // 此前文本进的是聚合 `live.reasoning`，而该字段属于「正在进行中的这一轮」，
                // 轮次一结束就被清空——于是推理行永远是空的，且整段推理**不留在记录里**。
                // 增量本身带着 `itemId`，归属信息一直在，只是被丢掉了。
                self.with_live(|live| {
                    if let Some(item) = live.items.iter_mut().find(|item| item.item_id == item_id) {
                        push_item_output(&mut item.output, &text);
                    }
                    live.stage = "thinking".to_string();
                });
            }
            // 助手正文仍走聚合字段：它在界面上的呈现是**散文**（Markdown 块），
            // 不是可折叠的行——答复就该可读，而不是藏在一个三角后面。
            TurnProgress::MessageDelta { text, .. } => {
                self.with_live(|live| {
                    push_capped(&mut live.message, &text);
                    live.stage = "writing".to_string();
                });
            }
            TurnProgress::ItemStarted(item) => {
                let entry = live_item(item);
                self.with_live(|live| {
                    // The runtime can re-announce an item; keep the first
                    // title and update state rather than appending a duplicate.
                    match live.items.iter_mut().find(|it| it.item_id == entry.item_id) {
                        // 重新宣告同一个 item 时**保留已收到的输出**：增量可能先于
                        // 这条宣告到达，整体覆盖会把它们抹掉。
                        Some(existing) => {
                            let output = std::mem::take(&mut existing.output);
                            *existing = entry;
                            existing.output = output;
                        }
                        None => live.items.push(entry),
                    }
                    live.stage = "working".to_string();
                });
            }
            TurnProgress::ItemOutput { item_id, text } => {
                self.with_live(|live| {
                    if let Some(item) = live.items.iter_mut().find(|it| it.item_id == item_id) {
                        push_item_output(&mut item.output, &text);
                    }
                    live.stage = "working".to_string();
                });
            }
            TurnProgress::ItemFinished {
                item_id,
                state,
                output,
                exit_code,
                duration_ms,
            } => {
                self.with_live(|live| {
                    if let Some(item) = live.items.iter_mut().find(|it| it.item_id == item_id) {
                        item.state = state_token(state).to_string();
                        item.exit_code = exit_code.or(item.exit_code);
                        item.duration_ms = duration_ms.or(item.duration_ms);
                        // The final payload repeats what the deltas already
                        // delivered; only use it when nothing streamed, so the
                        // tail does not get the middle of the output twice.
                        if item.output.is_empty()
                            && let Some(output) = output
                        {
                            push_item_output(&mut item.output, &output);
                        }
                    }
                });
            }
            TurnProgress::Tokens(usage) => {
                self.with_live(|live| {
                    live.tokens = Some(AgentTokens {
                        input_tokens: usage.input_tokens,
                        cached_input_tokens: usage.cached_input_tokens,
                        output_tokens: usage.output_tokens,
                        reasoning_output_tokens: usage.reasoning_output_tokens,
                        total_tokens: usage.total_tokens,
                        context_window: usage.context_window,
                    });
                });
            }
            TurnProgress::ApprovalRequested { .. } => {
                // The summary itself reaches the interface through `pending`,
                // which the session fills from the request; only the stage is
                // the live block's business.
                self.with_live(|live| live.stage = "awaiting_approval".to_string());
            }
            TurnProgress::ApprovalResolved { .. } => {
                self.with_live(|live| live.stage = "working".to_string());
            }
            TurnProgress::Stage(stage) => {
                self.with_live(|live| {
                    live.stage = match stage {
                        personal_taoli_core::agent::progress::Stage::Done => "done".to_string(),
                        personal_taoli_core::agent::progress::Stage::Working => {
                            "working".to_string()
                        }
                        personal_taoli_core::agent::progress::Stage::Thinking => {
                            "thinking".to_string()
                        }
                    };
                });
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use personal_taoli_core::agent::progress::{Stage, TokenUsage};

    fn status_with_live(turn_seq: u64) -> Arc<Mutex<AgentStatus>> {
        Arc::new(Mutex::new(AgentStatus {
            phase: "running",
            program: None,
            thread_id: None,
            trace_path: None,
            pending_approval: None,
            turns: Vec::new(),
            skills: Vec::new(),
            access_level: "ask".to_string(),
            live: Some(AgentLive {
                turn_seq,
                stage: "thinking".to_string(),
                message: String::new(),
                items: Vec::new(),
                tokens: None,
            }),
            error: None,
        }))
    }

    fn tracker(turn_seq: u64) -> (Arc<Mutex<AgentStatus>>, Arc<LiveTracker>) {
        let status = status_with_live(turn_seq);
        let tracker = Arc::new(LiveTracker::new(Arc::clone(&status), turn_seq));
        (status, tracker)
    }

    fn items(status: &Arc<Mutex<AgentStatus>>) -> Vec<AgentLiveItem> {
        status
            .lock()
            .unwrap()
            .live
            .as_ref()
            .map(|live| live.items.clone())
            .unwrap_or_default()
    }

    #[test]
    fn a_command_is_visible_from_start_through_output_to_exit() {
        let (status, tracker) = tracker(1);

        tracker.apply(TurnProgress::ItemStarted(ProgressItem {
            item_id: "call-1".to_string(),
            kind: ItemKind::Command,
            title: "/bin/zsh -lc 'sleep 3'".to_string(),
            detail: Some("/tmp".to_string()),
            state: ItemState::Running,
            output: String::new(),
            exit_code: None,
            duration_ms: None,
        }));

        // Visible while running, before any output arrives: this is the whole
        // point of streaming — the interface must show work in progress.
        let running = items(&status);
        assert_eq!(running.len(), 1);
        assert_eq!(running[0].state, "running");
        assert_eq!(running[0].kind, "command");
        assert_eq!(running[0].title, "/bin/zsh -lc 'sleep 3'");
        assert_eq!(running[0].detail.as_deref(), Some("/tmp"));

        // Output streams in chunks and is appended.
        tracker.apply(TurnProgress::ItemOutput {
            item_id: "call-1".to_string(),
            text: "first\n".to_string(),
        });
        tracker.apply(TurnProgress::ItemOutput {
            item_id: "call-1".to_string(),
            text: "second\n".to_string(),
        });
        assert_eq!(items(&status)[0].output, "first\nsecond\n");

        tracker.apply(TurnProgress::ItemFinished {
            item_id: "call-1".to_string(),
            state: ItemState::Completed,
            output: Some("first\nsecond\n".to_string()),
            exit_code: Some(0),
            duration_ms: Some(3895),
        });
        let done = items(&status);
        assert_eq!(done[0].state, "completed");
        assert_eq!(done[0].exit_code, Some(0));
        assert_eq!(done[0].duration_ms, Some(3895));
        // The final payload must not be appended on top of the streamed text.
        assert_eq!(done[0].output, "first\nsecond\n", "output was doubled");
    }

    #[test]
    fn a_final_output_is_used_when_nothing_streamed() {
        let (status, tracker) = tracker(1);
        tracker.apply(TurnProgress::ItemStarted(ProgressItem {
            item_id: "c".to_string(),
            kind: ItemKind::Command,
            title: "echo hi".to_string(),
            detail: None,
            state: ItemState::Running,
            output: String::new(),
            exit_code: None,
            duration_ms: None,
        }));
        // The runtime sends `aggregatedOutput` whole for short commands.
        tracker.apply(TurnProgress::ItemFinished {
            item_id: "c".to_string(),
            state: ItemState::Completed,
            output: Some("hi\n".to_string()),
            exit_code: Some(0),
            duration_ms: None,
        });
        assert_eq!(items(&status)[0].output, "hi\n");
    }

    #[test]
    fn streaming_text_drives_the_stage_and_is_kept() {
        let (status, tracker) = tracker(1);
        // The item is announced first, as the runtime does, then its deltas
        // arrive — the text must land on that item, not in a side channel.
        tracker.apply(TurnProgress::ItemStarted(ProgressItem {
            item_id: "r".to_string(),
            kind: ItemKind::Reasoning,
            title: "reasoning".to_string(),
            detail: None,
            state: ItemState::Running,
            output: String::new(),
            exit_code: None,
            duration_ms: None,
        }));
        tracker.apply(TurnProgress::ReasoningDelta {
            item_id: "r".to_string(),
            text: "We need".to_string(),
        });
        tracker.apply(TurnProgress::ReasoningDelta {
            item_id: "r".to_string(),
            text: " to run it".to_string(),
        });

        let live = status.lock().unwrap().live.clone().unwrap();
        assert_eq!(live.stage, "thinking");
        // The assertion that matters: the reasoning row carries the reasoning.
        // Before this change the text went to an aggregate field, so the row was
        // blank and the reasoning vanished when the turn ended.
        assert_eq!(
            live.items
                .iter()
                .find(|item| item.item_id == "r")
                .unwrap()
                .output,
            "We need to run it"
        );

        // The assistant's reply still streams into the aggregate: it renders as
        // prose, not as a collapsible row.
        tracker.apply(TurnProgress::MessageDelta {
            item_id: "m".to_string(),
            text: "done".to_string(),
        });
        let live = status.lock().unwrap().live.clone().unwrap();
        assert_eq!(live.message, "done");
        assert_eq!(live.stage, "writing");
    }

    /// A delta whose item was never announced must not be attributed to some
    /// other row, nor panic — it is simply not yet placeable.
    #[test]
    fn a_delta_for_an_unknown_item_is_ignored() {
        let (status, tracker) = tracker(1);
        tracker.apply(TurnProgress::ReasoningDelta {
            item_id: "never-announced".to_string(),
            text: "orphan".to_string(),
        });
        let live = status.lock().unwrap().live.clone().unwrap();
        assert!(live.items.is_empty());
        assert_eq!(live.stage, "thinking");
    }

    /// A re-announcement of the same item keeps whatever text already arrived.
    ///
    /// Without this, a late `item/started` would wipe the deltas that preceded it
    /// — the ordering is not guaranteed by the protocol, only usual.
    #[test]
    fn a_re_announcement_keeps_the_output_already_received() {
        let (status, tracker) = tracker(1);
        let announce = |tracker: &LiveTracker| {
            tracker.apply(TurnProgress::ItemStarted(ProgressItem {
                item_id: "r".to_string(),
                kind: ItemKind::Reasoning,
                title: "reasoning".to_string(),
                detail: None,
                state: ItemState::Running,
                output: String::new(),
                exit_code: None,
                duration_ms: None,
            }));
        };
        announce(&tracker);
        tracker.apply(TurnProgress::ReasoningDelta {
            item_id: "r".to_string(),
            text: "kept".to_string(),
        });
        // Re-announcement arrives late.
        announce(&tracker);
        let live = status.lock().unwrap().live.clone().unwrap();
        assert_eq!(live.items.len(), 1);
        assert_eq!(live.items[0].output, "kept");
    }

    #[test]
    fn a_repeated_item_announcement_updates_rather_than_duplicates() {
        let (status, tracker) = tracker(1);
        for _ in 0..2 {
            tracker.apply(TurnProgress::ItemStarted(ProgressItem {
                item_id: "same".to_string(),
                kind: ItemKind::Command,
                title: "echo".to_string(),
                detail: None,
                state: ItemState::Running,
                output: String::new(),
                exit_code: None,
                duration_ms: None,
            }));
        }
        assert_eq!(items(&status).len(), 1, "the row must not be duplicated");
    }

    #[test]
    fn tokens_are_mirrored() {
        let (status, tracker) = tracker(1);
        tracker.apply(TurnProgress::Tokens(TokenUsage {
            input_tokens: 7_472,
            cached_input_tokens: 7_296,
            output_tokens: 221,
            reasoning_output_tokens: 19,
            total_tokens: 7_693,
            context_window: Some(121_600),
        }));
        let tokens = status.lock().unwrap().live.clone().unwrap().tokens.unwrap();
        assert_eq!(tokens.total_tokens, 7_693);
        assert_eq!(tokens.context_window, Some(121_600));
    }

    /// A notification from a previous turn must not repaint the current one.
    ///
    /// The live block belongs to turn 1 while the tracker belongs to turn 2 —
    /// the state a straggling delta arrives in once the next turn has begun.
    #[test]
    fn events_for_another_turn_are_ignored() {
        let status = status_with_live(1);
        let stale = LiveTracker::new(Arc::clone(&status), 2);
        stale.apply(TurnProgress::MessageDelta {
            item_id: "m".to_string(),
            text: "stale".to_string(),
        });
        assert!(
            status
                .lock()
                .unwrap()
                .live
                .as_ref()
                .unwrap()
                .message
                .is_empty(),
            "a previous turn's delta must not reach the current live block"
        );

        // The matching tracker does apply, so the guard is not simply dropping
        // everything.
        let current = LiveTracker::new(Arc::clone(&status), 1);
        current.apply(TurnProgress::MessageDelta {
            item_id: "m".to_string(),
            text: "current".to_string(),
        });
        assert_eq!(
            status.lock().unwrap().live.as_ref().unwrap().message,
            "current"
        );
    }

    #[test]
    fn a_missing_live_block_is_not_an_error() {
        // The turn can finish and clear the view while a late event is in
        // flight; that must be a no-op, not a panic.
        let status = Arc::new(Mutex::new(AgentStatus {
            phase: "ready",
            program: None,
            thread_id: None,
            trace_path: None,
            pending_approval: None,
            turns: Vec::new(),
            skills: Vec::new(),
            access_level: "ask".to_string(),
            live: None,
            error: None,
        }));
        let tracker = LiveTracker::new(Arc::clone(&status), 1);
        tracker.apply(TurnProgress::Stage(Stage::Done));
        assert!(status.lock().unwrap().live.is_none());
    }

    #[test]
    fn long_output_keeps_a_bounded_tail() {
        let (status, tracker) = tracker(1);
        tracker.apply(TurnProgress::ItemStarted(ProgressItem {
            item_id: "c".to_string(),
            kind: ItemKind::Command,
            title: "chatty".to_string(),
            detail: None,
            state: ItemState::Running,
            output: String::new(),
            exit_code: None,
            duration_ms: None,
        }));
        for i in 0..500 {
            tracker.apply(TurnProgress::ItemOutput {
                item_id: "c".to_string(),
                text: format!("line {i}\n"),
            });
        }
        let output = items(&status)[0].output.clone();
        assert!(
            output.chars().count() <= ITEM_OUTPUT_TAIL,
            "unbounded output would be serialised on every poll: {} chars",
            output.chars().count()
        );
        // The tail is what survived, so the most recent line is present.
        assert!(
            output.ends_with("line 499\n"),
            "{}",
            &output[output.len().saturating_sub(40)..]
        );
        assert!(!output.starts_with("line 0\n"));
    }
}
