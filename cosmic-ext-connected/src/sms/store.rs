//! `SmsConversationStore` — owns SMS conversation state, message caches,
//! subscription orchestration, optimistic-send state, contacts, and SMS
//! notification dedup.
//!
//! M2: 35 SMS-touching fields migrated from `ConnectApplet`. Method bodies
//! remain stubbed; `app.rs` accesses fields directly via `self.sms.<field>`.
//! Field encapsulation tightens as method bodies fill in (M3–M5).

#![allow(dead_code)] // stub methods; remove once call sites land

use crate::app::{show_and_auto_close, DeviceInfo, LoadingPhase, Message, SmsLoadingState};
use crate::config::Config;
use crate::fl;
use crate::sms::{
    conversation_list_subscription, fetch_older_messages_async, request_attachment_async,
    send_new_sms_async, send_sms_async, view_conversation_list, view_message_thread,
    view_new_message, ConversationListParams, MessageThreadParams, NewMessageParams,
};
use crate::subscriptions::conversation_message_subscription;
use cosmic::iced::widget::scrollable;
use cosmic::iced::{clipboard, Subscription};
use cosmic::widget;
use cosmic::Element;
use kdeconnect_dbus::contacts::ContactLookup;
use kdeconnect_dbus::plugins::{
    is_address_valid, ConversationSummary, MessageType, SmsMessage, OPTIMISTIC_MESSAGE_UID,
};
use kdeconnect_dbus::{normalize_phone_number, phone_suffix};
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use tokio::sync::Mutex;
use zbus::Connection;

/// Read-only context the parent app passes to the store on each call.
///
/// `conn` is `Option` because the app may not yet have a D-Bus connection
/// when an SMS message arrives; arms that need it guard internally.
pub struct SmsCtx<'a> {
    pub conn: Option<&'a Arc<Mutex<Connection>>>,
    pub config: &'a Config,
    pub devices: &'a [DeviceInfo],
}

/// Reply from the store back to the parent app describing app-level
/// state changes the caller must apply.
#[derive(Debug)]
pub enum SmsReply {
    /// SMS view is closing — caller should reset `view_mode` to `DevicePage`.
    ExitSms,
    /// Emit a transient status message (3s auto-clear).
    Status(String),
    /// Set or clear `status_message` directly without auto-clear.
    /// Used for sticky "Loading…" indicators that pair with explicit clear.
    SetStatus(Option<String>),
    /// Surface an error via the app's `error` field.
    Error(String),
    /// New-message send succeeded: set status_message + return to ConversationList.
    NewMessageSent(String),
    /// No app-level state change required.
    NoOp,
}

/// Which SMS sub-view the parent app is rendering.
#[derive(Debug, Clone, Copy)]
pub enum SmsViewMode {
    ConversationList,
    MessageThread,
    NewMessage,
}

pub struct SmsConversationStore {
    // Active SMS device
    pub(crate) sms_device_id: Option<String>,
    pub(crate) sms_device_name: Option<String>,

    // Conversation list
    pub(crate) conversations: Vec<ConversationSummary>,
    pub(crate) sms_prefetch: Option<(String, Vec<ConversationSummary>)>,
    pub(crate) conversation_sync_active: bool,
    pub(crate) conversation_list_subscription_active: bool,
    pub(crate) message_sync_active: bool,
    pub(crate) conversation_load_active: bool,
    pub(crate) initial_load_complete: bool,

    // Active thread
    pub(crate) loading_thread_id: Option<i64>,
    pub(crate) known_message_ids: HashSet<i32>,
    pub(crate) current_thread_id: Option<i64>,
    pub(crate) current_thread_addresses: Option<Vec<String>>,
    pub(crate) current_thread_sub_id: Option<i64>,
    pub(crate) messages: Vec<SmsMessage>,
    pub(crate) sms_loading_state: SmsLoadingState,
    pub(crate) contacts: ContactLookup,
    pub(crate) conversation_list_key: u32,
    pub(crate) conversations_displayed: usize,

    // Reply compose / send
    pub(crate) sms_compose_text: String,
    pub(crate) sms_sending: bool,
    pub(crate) sms_sending_body: Option<String>,

    // Message pagination / scroll preservation
    pub(crate) messages_loaded_count: u32,
    pub(crate) messages_has_more: bool,
    pub(crate) scroll_offset_before_load: Option<f32>,
    pub(crate) content_height_before_load: Option<f32>,

    // New-message compose
    pub(crate) new_message_recipients: Vec<(String, String)>,
    pub(crate) new_message_recipient_input: String,
    pub(crate) new_message_body: String,
    pub(crate) new_message_sending: bool,
    pub(crate) contact_suggestions: Vec<(String, String)>,

    // SMS notification deduplication
    pub(crate) last_seen_sms: HashMap<i64, i64>,

    // Long-press copy
    pub(crate) pressed_bubble_uid: Option<i32>,
    pub(crate) pressed_bubble_body: Option<String>,
    pub(crate) show_copy_hint: bool,
}

impl SmsConversationStore {
    pub fn new() -> Self {
        Self {
            sms_device_id: None,
            sms_device_name: None,
            conversations: Vec::new(),
            sms_prefetch: None,
            conversation_sync_active: false,
            conversation_list_subscription_active: false,
            message_sync_active: false,
            conversation_load_active: false,
            initial_load_complete: false,
            loading_thread_id: None,
            known_message_ids: HashSet::new(),
            current_thread_id: None,
            current_thread_addresses: None,
            current_thread_sub_id: None,
            messages: Vec::new(),
            sms_loading_state: SmsLoadingState::Idle,
            contacts: ContactLookup::default(),
            conversation_list_key: 0,
            conversations_displayed: 10,
            sms_compose_text: String::new(),
            sms_sending: false,
            sms_sending_body: None,
            messages_loaded_count: 0,
            messages_has_more: true,
            scroll_offset_before_load: None,
            content_height_before_load: None,
            new_message_recipients: Vec::new(),
            new_message_recipient_input: String::new(),
            new_message_body: String::new(),
            new_message_sending: false,
            contact_suggestions: Vec::new(),
            last_seen_sms: HashMap::new(),
            pressed_bubble_uid: None,
            pressed_bubble_body: None,
            show_copy_hint: false,
        }
    }

