//! `SmsConversationStore` — owns SMS conversation state, message caches,
//! subscription orchestration, optimistic-send state, contacts, and SMS
//! notification dedup.
//!
//! M2: 35 SMS-touching fields migrated from `ConnectApplet`. Method bodies
//! remain stubbed; `app.rs` accesses fields directly via `self.sms.<field>`.
//! Field encapsulation tightens as method bodies fill in (M3–M5).

#![allow(dead_code)] // stub methods; remove once call sites land

use crate::app::{DeviceInfo, Message, SmsLoadingState};
use crate::config::Config;
use cosmic::iced::Subscription;
use cosmic::Element;
use kdeconnect_dbus::contacts::ContactLookup;
use kdeconnect_dbus::plugins::{ConversationSummary, SmsMessage};
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use tokio::sync::Mutex;
use zbus::Connection;

/// Read-only context the parent app passes to the store on each call.
pub struct SmsCtx<'a> {
    pub conn: &'a Arc<Mutex<Connection>>,
    pub config: &'a Config,
    pub devices: &'a [DeviceInfo],
}

/// Reply from the store back to the parent app describing app-level
/// state changes the caller must apply.
#[derive(Debug)]
pub enum SmsReply {
    /// SMS view is closing — caller should reset `view_mode` to `DevicePage`.
    ExitSms,
    /// Emit a transient status message via the app's normal status flow.
    Status(String),
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

    pub fn update(
        &mut self,
        _msg: Message,
        _ctx: &SmsCtx,
    ) -> (cosmic::app::Task<Message>, SmsReply) {
        unimplemented!()
    }

    pub fn view(&self, _mode: SmsViewMode) -> Element<'_, Message> {
        unimplemented!()
    }

    pub fn subscriptions(&self, _config: &Config) -> Vec<Subscription<Message>> {
        unimplemented!()
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
