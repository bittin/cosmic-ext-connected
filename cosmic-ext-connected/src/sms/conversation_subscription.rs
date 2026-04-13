//! Subscription for incremental conversation list loading via D-Bus signals.
//!
//! This module provides a subscription that listens for conversationCreated and
//! conversationUpdated signals to provide real-time UI updates as conversations
//! are received from the phone.

use crate::app::Message;
use crate::constants::dbus::RETRY_DELAY_SECS;
use crate::constants::sms::{
    CONVERSATION_LIST_PHONE_WAIT_MS, CONVERSATION_TIMEOUT_CACHED_SECS,
};
use futures_util::StreamExt;
use kdeconnect_dbus::plugins::{
    parse_sms_message, ConversationSummary, ConversationsProxy, SmsProxy,
};
use zbus::Connection;

/// Heartbeat interval after sync indicator is dismissed (seconds).
/// Keeps the unfold alive so iced can cancel it when the view closes.
const HEARTBEAT_SLEEP_SECS: u64 = 30;

/// State for conversation list subscription.
#[allow(clippy::large_enum_variant)]
enum ConversationListState {
    Init {
        device_id: String,
    },
    /// Emitting cached conversations one at a time before listening for signals
    EmittingCached {
        conn: Connection,
        stream: zbus::MessageStream,
        device_id: String,
        pending_conversations: Vec<ConversationSummary>,
    },
    Listening {
        #[allow(dead_code)]
        conn: Connection,
        stream: zbus::MessageStream,
        device_id: String,
        /// Absolute deadline for how long to wait before dismissing the sync
        /// spinner. After it fires, the subscription continues listening.
        phone_deadline: tokio::time::Instant,
        /// Whether `ConversationSyncComplete` has been emitted (sync spinner dismissed).
        sync_complete_emitted: bool,
    },
    /// Terminal state — stream is finished.
    Done,
}