    /// Check if loading more messages (pagination)
    pub(crate) fn is_loading_more_messages(&self) -> bool {
        matches!(self.sms_loading_state, SmsLoadingState::LoadingMoreMessages)
    }

    /// Find the latest conversation timestamp for a phone number.
    /// Uses suffix matching (last 10 digits) to handle format variations.
    pub(crate) fn find_conversation_timestamp(&self, phone: &str) -> Option<i64> {
        let phone_digits = normalize_phone_number(phone);
        let target_suffix = phone_suffix(&phone_digits);

        self.conversations
            .iter()
            .filter(|conv| {
                conv.addresses.iter().any(|addr| {
                    let addr_digits = normalize_phone_number(addr);
                    let addr_suffix = phone_suffix(&addr_digits);
                    target_suffix == addr_suffix
                })
            })
            .map(|conv| conv.timestamp)
            .max()
    }

    /// Generate contact suggestions with phone numbers sorted by conversation recency.
    /// Returns (contact_name, phone_number) tuples, limited to max_suggestions.
    pub(crate) fn generate_contact_suggestions(
        &self,
        query: &str,
        max_suggestions: usize,
    ) -> Vec<(String, String)> {
        if query.is_empty() {
            return Vec::new();
        }

        // Search for contacts matching the query (get more to account for multi-number expansion)
        let matching_contacts = self.contacts.search_by_name(query, max_suggestions);

        // Expand each contact into (name, phone, timestamp) entries
        let mut entries: Vec<(String, String, Option<i64>)> = Vec::new();
        for contact in matching_contacts {
            for phone in &contact.phone_numbers {
                let timestamp = self.find_conversation_timestamp(phone);
                entries.push((contact.name.clone(), phone.clone(), timestamp));
            }
        }

        // Sort by timestamp: most recent conversations first, then None (never contacted)
        entries.sort_by(|a, b| match (&b.2, &a.2) {
            (Some(ts_b), Some(ts_a)) => ts_b.cmp(ts_a), // Both have timestamps: recent first
            (Some(_), None) => std::cmp::Ordering::Less, // b has timestamp, a doesn't: b first
            (None, Some(_)) => std::cmp::Ordering::Greater, // a has timestamp, b doesn't: a first
            (None, None) => std::cmp::Ordering::Equal,  // Neither has timestamp: keep order
        });

        // Take up to max_suggestions and drop the timestamp
        entries
            .into_iter()
            .take(max_suggestions)
            .map(|(name, phone, _)| (name, phone))
            .collect()
    }

    /// Check if a phone number is already in the committed recipients list.
    /// Uses suffix matching (last 10 digits) to handle format variations.
    pub(crate) fn is_recipient_duplicate(&self, phone: &str) -> bool {
        let normalized = normalize_phone_number(phone);
        let suffix = phone_suffix(&normalized);
        self.new_message_recipients.iter().any(|(_, existing)| {
            let existing_normalized = normalize_phone_number(existing);
            phone_suffix(&existing_normalized) == suffix
        })
    }

    /// Generate contact suggestions filtered to exclude already-added recipients.
    pub(crate) fn generate_contact_suggestions_filtered(
        &self,
        query: &str,
        max: usize,
    ) -> Vec<(String, String)> {
        self.generate_contact_suggestions(query, max + self.new_message_recipients.len())
            .into_iter()
            .filter(|(_, phone)| !self.is_recipient_duplicate(phone))
            .take(max)
            .collect()
    }

    pub fn update(
        &mut self,
        msg: Message,
        ctx: &SmsCtx,
    ) -> (cosmic::app::Task<Message>, SmsReply) {
        match msg {
            // === Batch 1: Conversation list ===
            Message::ConversationsLoaded(convs) => {
                // Slow path: full sync complete from phone (legacy batch loading)
                tracing::info!(
                    "Background sync complete: {} conversations (had {} cached)",
                    convs.len(),
                    self.conversations.len()
                );
                // Only update if we got conversations back
                if !convs.is_empty() {
                    // Pre-populate last_seen_sms to prevent false notifications
                    // for messages that already exist in loaded conversations
                    for conv in &convs {
                        // Only update if we don't have a newer timestamp already
                        let current = self.last_seen_sms.get(&conv.thread_id).copied();
                        if current.is_none() || current < Some(conv.timestamp) {
                            self.last_seen_sms.insert(conv.thread_id, conv.timestamp);
                        }
                    }

                    self.conversations = convs;
                    self.conversation_list_key = self.conversation_list_key.wrapping_add(1);
                }
                // Background sync complete - clear sync indicator
                self.conversation_sync_active = false;
                // Reset loading state if still loading
                if matches!(
                    self.sms_loading_state,
                    SmsLoadingState::LoadingConversations(_)
                ) {
                    self.sms_loading_state = SmsLoadingState::Idle;
                }
                (cosmic::app::Task::none(), SmsReply::NoOp)
            }

            Message::SmsPrefetchReady(device_id, conversations) => {
                if !conversations.is_empty() {
                    self.sms_prefetch = Some((device_id, conversations));
                }
                (cosmic::app::Task::none(), SmsReply::NoOp)
            }

            // Subscription-based conversation list loading handlers
            Message::ConversationReceived {
                device_id,
                conversation,
            } => {
                // Guard: Only process if for current device
                if self.sms_device_id.as_ref() != Some(&device_id) {
                    tracing::debug!(
                        "Ignoring conversation for device {} (current: {:?})",
                        device_id,
                        self.sms_device_id
                    );
                    return (cosmic::app::Task::none(), SmsReply::NoOp);
                }

                // Update or insert conversation by thread_id
                if let Some(existing) = self
                    .conversations
                    .iter_mut()
                    .find(|c| c.thread_id == conversation.thread_id)
                {
                    // Only update if the new conversation has a newer timestamp
                    if conversation.timestamp > existing.timestamp {
                        *existing = conversation.clone();
                        tracing::debug!(
                            "Updated conversation thread {} (newer timestamp)",
                            conversation.thread_id
                        );
                    }
                } else {
                    // Insert new conversation
                    self.conversations.push(conversation.clone());
                    tracing::debug!("Added new conversation thread {}", conversation.thread_id);
                }

                // Re-sort by timestamp (newest first) and truncate
                self.conversations
                    .sort_by_key(|c| std::cmp::Reverse(c.timestamp));
                self.conversations
                    .truncate(kdeconnect_dbus::plugins::MAX_CONVERSATIONS);

                // Update last_seen for notification deduplication
                let current = self.last_seen_sms.get(&conversation.thread_id).copied();
                if current.is_none() || current < Some(conversation.timestamp) {
                    self.last_seen_sms
                        .insert(conversation.thread_id, conversation.timestamp);
                }

                // Transition from loading spinner to showing data (but keep sync indicator)
                if matches!(
                    self.sms_loading_state,
                    SmsLoadingState::LoadingConversations(_)
                ) {
                    self.sms_loading_state = SmsLoadingState::Idle;
                }
                (cosmic::app::Task::none(), SmsReply::NoOp)
            }

            Message::ConversationSyncStarted { device_id } => {
                // Guard: Only process if for current device
                if self.sms_device_id.as_ref() != Some(&device_id) {
                    return (cosmic::app::Task::none(), SmsReply::NoOp);
                }

                tracing::debug!("Conversation sync started for device {}", device_id);
                // Update loading phase to indicate we're waiting for signals
                if matches!(
                    self.sms_loading_state,
                    SmsLoadingState::LoadingConversations(LoadingPhase::Connecting)
                ) {
                    self.sms_loading_state =
                        SmsLoadingState::LoadingConversations(LoadingPhase::Requesting);
                }
                (cosmic::app::Task::none(), SmsReply::NoOp)
            }

            Message::ConversationSyncComplete { device_id } => {
                // Guard: Only process if for current device
                if self.sms_device_id.as_ref() != Some(&device_id) {
                    return (cosmic::app::Task::none(), SmsReply::NoOp);
                }

                tracing::info!(
                    "Conversation sync indicator dismissed for device {}, {} conversations loaded",
                    device_id,
                    self.conversations.len()
                );

                // Clear sync indicator only. The subscription keeps running
                // to catch new conversations while the SMS view is open.
                self.conversation_sync_active = false;

                // Only dismiss loading spinner if we have data to show.
                // If conversations is empty, keep the spinner — the subscription
                // continues listening and may receive conversations later.
                // This prevents a false "no conversations" message on cold start
                // when the phone is slow to respond.
                if matches!(
                    self.sms_loading_state,
                    SmsLoadingState::LoadingConversations(_)
                ) && !self.conversations.is_empty()
                {
                    self.sms_loading_state = SmsLoadingState::Idle;
                }
                (cosmic::app::Task::none(), SmsReply::NoOp)
            }

            Message::ContactsLoaded(device_id, contacts) => {
                // Only update if contacts are for the current SMS device
                if self.sms_device_id.as_ref() == Some(&device_id) {
                    tracing::info!(
                        "Loaded {} contacts for device {}",
                        contacts.len(),
                        device_id
                    );
                    self.contacts = contacts;
                } else {
                    tracing::debug!(
                        "Ignoring contacts for device {} (current: {:?})",
                        device_id,
                        self.sms_device_id
                    );
                }
                (cosmic::app::Task::none(), SmsReply::NoOp)
            }

            Message::LoadMoreConversations => {
                // Show 10 more conversations (up to total available)
                self.conversations_displayed =
                    (self.conversations_displayed + 10).min(self.conversations.len());
                (cosmic::app::Task::none(), SmsReply::NoOp)
            }

            Message::SmsError(err) => {
                tracing::error!("SMS error: {}", err);
                self.sms_loading_state = SmsLoadingState::Idle;
                // Also clear subscription state on error
                self.conversation_load_active = false;
                self.conversation_list_subscription_active = false;
                self.message_sync_active = false;
                let status = format!("SMS error: {}", err);
                (cosmic::app::Task::none(), SmsReply::Status(status))
            }

            // === Batch 2: Message thread / scroll / bubble ===
            Message::OlderMessagesLoaded(
                thread_id,
                older_msgs,
                has_more_heuristic,
                total_count,
            ) => {
                // Only reset to Idle if we're currently loading more messages
                if matches!(self.sms_loading_state, SmsLoadingState::LoadingMoreMessages) {
                    self.sms_loading_state = SmsLoadingState::Idle;
                }

                if self.current_thread_id == Some(thread_id) {
                    // Filter out messages already known (safety net for signal cross-talk)
                    let older_msgs: Vec<_> = older_msgs
                        .into_iter()
                        .filter(|m| !self.known_message_ids.contains(&m.uid))
                        .collect();
                    for m in &older_msgs {
                        self.known_message_ids.insert(m.uid);
                    }

                    if !older_msgs.is_empty() {
                        let prepended_count = older_msgs.len();
                        tracing::info!(
                            "Prepending {} older messages to thread {} (had {}, total: {:?})",
                            prepended_count,
                            thread_id,
                            self.messages.len(),
                            total_count
                        );

                        // Prepend older messages (they come sorted oldest first)
                        let mut combined = older_msgs;
                        combined.append(&mut self.messages);
                        self.messages = combined;

                        // Update loaded count
                        self.messages_loaded_count = self.messages.len() as u32;

                        // Use total_count for accurate pagination if available,
                        // otherwise fall back to heuristic
                        self.messages_has_more = match total_count {
                            Some(total) => (self.messages.len() as u64) < total,
                            None => has_more_heuristic,
                        };

                        // Calculate scroll adjustment to preserve user's position
                        // When we prepend messages, the content shifts down. We need to
                        // scroll down by the estimated height of the prepended content.
                        if let (Some(old_offset), Some(old_height)) = (
                            self.scroll_offset_before_load.take(),
                            self.content_height_before_load.take(),
                        ) {
                            // Estimate prepended content height (avg ~70px per message)
                            const ESTIMATED_MSG_HEIGHT: f32 = 70.0;
                            let prepended_height = prepended_count as f32 * ESTIMATED_MSG_HEIGHT;
                            let new_content_height = old_height + prepended_height;
                            let new_offset = old_offset + prepended_height;

                            // Calculate relative offset (0.0 = top, 1.0 = bottom)
                            let relative_y = (new_offset / new_content_height).clamp(0.0, 1.0);

                            tracing::debug!(
                                "Scroll adjustment: old_offset={:.1}, prepended_height={:.1}, new_relative_y={:.3}",
                                old_offset, prepended_height, relative_y
                            );

                            return (
                                scrollable::snap_to(
                                    widget::Id::new("message-thread"),
                                    scrollable::RelativeOffset {
                                        x: Some(0.0),
                                        y: Some(relative_y),
                                    },
                                ),
                                SmsReply::NoOp,
                            );
                        }
                    } else {
                        tracing::info!("No older messages returned for thread {}", thread_id);
                        // No more messages available
                        self.messages_has_more = false;
                        // Clear scroll state
                        self.scroll_offset_before_load = None;
                        self.content_height_before_load = None;
                    }
                }
                (cosmic::app::Task::none(), SmsReply::NoOp)
            }
            Message::MessageThreadScrolled(viewport) => {
                // Prefetch older messages when user scrolls near the top
                // Trigger when within 100 pixels of the top and not already loading
                const PREFETCH_THRESHOLD_PX: f32 = 100.0;

                let scroll_offset = viewport.absolute_offset().y;
                let content_height = viewport.content_bounds().height;

                if scroll_offset < PREFETCH_THRESHOLD_PX
                    && self.messages_has_more
                    && !self.is_loading_more_messages()
                    && self.initial_load_complete
                    && !self.messages.is_empty()
                {
                    tracing::debug!(
                        "Prefetching older messages (scroll_y={:.1}px, content_height={:.1}px)",
                        scroll_offset,
                        content_height
                    );

                    // Store scroll state for position preservation after load
                    self.scroll_offset_before_load = Some(scroll_offset);
                    self.content_height_before_load = Some(content_height);

                    // Trigger loading older messages (same logic as LoadMoreMessages)
                    if let (Some(conn), Some(device_id), Some(thread_id)) = (
                        ctx.conn,
                        self.sms_device_id.as_ref(),
                        self.current_thread_id,
                    ) {
                        self.sms_loading_state = SmsLoadingState::LoadingMoreMessages;
                        let start_index = self.messages_loaded_count;
                        let count = ctx.config.messages_per_page;

                        return (
                            cosmic::app::Task::perform(
                                fetch_older_messages_async(
                                    conn.clone(),
                                    device_id.clone(),
                                    thread_id,
                                    start_index,
                                    count,
                                ),
                                cosmic::Action::App,
                            ),
                            SmsReply::NoOp,
                        );
                    }
                }
                (cosmic::app::Task::none(), SmsReply::NoOp)
            }

            Message::BubblePressStarted { uid, body } => {
                self.pressed_bubble_uid = Some(uid);
                self.pressed_bubble_body = Some(body);
                self.show_copy_hint = false;
                // Spawn delayed task - fires after 500ms to show hint
                (
                    cosmic::app::Task::perform(
                        async {
                            tokio::time::sleep(std::time::Duration::from_millis(500)).await;
                        },
                        |_| cosmic::Action::App(Message::BubbleHintTimer),
                    ),
                    SmsReply::NoOp,
                )
            }

            Message::BubblePressReleased => {
                // Clear pressed state - cancels the long-press action
                self.pressed_bubble_uid = None;
                self.pressed_bubble_body = None;
                self.show_copy_hint = false;
                (cosmic::app::Task::none(), SmsReply::NoOp)
            }

            Message::BubbleHintTimer => {
                // 500ms elapsed - show "Hold to copy" hint and start 1.5s timer for actual copy
                if self.pressed_bubble_uid.is_some() {
                    self.show_copy_hint = true;
                    return (
                        cosmic::app::Task::perform(
                            async {
                                tokio::time::sleep(std::time::Duration::from_millis(1500)).await;
                            },
                            |_| cosmic::Action::App(Message::BubbleLongPressComplete),
                        ),
                        SmsReply::NoOp,
                    );
                }
                (cosmic::app::Task::none(), SmsReply::NoOp)
            }

            Message::BubbleLongPressComplete => {
                // 2s total elapsed - copy to clipboard if still pressed
                if let Some(body) = self.pressed_bubble_body.take() {
                    self.pressed_bubble_uid = None;
                    self.show_copy_hint = false;
                    return (clipboard::write(body), SmsReply::NoOp);
                }
                (cosmic::app::Task::none(), SmsReply::NoOp)
            }

            // Subscription-based message loading handlers
            Message::ConversationLoadStarted { thread_id } => {
                // D-Bus request fired, subscription is now active
                if self.current_thread_id == Some(thread_id) {
                    tracing::debug!(
                        "Conversation {} load started, waiting for subscription signals",
                        thread_id
                    );
                    // Update loading phase to indicate we're waiting for signals
                    if matches!(
                        self.sms_loading_state,
                        SmsLoadingState::LoadingMessages(LoadingPhase::Connecting)
                    ) {
                        self.sms_loading_state =
                            SmsLoadingState::LoadingMessages(LoadingPhase::Requesting);
                    }
                }
                (cosmic::app::Task::none(), SmsReply::NoOp)
            }
            Message::ConversationMessageReceived { thread_id, message } => {
                // Guard: Only process if still viewing this thread
                if self.current_thread_id != Some(thread_id) {
                    tracing::debug!(
                        "Ignoring message for thread {} (current: {:?})",
                        thread_id,
                        self.current_thread_id
                    );
                    return (cosmic::app::Task::none(), SmsReply::NoOp);
                }

                // Reconcile optimistic message: if this incoming sent message
                // matches our optimistic insert's body within a 5-minute window,
                // upgrade the optimistic entry in-place instead of inserting a duplicate.
                if message.uid != OPTIMISTIC_MESSAGE_UID
                    && message.message_type == MessageType::Sent
                {
                    if let Some(pos) = self.messages.iter().position(|m| {
                        m.uid == OPTIMISTIC_MESSAGE_UID
                            && m.message_type == MessageType::Sent
                            && m.body == message.body
                            && (message.date - m.date).unsigned_abs() < 300_000
                    }) {
                        tracing::info!(
                            "Reconciling optimistic message with real uid={}",
                            message.uid
                        );
                        self.messages[pos].uid = message.uid;
                        self.messages[pos].date = message.date;
                        self.known_message_ids.remove(&OPTIMISTIC_MESSAGE_UID);
                        self.known_message_ids.insert(message.uid);
                        self.sms_sending_body = None;
                        return (cosmic::app::Task::none(), SmsReply::NoOp);
                    }
                }

                // Deduplication: skip if already have this message
                if self.known_message_ids.contains(&message.uid) {
                    tracing::debug!(
                        "Skipping duplicate message uid={} for thread {}",
                        message.uid,
                        thread_id
                    );
                    return (cosmic::app::Task::none(), SmsReply::NoOp);
                }
                self.known_message_ids.insert(message.uid);

                // Extract sub_id from first message (for MMS group messaging)
                if self.current_thread_sub_id.is_none() {
                    self.current_thread_sub_id = Some(message.sub_id);
                    tracing::debug!("Set sub_id to {} for thread {}", message.sub_id, thread_id);
                }

                // Check if this confirms our pending sent message
                let confirmed_send = self.sms_sending_body.is_some()
                    && message.message_type == MessageType::Sent
                    && self.sms_sending_body.as_deref() == Some(message.body.as_str());
                if confirmed_send {
                    tracing::info!("Confirmed delivery of sent message uid={}", message.uid);
                    self.sms_sending_body = None;
                }

                // Insert message in sorted order by date
                let insert_pos = self
                    .messages
                    .iter()
                    .position(|m| m.date > message.date)
                    .unwrap_or(self.messages.len());
                self.messages.insert(insert_pos, message);

                tracing::debug!(
                    "Added message to thread {}, now have {} messages",
                    thread_id,
                    self.messages.len()
                );

                // Clear loading spinner after first message, show sync indicator instead
                if matches!(self.sms_loading_state, SmsLoadingState::LoadingMessages(_)) {
                    self.sms_loading_state = SmsLoadingState::Idle;
                    self.message_sync_active = true;
                }

                // Scroll to bottom when a sent message is confirmed.
                if confirmed_send {
                    return (
                        scrollable::snap_to(
                            widget::Id::new("message-thread"),
                            scrollable::RelativeOffset::END.into(),
                        ),
                        SmsReply::NoOp,
                    );
                }
                // While the initial load is in flight, keep the newest message
                // in view as messages stream in. Necessary because the daemon's
                // worker emits `conversationLoaded` only when it actually
                // fetched fresh phone data (see daemon's
                // `addMessages()` → `conversationLoaded`); for cached-store
                // hits the worker just emits per-message signals and finishes
                // silently, so `ConversationStoreLoaded` and
                // `ConversationLoadComplete` never fire and the scroll stays
                // pinned at the top of an oldest-first list. Bounded by
                // `initial_load_complete` so we don't yank a user reading
                // older messages when a new SMS arrives later.
                if !self.initial_load_complete {
                    return (
                        scrollable::snap_to(
                            widget::Id::new("message-thread"),
                            scrollable::RelativeOffset::END.into(),
                        ),
                        SmsReply::NoOp,
                    );
                }
                (cosmic::app::Task::none(), SmsReply::NoOp)
            }
            Message::ConversationStoreLoaded {
                thread_id,
                total_count,
            } => {
                // Local store read complete - scroll to show messages while
                // continuing to listen for phone response data
                if self.current_thread_id != Some(thread_id) {
                    return (cosmic::app::Task::none(), SmsReply::NoOp);
                }

                tracing::info!(
                    "Local store loaded for thread {}: {} messages displayed, {} total in store",
                    thread_id,
                    self.messages.len(),
                    total_count
                );

                // Update pagination state
                // Note: total_count from conversationLoaded reflects the LOCAL store count,
                // which may be 0 or 1 after a reboot. Use a heuristic instead: if we've
                // loaded a full page, there are likely more messages available.
                self.messages_loaded_count = self.messages.len() as u32;
                self.messages_has_more =
                    if total_count > 0 && (self.messages.len() as u64) < total_count {
                        true
                    } else {
                        self.messages.len() >= ctx.config.messages_per_page as usize
                    };

                // Scroll to bottom to show latest messages
                if !self.messages.is_empty() {
                    return (
                        scrollable::snap_to(
                            widget::Id::new("message-thread"),
                            scrollable::RelativeOffset::END.into(),
                        ),
                        SmsReply::NoOp,
                    );
                }
                (cosmic::app::Task::none(), SmsReply::NoOp)
            }
            Message::ConversationLoadComplete {
                thread_id,
                total_count,
            } => {
                // Guard: Only process if still viewing this thread
                if self.current_thread_id != Some(thread_id) {
                    tracing::debug!(
                        "Ignoring load complete for thread {} (current: {:?})",
                        thread_id,
                        self.current_thread_id
                    );
                    return (cosmic::app::Task::none(), SmsReply::NoOp);
                }

                tracing::info!(
                    "Conversation {} loading complete: {} messages loaded, {} total in conversation",
                    thread_id,
                    self.messages.len(),
                    total_count
                );

                // Messages are already sorted during insertion, but sort again as safety
                self.messages.sort_by_key(|m| m.date);

                // Update pagination state
                // Note: total_count from conversationLoaded reflects the LOCAL store count,
                // not the phone's total. Use a heuristic: if we've loaded a full page
                // worth of messages, there are likely more available.
                self.messages_loaded_count = self.messages.len() as u32;
                self.messages_has_more =
                    if total_count > 0 && (self.messages.len() as u64) < total_count {
                        true
                    } else {
                        self.messages.len() >= ctx.config.messages_per_page as usize
                    };

                // Update last_seen_sms with the newest message timestamp
                if let Some(newest) = self.messages.iter().map(|m| m.date).max() {
                    let current = self.last_seen_sms.get(&thread_id).copied();
                    if current.is_none() || current < Some(newest) {
                        self.last_seen_sms.insert(thread_id, newest);
                    }
                }

                // Clear sync indicator and loading state. The subscription keeps
                // running to catch new messages (including sent message echoes).
                self.message_sync_active = false;
                self.initial_load_complete = true;
                self.sms_loading_state = SmsLoadingState::Idle;

                // Scroll to bottom if we loaded messages
                if !self.messages.is_empty() {
                    return (
                        scrollable::snap_to(
                            widget::Id::new("message-thread"),
                            scrollable::RelativeOffset::END.into(),
                        ),
                        SmsReply::NoOp,
                    );
                }
                (cosmic::app::Task::none(), SmsReply::NoOp)
            }

            // === Batch 3: Reply send + attachments + notification ===
            Message::SmsComposeInput(text) => {
                self.sms_compose_text = text;
                (cosmic::app::Task::none(), SmsReply::NoOp)
            }
            Message::SendSms => {
                tracing::info!("SendSms triggered");
                tracing::info!(
                    "State: conn={}, device_id={:?}, thread_id={:?}, text_empty={}, sending={}",
                    ctx.conn.is_some(),
                    self.sms_device_id,
                    self.current_thread_id,
                    self.sms_compose_text.is_empty(),
                    self.sms_sending
                );
                if let (Some(conn), Some(device_id), Some(thread_id)) = (
                    ctx.conn,
                    self.sms_device_id.as_ref(),
                    self.current_thread_id,
                ) {
                    if !self.sms_compose_text.is_empty() && !self.sms_sending {
                        let message_text = self.sms_compose_text.clone();
                        self.sms_sending = true;
                        self.sms_sending_body = Some(message_text.clone());
                        tracing::info!(
                            "Dispatching send_sms_async via replyToConversation for thread_id={}",
                            thread_id
                        );
                        return (
                            cosmic::app::Task::perform(
                                send_sms_async(
                                    conn.clone(),
                                    device_id.clone(),
                                    thread_id,
                                    message_text,
                                ),
                                cosmic::Action::App,
                            ),
                            SmsReply::NoOp,
                        );
                    } else {
                        tracing::warn!(
                            "SendSms conditions not met: text_empty={}, sending={}",
                            self.sms_compose_text.is_empty(),
                            self.sms_sending
                        );
                    }
                } else {
                    tracing::warn!("SendSms missing required state");
                }
                (cosmic::app::Task::none(), SmsReply::NoOp)
            }
            Message::SmsSendResult(result) => {
                self.sms_sending = false;
                match result {
                    Ok(sent_body) => {
                        tracing::info!("SMS sent successfully");
                        self.sms_compose_text.clear();

                        if let Some(thread_id) = self.current_thread_id {
                            let now_ms = std::time::SystemTime::now()
                                .duration_since(std::time::UNIX_EPOCH)
                                .map(|d| d.as_millis() as i64)
                                .unwrap_or(0);

                            // Clone before move into conversation preview
                            let optimistic_body = sent_body.clone();

                            // Update conversation list preview so it reflects the new message
                            // when user navigates back
                            if let Some(conv) = self
                                .conversations
                                .iter_mut()
                                .find(|c| c.thread_id == thread_id)
                            {
                                conv.last_message = sent_body;
                                conv.timestamp = now_ms;
                            }
                            self.conversations
                                .sort_by_key(|c| std::cmp::Reverse(c.timestamp));

                            // Insert optimistic message if echo hasn't already arrived.
                            // sms_sending_body is cleared by confirmed_send in
                            // ConversationMessageReceived if the echo arrived before
                            // SmsSendResult — skip to avoid duplicate.
                            if self.sms_sending_body.is_some() {
                                let optimistic = SmsMessage {
                                    body: optimistic_body,
                                    addresses: self
                                        .current_thread_addresses
                                        .clone()
                                        .unwrap_or_default(),
                                    date: now_ms,
                                    message_type: MessageType::Sent,
                                    read: true,
                                    thread_id,
                                    uid: OPTIMISTIC_MESSAGE_UID,
                                    sub_id: self.current_thread_sub_id.unwrap_or(-1),
                                    attachments: vec![],
                                };
                                self.messages.push(optimistic);
                                self.known_message_ids.insert(OPTIMISTIC_MESSAGE_UID);
                                self.sms_sending_body = None;

                                // No subscription restart needed — the message subscription
                                // runs as long as the thread is open and will catch the
                                // phone's echo naturally for optimistic reconciliation.

                                return (
                                    scrollable::snap_to(
                                        widget::Id::new("message-thread"),
                                        scrollable::RelativeOffset::END.into(),
                                    ),
                                    SmsReply::NoOp,
                                );
                            }
                        }

                        (cosmic::app::Task::none(), SmsReply::Status(fl!("sms-sent")))
                    }
                    Err(err) => {
                        tracing::error!("SMS send error: {}", err);
                        self.sms_sending_body = None;
                        let status = format!("{}: {}", fl!("sms-failed"), err);
                        (cosmic::app::Task::none(), SmsReply::Status(status))
                    }
                }
            }

            // Attachment messages
            Message::OpenAttachment {
                device_id,
                device_name,
                part_id,
                unique_identifier,
            } => {
                // Check if KDE Connect has already cached this attachment
                // KDE Connect daemon caches to ~/.cache/kdeconnect.daemon/<device-name>/
                let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".to_string());
                let cache_dir = std::path::PathBuf::from(home)
                    .join(".cache/kdeconnect.daemon")
                    .join(&device_name);
                let cached_path = cache_dir.join(&unique_identifier);

                if cached_path.exists() {
                    // Already cached — open immediately
                    let path_str = cached_path.to_string_lossy().to_string();
                    return (
                        cosmic::app::Task::perform(
                            async move {
                                let _ = tokio::process::Command::new("xdg-open")
                                    .arg(&path_str)
                                    .spawn();
                            },
                            |_| cosmic::Action::App(Message::ClearStatusMessage),
                        ),
                        SmsReply::NoOp,
                    );
                }

                // Not cached — request from phone via D-Bus
                if let Some(conn) = ctx.conn {
                    return (
                        cosmic::app::Task::perform(
                            request_attachment_async(
                                conn.clone(),
                                device_id,
                                device_name,
                                part_id,
                                unique_identifier,
                            ),
                            cosmic::Action::App,
                        ),
                        SmsReply::SetStatus(Some(fl!("loading-attachment"))),
                    );
                }
                (cosmic::app::Task::none(), SmsReply::NoOp)
            }
            Message::AttachmentReady(file_path) => {
                (
                    cosmic::app::Task::perform(
                        async move {
                            let _ = tokio::process::Command::new("xdg-open")
                                .arg(&file_path)
                                .spawn();
                        },
                        |_| cosmic::Action::App(Message::ClearStatusMessage),
                    ),
                    SmsReply::SetStatus(None),
                )
            }
            Message::AttachmentError(err) => {
                tracing::error!("Attachment error: {}", err);
                (
                    cosmic::app::Task::none(),
                    SmsReply::Status(fl!("attachment-failed")),
                )
            }

            Message::SmsNotificationReceived(device_id, message) => {
                // Freshness check: only notify for messages received within the last 30 seconds.
                // This prevents false notifications when fetching historical messages and handles
                // cross-process deduplication (COSMIC spawns multiple applet instances).
                let now_ms = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.as_millis() as i64)
                    .unwrap_or(0);
                let message_age_ms = now_ms - message.date;
                if message_age_ms > 30_000 {
                    // Message is older than 30 seconds, skip notification
                    return (cosmic::app::Task::none(), SmsReply::NoOp);
                }

                // Check if we've already seen this message (deduplication)
                let last_seen = self.last_seen_sms.get(&message.thread_id).copied();
                if last_seen.is_some() && last_seen >= Some(message.date) {
                    // Already seen this message or an older one
                    return (cosmic::app::Task::none(), SmsReply::NoOp);
                }

                // Update last seen timestamp for this thread
                self.last_seen_sms.insert(message.thread_id, message.date);

                // Capture config settings
                let show_sender = ctx.config.sms_notification_show_sender;
                let show_content = ctx.config.sms_notification_show_content;
                let timeout_ms = ctx.config.notification_timeout_secs * 1000;
                let message_body = message.body.clone();

                // Resolve sender name: use cached contacts if available, otherwise load from disk
                let cached_sender_name = if show_sender {
                    let has_cached_contacts = self.sms_device_id.as_ref() == Some(&device_id)
                        && !self.contacts.is_empty();
                    if has_cached_contacts {
                        Some(self.contacts.get_group_display_name(&message.addresses, 3))
                    } else {
                        None
                    }
                } else {
                    None
                };

                let addresses = message.addresses.clone();

                // Show notification asynchronously
                (
                    cosmic::app::Task::perform(
                        async move {
                            // Build summary: resolve sender name if needed and not already cached
                            let summary = if show_sender {
                                let sender_name = match cached_sender_name {
                                    Some(name) => name,
                                    None => {
                                        let contacts = ContactLookup::load_for_device(&device_id).await;
                                        contacts.get_group_display_name(&addresses, 3)
                                    }
                                };
                                fl!("sms-notification-title-from", sender = sender_name)
                            } else {
                                fl!("sms-notification-title")
                            };

                            let body = if show_content {
                                message_body
                            } else {
                                fl!("sms-notification-body-hidden")
                            };

                            let mut notification = notify_rust::Notification::new();
                            notification
                                .summary(&summary)
                                .body(&body)
                                .icon("phone-symbolic")
                                .appname("Connected")
                                .timeout(notify_rust::Timeout::Never);
                            show_and_auto_close(notification, timeout_ms, "SMS").await;
                        },
                        |_| cosmic::Action::App(Message::RefreshDevices),
                    ),
                    SmsReply::NoOp,
                )
            }

            // === Batch 4: New-message compose ===
            Message::NewMessageRecipientInput(text) => {
                self.contact_suggestions = self.generate_contact_suggestions_filtered(&text, 10);
                self.new_message_recipient_input = text;
                (cosmic::app::Task::none(), SmsReply::NoOp)
            }
            Message::NewMessageBodyInput(text) => {
                self.new_message_body = text;
                (cosmic::app::Task::none(), SmsReply::NoOp)
            }
            Message::AddManualRecipient => {
                let input = self.new_message_recipient_input.trim().to_string();
                if is_address_valid(&input) && !self.is_recipient_duplicate(&input) {
                    let display = self.contacts.get_name_or_number(&input);
                    self.new_message_recipients.push((display, input));
                    self.new_message_recipient_input.clear();
                    self.contact_suggestions.clear();
                    return (
                        widget::text_input::focus(widget::Id::new("new-message-recipient")),
                        SmsReply::NoOp,
                    );
                }
                (cosmic::app::Task::none(), SmsReply::NoOp)
            }
            Message::RemoveRecipient(index) => {
                if index < self.new_message_recipients.len() {
                    self.new_message_recipients.remove(index);
                }
                (cosmic::app::Task::none(), SmsReply::NoOp)
            }
            Message::SelectContact(_name, phone) => {
                if !self.is_recipient_duplicate(&phone) {
                    let display = self.contacts.get_name_or_number(&phone);
                    self.new_message_recipients.push((display, phone));
                    self.new_message_recipient_input.clear();
                    self.contact_suggestions.clear();
                    return (
                        widget::text_input::focus(widget::Id::new("new-message-recipient")),
                        SmsReply::NoOp,
                    );
                }
                (cosmic::app::Task::none(), SmsReply::NoOp)
            }
            Message::SendNewMessage => {
                if let (Some(conn), Some(device_id)) = (ctx.conn, self.sms_device_id.as_ref()) {
                    if !self.new_message_recipients.is_empty()
                        && !self.new_message_body.is_empty()
                        && !self.new_message_sending
                    {
                        let recipients: Vec<String> = self
                            .new_message_recipients
                            .iter()
                            .map(|(_, phone)| phone.clone())
                            .collect();
                        let message = self.new_message_body.clone();
                        self.new_message_sending = true;
                        return (
                            cosmic::app::Task::perform(
                                send_new_sms_async(
                                    conn.clone(),
                                    device_id.clone(),
                                    recipients,
                                    message,
                                ),
                                cosmic::Action::App,
                            ),
                            SmsReply::NoOp,
                        );
                    }
                }
                (cosmic::app::Task::none(), SmsReply::NoOp)
            }
            Message::NewMessageSendResult(result) => {
                self.new_message_sending = false;
                match &result {
                    Ok(msg) => {
                        tracing::info!("New message send result: {}", msg);
                        // Clear fields and return to conversation list
                        let success_msg = msg.clone();
                        self.new_message_recipients.clear();
                        self.new_message_recipient_input.clear();
                        self.new_message_body.clear();
                        // Enable subscription to catch the new conversation when the phone
                        // syncs back. The subscription listens over a longer window than a
                        // one-shot fetch, giving the phone time to process the send and
                        // emit a conversationCreated signal.
                        if self.sms_device_id.is_some() {
                            self.conversation_list_subscription_active = true;
                            self.conversation_sync_active = true;
                        }
                        (
                            cosmic::app::Task::none(),
                            SmsReply::NewMessageSent(success_msg),
                        )
                    }
                    Err(err) => {
                        tracing::error!("New message send error: {}", err);
                        (
                            cosmic::app::Task::none(),
                            SmsReply::Status(format!("Send failed: {}", err)),
                        )
                    }
                }
            }

            // Catch-all: non-SMS variants and SMS variants not yet migrated
            // Phase C will narrow `app.rs` delegation to the migrated subset,
            // so this arm only fires for genuine routing mistakes after M3.
            _ => unreachable!("non-SMS Message routed to SmsConversationStore"),
        }
    }

    /// Render the active SMS sub-view.
    ///
    /// `status_message` is owned by the parent app and threaded through for
    /// the message-thread view's send-confirmation/error banner.
    pub fn view<'a>(
        &'a self,
        mode: SmsViewMode,
        status_message: Option<&'a str>,
    ) -> Element<'a, Message> {
        match mode {
            SmsViewMode::ConversationList => view_conversation_list(ConversationListParams {
                device_name: self.sms_device_name.as_deref(),
                conversations: &self.conversations,
                conversations_displayed: self.conversations_displayed,
                contacts: &self.contacts,
                loading_state: &self.sms_loading_state,
                sync_active: self.conversation_sync_active,
            }),
            SmsViewMode::MessageThread => {
                let thread = view_message_thread(MessageThreadParams {
                    device_id: self.sms_device_id.as_deref().unwrap_or(""),
                    device_name: self.sms_device_name.as_deref().unwrap_or(""),
                    thread_addresses: self.current_thread_addresses.as_deref(),
                    messages: &self.messages,
                    contacts: &self.contacts,
                    loading_state: &self.sms_loading_state,
                    sms_compose_text: &self.sms_compose_text,
                    sms_sending: self.sms_sending,
                    sync_active: self.message_sync_active,
                    pressed_bubble_uid: self.pressed_bubble_uid,
                    show_copy_hint: self.show_copy_hint,
                    status_message,
                });
                // popup_container uses Shrink height internally, which sets a
                // compression flag on iced's layout limits. Under compression,
                // the flex layout processes all children in document order and a
                // scrollable's intrinsic content size consumes all available
                // height, leaving 0 for the compose row below it. A Fixed height
                // wrapper is the only way to clear that flag (Fill doesn't);
                // the value is capped to popup_container's 1000px max.
                widget::container(thread)
                    .height(cosmic::iced::Length::Fixed(10_000.0))
                    .width(cosmic::iced::Length::Fill)
                    .into()
            }
            SmsViewMode::NewMessage => view_new_message(NewMessageParams {
                recipients: &self.new_message_recipients,
                recipient_input: &self.new_message_recipient_input,
                body: &self.new_message_body,
                sending: self.new_message_sending,
                contact_suggestions: &self.contact_suggestions,
            }),
        }
    }

    /// SMS-state-driven subscriptions: conversation-list refresh and the
    /// per-thread message subscription. The unconditional SMS/call notification
    /// subscriptions stay in `app.rs::subscription()` because they're gated on
    /// device reachability + config, not store state.
    pub fn subscriptions(&self, config: &Config) -> Vec<Subscription<Message>> {
        let mut subs: Vec<Subscription<Message>> = Vec::new();

        // Conversation list subscription (incremental loading + background sync)
        if self.conversation_list_subscription_active {
            if let Some(device_id) = self.sms_device_id.clone() {
                subs.push(Subscription::run_with(
                    ("conversation_list", device_id.clone()),
                    |(_, device_id)| conversation_list_subscription(device_id.clone()),
                ));
            }
        }

        // Per-thread message subscription (incremental message loading)
        if self.conversation_load_active {
            if let (Some(thread_id), Some(device_id)) =
                (self.loading_thread_id, self.sms_device_id.clone())
            {
                let messages_per_page = config.messages_per_page;
                subs.push(Subscription::run_with(
                    (
                        "conversation_messages",
                        thread_id,
                        device_id.clone(),
                        messages_per_page,
                    ),
                    |(_, thread_id, device_id, messages_per_page)| {
                        conversation_message_subscription(
                            *thread_id,
                            device_id.clone(),
                            *messages_per_page,
                        )
                    },
                ));
            }
        }

        subs
    }

    pub fn open(
        &mut self,
        _device_id: String,
        _device_name: Option<String>,
        _ctx: &SmsCtx,
    ) -> cosmic::app::Task<Message> {
        unimplemented!()
    }

    pub fn close(&mut self) {
        unimplemented!()
    }

    pub fn handle_notification(
        &mut self,
        _device_id: String,
        _message: SmsMessage,
        _ctx: &SmsCtx,
    ) -> cosmic::app::Task<Message> {
        unimplemented!()
    }
}

impl Default for SmsConversationStore {
    fn default() -> Self {
        Self::new()
    }
}