/// Create a stream that listens for conversation list updates via D-Bus signals.
///
/// This subscription handles incremental conversation loading by:
/// 1. Setting up D-Bus match rules for signals
/// 2. Getting initial cached conversations via activeConversations()
/// 3. Firing requestAllConversationThreads() to trigger phone sync
/// 4. Listening for `conversationCreated`/`conversationUpdated` signals
/// 5. Emitting `Message::ConversationReceived` for each conversation (immediate UI update)
/// 6. Emitting `Message::ConversationSyncComplete` when phone deadline fires (dismisses spinner)
///
/// The subscription runs as long as the SMS view is open. It is cancelled by
/// iced dropping it when `conversation_list_subscription_active` becomes false.
pub fn conversation_list_subscription(
    device_id: String,
) -> impl futures_util::Stream<Item = Message> {
    futures_util::stream::unfold(
        ConversationListState::Init { device_id },
        |state| async move {
            match state {
                ConversationListState::Init { device_id } => {
                    // Connect to D-Bus
                    let conn = match Connection::session().await {
                        Ok(c) => c,
                        Err(e) => {
                            tracing::error!(
                                "Failed to connect to D-Bus for conversation list: {}",
                                e
                            );
                            tokio::time::sleep(std::time::Duration::from_secs(RETRY_DELAY_SECS))
                                .await;
                            return Some((
                                Message::SmsError(format!("D-Bus connection failed: {}", e)),
                                ConversationListState::Init { device_id },
                            ));
                        }
                    };

                    // Add match rules for conversation signals
                    let dbus_proxy = match zbus::fdo::DBusProxy::new(&conn).await {
                        Ok(p) => p,
                        Err(e) => {
                            tracing::error!("Failed to create DBus proxy: {}", e);
                            return Some((
                                Message::SmsError(format!("D-Bus proxy failed: {}", e)),
                                ConversationListState::Init { device_id },
                            ));
                        }
                    };

                    // Subscribe to conversationCreated signals
                    let created_rule = zbus::MatchRule::builder()
                        .msg_type(zbus::message::Type::Signal)
                        .interface("org.kde.kdeconnect.device.conversations")
                        .and_then(|b| b.member("conversationCreated"))
                        .map(|b| b.build());

                    if let Ok(rule) = created_rule {
                        if let Err(e) = dbus_proxy.add_match_rule(rule).await {
                            tracing::warn!(
                                "Failed to add conversationCreated match rule: {}",
                                e
                            );
                        } else {
                            tracing::debug!("Added match rule for conversationCreated signals");
                        }
                    }

                    // Subscribe to conversationUpdated signals
                    let updated_rule = zbus::MatchRule::builder()
                        .msg_type(zbus::message::Type::Signal)
                        .interface("org.kde.kdeconnect.device.conversations")
                        .and_then(|b| b.member("conversationUpdated"))
                        .map(|b| b.build());

                    if let Ok(rule) = updated_rule {
                        if let Err(e) = dbus_proxy.add_match_rule(rule).await {
                            tracing::warn!(
                                "Failed to add conversationUpdated match rule: {}",
                                e
                            );
                        } else {
                            tracing::debug!("Added match rule for conversationUpdated signals");
                        }
                    }

                    // Subscribe to conversationLoaded signals
                    let loaded_rule = zbus::MatchRule::builder()
                        .msg_type(zbus::message::Type::Signal)
                        .interface("org.kde.kdeconnect.device.conversations")
                        .and_then(|b| b.member("conversationLoaded"))
                        .map(|b| b.build());

                    if let Ok(rule) = loaded_rule {
                        if let Err(e) = dbus_proxy.add_match_rule(rule).await {
                            tracing::warn!(
                                "Failed to add conversationLoaded match rule: {}",
                                e
                            );
                        } else {
                            tracing::debug!("Added match rule for conversationLoaded signals");
                        }
                    }

                    // Create message stream BEFORE firing request
                    let stream = zbus::MessageStream::from(&conn);

                    // Build conversations proxy for the device
                    let device_path = format!(
                        "{}/devices/{}",
                        kdeconnect_dbus::BASE_PATH,
                        device_id
                    );

                    let conversations_proxy = match ConversationsProxy::builder(&conn)
                        .path(device_path.as_str())
                        .ok()
                        .map(|b| b.build())
                    {
                        Some(fut) => match fut.await {
                            Ok(p) => Some(p),
                            Err(e) => {
                                tracing::warn!("Failed to create conversations proxy: {}", e);
                                None
                            }
                        },
                        None => None,
                    };

                    // Get cached conversations first (for immediate display)
                    let mut initial_conversations: Vec<ConversationSummary> = Vec::new();
                    if let Some(ref proxy) = conversations_proxy {
                        if let Ok(cached) = proxy.active_conversations().await {
                            tracing::info!("Got {} cached conversation values", cached.len());
                            for value in &cached {
                                if let Some(sms_msg) = parse_sms_message(value) {
                                    let has_attachments = !sms_msg.attachments.is_empty();
                                    initial_conversations.push(ConversationSummary {
                                        thread_id: sms_msg.thread_id,
                                        addresses: sms_msg.addresses,
                                        last_message: sms_msg.body,
                                        timestamp: sms_msg.date,
                                        unread: !sms_msg.read,
                                        has_attachments,
                                    });
                                }
                            }
                            // Sort by timestamp (newest first) and deduplicate
                            initial_conversations
                                .sort_by(|a, b| b.timestamp.cmp(&a.timestamp));
                            let mut seen = std::collections::HashSet::new();
                            initial_conversations.retain(|c| seen.insert(c.thread_id));
                            tracing::info!(
                                "Parsed {} unique cached conversations",
                                initial_conversations.len()
                            );
                        }
                    }

                    // Fire TWO requests (mirrors the pattern from conversation message loading):
                    // 1. SMS plugin's requestAllConversations → sends network packet to phone →
                    //    response goes through addMessages() → populates m_conversations and
                    //    emits conversationCreated signals
                    // 2. Conversations interface's requestAllConversationThreads → may read
                    //    from local store → emits signals for cached conversations
                    //
                    // Without the SMS plugin request, the Conversations interface may only
                    // read from an empty local store and emit no signals.
                    let sms_path = format!(
                        "{}/devices/{}/sms",
                        kdeconnect_dbus::BASE_PATH,
                        device_id
                    );

                    match SmsProxy::builder(&conn)
                        .path(sms_path.as_str())
                        .ok()
                        .map(|b| b.build())
                    {
                        Some(fut) => match fut.await {
                            Ok(sms_proxy) => {
                                if let Err(e) = sms_proxy.request_all_conversations().await {
                                    tracing::warn!(
                                        "SMS plugin requestAllConversations failed (non-fatal): {}",
                                        e
                                    );
                                } else {
                                    tracing::debug!(
                                        "SMS plugin requestAllConversations fired for device {} (cache priming)",
                                        device_id
                                    );
                                }
                            }
                            Err(e) => {
                                tracing::warn!(
                                    "Failed to create SMS proxy (non-fatal): {}",
                                    e
                                );
                            }
                        },
                        None => {
                            tracing::warn!("Failed to build SMS proxy path (non-fatal)");
                        }
                    }

                    // Conversations interface request (local store signals)
                    if let Some(ref proxy) = conversations_proxy {
                        tracing::info!(
                            "Firing requestAllConversationThreads for device {}",
                            device_id
                        );
                        if let Err(e) = proxy.request_all_conversation_threads().await {
                            tracing::warn!("Failed to request conversation threads: {}", e);
                        }
                    }

                    let now = tokio::time::Instant::now();

                    // If we have cached data, transition to EmittingCached state
                    if !initial_conversations.is_empty() {
                        tracing::info!(
                            "Emitting {} cached conversations for device {}",
                            initial_conversations.len(),
                            device_id
                        );

                        // Emit the first one and store the rest
                        let first = initial_conversations.remove(0);
                        return Some((
                            Message::ConversationReceived {
                                device_id: device_id.clone(),
                                conversation: first,
                            },
                            ConversationListState::EmittingCached {
                                conn,
                                stream,
                                device_id,
                                pending_conversations: initial_conversations,
                            },
                        ));
                    }

                    // No cached data — use longer phone wait (cold start)
                    let phone_deadline = now
                        + tokio::time::Duration::from_millis(CONVERSATION_LIST_PHONE_WAIT_MS);
                    Some((
                        Message::ConversationSyncStarted {
                            device_id: device_id.clone(),
                        },
                        ConversationListState::Listening {
                            conn,
                            stream,
                            device_id,
                            phone_deadline,
                            sync_complete_emitted: false,
                        },
                    ))
                }
                ConversationListState::EmittingCached {
                    conn,
                    stream,
                    device_id,
                    mut pending_conversations,
                } => {
                    // Emit cached conversations one at a time
                    if !pending_conversations.is_empty() {
                        let conversation = pending_conversations.remove(0);
                        tracing::debug!(
                            "Emitting cached conversation: thread {} ({} remaining)",
                            conversation.thread_id,
                            pending_conversations.len()
                        );
                        return Some((
                            Message::ConversationReceived {
                                device_id: device_id.clone(),
                                conversation,
                            },
                            ConversationListState::EmittingCached {
                                conn,
                                stream,
                                device_id,
                                pending_conversations,
                            },
                        ));
                    }

                    // All cached conversations emitted, transition to listening for signals.
                    // Use shorter phone wait since we have cache (warm start).
                    tracing::debug!(
                        "Finished emitting cached conversations, now listening for signals for device {}",
                        device_id
                    );
                    let now = tokio::time::Instant::now();
                    let phone_deadline = now
                        + tokio::time::Duration::from_secs(CONVERSATION_TIMEOUT_CACHED_SECS);
                    Some((
                        Message::ConversationSyncStarted {
                            device_id: device_id.clone(),
                        },
                        ConversationListState::Listening {
                            conn,
                            stream,
                            device_id,
                            phone_deadline,
                            sync_complete_emitted: false,
                        },
                    ))
                }
                ConversationListState::Listening {
                    conn,
                    mut stream,
                    device_id,
                    phone_deadline,
                    mut sync_complete_emitted,
                } => {
                    loop {
                        let now = tokio::time::Instant::now();

                        // Phone deadline: dismiss the sync indicator once, then
                        // continue listening with a long heartbeat sleep.
                        let sleep_duration = if sync_complete_emitted {
                            tokio::time::Duration::from_secs(HEARTBEAT_SLEEP_SECS)
                        } else if now >= phone_deadline {
                            tracing::info!(
                                "Conversation list sync: phone deadline reached for device {}, \
                                 dismissing spinner (subscription continues)",
                                device_id
                            );
                            sync_complete_emitted = true;
                            return Some((
                                Message::ConversationSyncComplete {
                                    device_id: device_id.clone(),
                                },
                                ConversationListState::Listening {
                                    conn,
                                    stream,
                                    device_id,
                                    phone_deadline,
                                    sync_complete_emitted,
                                },
                            ));
                        } else {
                            phone_deadline.saturating_duration_since(now)
                        };

                        tokio::select! {
                            biased;

                            // Wait for D-Bus signals
                            msg_option = stream.next() => {
                                match msg_option {
                                    Some(Ok(msg)) => {
                                        if msg.header().message_type() == zbus::message::Type::Signal {
                                            if let (Some(interface), Some(member)) =
                                                (msg.header().interface(), msg.header().member())
                                            {
                                                let iface_str = interface.as_str();
                                                let member_str = member.as_str();

                                                // Check if this signal is for our device
                                                let is_our_device = msg.header().path()
                                                    .map(|p| p.as_str().contains(&device_id))
                                                    .unwrap_or(false);

                                                if !is_our_device {
                                                    continue;
                                                }

                                                // Handle conversationCreated signals
                                                if iface_str == "org.kde.kdeconnect.device.conversations"
                                                    && member_str == "conversationCreated"
                                                {
                                                    let body = msg.body();
                                                    if let Ok(value) = body.deserialize::<zbus::zvariant::OwnedValue>() {
                                                        if let Some(sms_msg) = parse_sms_message(&value) {
                                                            let has_attachments = !sms_msg.attachments.is_empty();
                                                            let conversation = ConversationSummary {
                                                                thread_id: sms_msg.thread_id,
                                                                addresses: sms_msg.addresses,
                                                                last_message: sms_msg.body,
                                                                timestamp: sms_msg.date,
                                                                unread: !sms_msg.read,
                                                                has_attachments,
                                                            };
                                                            tracing::debug!(
                                                                "conversationCreated: thread {} for device {}",
                                                                conversation.thread_id,
                                                                device_id
                                                            );
                                                            return Some((
                                                                Message::ConversationReceived {
                                                                    device_id: device_id.clone(),
                                                                    conversation,
                                                                },
                                                                ConversationListState::Listening {
                                                                    conn,
                                                                    stream,
                                                                    device_id,
                                                                    phone_deadline,
                                                                    sync_complete_emitted,
                                                                },
                                                            ));
                                                        }
                                                    }
                                                }

                                                // Handle conversationUpdated signals
                                                if iface_str == "org.kde.kdeconnect.device.conversations"
                                                    && member_str == "conversationUpdated"
                                                {
                                                    let body = msg.body();
                                                    if let Ok(value) = body.deserialize::<zbus::zvariant::OwnedValue>() {
                                                        if let Some(sms_msg) = parse_sms_message(&value) {
                                                            let has_attachments = !sms_msg.attachments.is_empty();
                                                            let conversation = ConversationSummary {
                                                                thread_id: sms_msg.thread_id,
                                                                addresses: sms_msg.addresses,
                                                                last_message: sms_msg.body,
                                                                timestamp: sms_msg.date,
                                                                unread: !sms_msg.read,
                                                                has_attachments,
                                                            };
                                                            tracing::debug!(
                                                                "conversationUpdated: thread {} for device {}",
                                                                conversation.thread_id,
                                                                device_id
                                                            );
                                                            return Some((
                                                                Message::ConversationReceived {
                                                                    device_id: device_id.clone(),
                                                                    conversation,
                                                                },
                                                                ConversationListState::Listening {
                                                                    conn,
                                                                    stream,
                                                                    device_id,
                                                                    phone_deadline,
                                                                    sync_complete_emitted,
                                                                },
                                                            ));
                                                        }
                                                    }
                                                }

                                                // Handle conversationLoaded signals (progress marker)
                                                if iface_str == "org.kde.kdeconnect.device.conversations"
                                                    && member_str == "conversationLoaded"
                                                {
                                                    tracing::debug!(
                                                        "conversationLoaded signal for device {}",
                                                        device_id
                                                    );
                                                    // Continue listening - more signals may come
                                                }
                                            }
                                        }
                                    }
                                    Some(Err(e)) => {
                                        tracing::warn!("D-Bus stream error: {}", e);
                                    }
                                    None => {
                                        // Stream ended (D-Bus connection dropped)
                                        tracing::warn!(
                                            "D-Bus message stream ended for device {}",
                                            device_id
                                        );
                                        if !sync_complete_emitted {
                                            return Some((
                                                Message::ConversationSyncComplete { device_id },
                                                ConversationListState::Done,
                                            ));
                                        }
                                        return None;
                                    }
                                }
                            }

                            // Sleep until phone deadline or heartbeat
                            _ = tokio::time::sleep(sleep_duration) => {
                                // Loop back — deadline check at top will handle expiry
                            }
                        }
                    }
                }
                ConversationListState::Done => None,
            }
        },
    )
}
