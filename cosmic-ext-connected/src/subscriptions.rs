//! D-Bus signal subscriptions for real-time updates from KDE Connect.

use crate::app::Message;
use crate::constants::dbus::RETRY_DELAY_SECS;
use crate::constants::sms::{MESSAGE_SUBSCRIPTION_TIMEOUT_SECS, PHONE_RESPONSE_TIMEOUT_MS};
use crate::notifications::{
    should_show_call_notification, should_show_file_notification, should_show_sms_notification,
};
use futures_util::StreamExt;
use kdeconnect_dbus::plugins::{parse_sms_message, MessageType};
use kdeconnect_dbus::DeviceProxy;
use zbus::Connection;

/// State for D-Bus signal subscription.
#[allow(clippy::large_enum_variant)]
enum DbusSubscriptionState {
    Init,
    Listening {
        #[allow(dead_code)]
        conn: Connection,
        stream: zbus::MessageStream,
        /// Last file URL and time for deduplication of rapid signals
        #[allow(dead_code)]
        last_file: Option<(String, std::time::Instant)>,
    },
}

/// Create a stream that listens for D-Bus signals from KDE Connect.
pub fn dbus_signal_subscription() -> impl futures_util::Stream<Item = Message> {
    futures_util::stream::unfold(DbusSubscriptionState::Init, |state| async move {
        match state {
            DbusSubscriptionState::Init => {
                // Connect to D-Bus
                let conn = match Connection::session().await {
                    Ok(c) => c,
                    Err(e) => {
                        tracing::error!("Failed to connect to D-Bus for signals: {}", e);
                        tokio::time::sleep(std::time::Duration::from_secs(RETRY_DELAY_SECS)).await;
                        return Some((
                            Message::Error("D-Bus connection failed".to_string()),
                            DbusSubscriptionState::Init,
                        ));
                    }
                };

                // Add match rule to receive KDE Connect signals
                let dbus_proxy = match zbus::fdo::DBusProxy::new(&conn).await {
                    Ok(p) => p,
                    Err(e) => {
                        tracing::error!("Failed to create DBus proxy: {}", e);
                        return Some((
                            Message::Error("D-Bus proxy failed".to_string()),
                            DbusSubscriptionState::Init,
                        ));
                    }
                };

                // Subscribe to all signals from KDE Connect daemon
                if let Ok(rule) = zbus::MatchRule::builder()
                    .msg_type(zbus::message::Type::Signal)
                    .sender("org.kde.kdeconnect.daemon")
                    .map(|b| b.build())
                {
                    if let Err(e) = dbus_proxy.add_match_rule(rule).await {
                        tracing::warn!("Failed to add match rule: {}", e);
                    } else {
                        tracing::debug!("Added match rule for kdeconnect daemon signals");
                    }
                }

                // Also subscribe to property changes (for battery, pairing state, etc.)
                if let Ok(rule) = zbus::MatchRule::builder()
                    .msg_type(zbus::message::Type::Signal)
                    .interface("org.freedesktop.DBus.Properties")
                    .map(|b| b.build())
                {
                    if let Err(e) = dbus_proxy.add_match_rule(rule).await {
                        tracing::warn!("Failed to add properties match rule: {}", e);
                    } else {
                        tracing::debug!("Added match rule for property change signals");
                    }
                }

                // Subscribe to share plugin signals for file notifications
                if let Ok(rule) = zbus::MatchRule::builder()
                    .msg_type(zbus::message::Type::Signal)
                    .interface("org.kde.kdeconnect.device.share")
                    .map(|b| b.build())
                {
                    if let Err(e) = dbus_proxy.add_match_rule(rule).await {
                        tracing::warn!("Failed to add share match rule: {}", e);
                    } else {
                        tracing::debug!("Added match rule for share signals");
                    }
                } else {
                    tracing::warn!("Failed to build share match rule");
                }

                tracing::debug!("D-Bus signal subscription started");

                // Create message stream
                let stream = zbus::MessageStream::from(&conn);

                Some((
                    Message::DbusSignalReceived,
                    DbusSubscriptionState::Listening {
                        conn,
                        stream,
                        last_file: None,
                    },
                ))
            }
            DbusSubscriptionState::Listening {
                conn,
                mut stream,
                last_file,
            } => {
                // Wait for relevant signals - be selective to avoid excessive refreshes
                loop {
                    match stream.next().await {
                        Some(Ok(msg)) => {
                            if msg.header().message_type() == zbus::message::Type::Signal {
                                if let (Some(interface), Some(member)) =
                                    (msg.header().interface(), msg.header().member())
                                {
                                    let iface_str = interface.as_str();
                                    let member_str = member.as_str();

                                    // Handle share signals for file notifications
                                    if iface_str == "org.kde.kdeconnect.device.share"
                                        && member_str == "shareReceived"
                                    {
                                        // Extract device ID from path
                                        if let Some(path) = msg.header().path() {
                                            let path_str = path.as_str();
                                            if let Some(rest) = path_str
                                                .strip_prefix("/modules/kdeconnect/devices/")
                                            {
                                                let device_id = rest
                                                    .split('/')
                                                    .next()
                                                    .unwrap_or(rest)
                                                    .to_string();

                                                // Parse the signal body
                                                let body = msg.body();
                                                if let Ok((file_url,)) =
                                                    body.deserialize::<(String,)>()
                                                {
                                                    // Cross-process deduplication via file lock
                                                    // KDE Connect sends 3 duplicate signals per file transfer
                                                    // and COSMIC spawns multiple applet processes
                                                    if !should_show_file_notification(&file_url) {
                                                        continue;
                                                    }

                                                    let file_name = file_url
                                                        .strip_prefix("file://")
                                                        .unwrap_or(&file_url)
                                                        .rsplit('/')
                                                        .next()
                                                        .unwrap_or("file")
                                                        .to_string();

                                                    return Some((
                                                        Message::FileReceived {
                                                            device_name: device_id,
                                                            file_url,
                                                            file_name,
                                                        },
                                                        DbusSubscriptionState::Listening {
                                                            conn,
                                                            stream,
                                                            last_file,
                                                        },
                                                    ));
                                                }
                                            }
                                        }
                                    }

                                    // Only trigger refresh on specific device-related signals
                                    let is_relevant = match iface_str {
                                        // Daemon signals for device discovery
                                        "org.kde.kdeconnect.daemon" => matches!(
                                            member_str,
                                            "deviceAdded"
                                                | "deviceRemoved"
                                                | "deviceVisibilityChanged"
                                                | "announcedNameChanged"
                                        ),
                                        // Device signals for pairing state
                                        "org.kde.kdeconnect.device" => matches!(
                                            member_str,
                                            "reachableChanged"
                                                | "trustedChanged"
                                                | "pairingRequest"
                                                | "hasPairingRequestsChanged"
                                        ),
                                        // Battery and notification plugin signals
                                        "org.kde.kdeconnect.device.battery" => true,
                                        "org.kde.kdeconnect.device.notifications" => true,
                                        // Property changes for any kdeconnect interface
                                        "org.freedesktop.DBus.Properties" => {
                                            member_str == "PropertiesChanged"
                                        }
                                        _ => false,
                                    };

                                    if is_relevant {
                                        tracing::debug!("D-Bus signal: {}.{}", interface, member);
                                        return Some((
                                            Message::DbusSignalReceived,
                                            DbusSubscriptionState::Listening {
                                                conn,
                                                stream,
                                                last_file,
                                            },
                                        ));
                                    }
                                }
                            }
                        }
                        Some(Err(e)) => {
                            tracing::warn!("D-Bus stream error: {}", e);
                        }
                        None => {
                            tracing::warn!("D-Bus stream ended, reconnecting...");
                            return Some((
                                Message::DbusSignalReceived,
                                DbusSubscriptionState::Init,
                            ));
                        }
                    }
                }
            }
        }
    })
}

/// State for SMS notification subscription.
#[allow(clippy::large_enum_variant)]
enum SmsSubscriptionState {
    Init,
    Listening {
        #[allow(dead_code)]
        conn: Connection,
        stream: zbus::MessageStream,
    },
}

/// Create a stream that listens for incoming SMS messages via D-Bus signals.
pub fn sms_notification_subscription() -> impl futures_util::Stream<Item = Message> {
    futures_util::stream::unfold(SmsSubscriptionState::Init, |state| async move {
        match state {
            SmsSubscriptionState::Init => {
                // Connect to D-Bus
                let conn = match Connection::session().await {
                    Ok(c) => c,
                    Err(e) => {
                        tracing::error!("Failed to connect to D-Bus for SMS signals: {}", e);
                        tokio::time::sleep(std::time::Duration::from_secs(RETRY_DELAY_SECS)).await;
                        return Some((
                            Message::Error("D-Bus connection failed for SMS".to_string()),
                            SmsSubscriptionState::Init,
                        ));
                    }
                };

                // Add match rule for conversationUpdated signals
                let dbus_proxy = match zbus::fdo::DBusProxy::new(&conn).await {
                    Ok(p) => p,
                    Err(e) => {
                        tracing::error!("Failed to create DBus proxy for SMS: {}", e);
                        return Some((
                            Message::Error("D-Bus proxy failed for SMS".to_string()),
                            SmsSubscriptionState::Init,
                        ));
                    }
                };

                // Subscribe to conversation signals from KDE Connect
                // Note: interface() returns Result, so we chain with and_then for member()
                let rule_result = zbus::MatchRule::builder()
                    .msg_type(zbus::message::Type::Signal)
                    .interface("org.kde.kdeconnect.device.conversations")
                    .and_then(|b| b.member("conversationUpdated"))
                    .map(|b| b.build());

                if let Ok(rule) = rule_result {
                    if let Err(e) = dbus_proxy.add_match_rule(rule).await {
                        tracing::warn!("Failed to add SMS match rule: {}", e);
                    } else {
                        tracing::debug!("Added match rule for SMS conversationUpdated signals");
                    }
                }

                tracing::debug!("SMS notification subscription started");

                // Create message stream
                let stream = zbus::MessageStream::from(&conn);

                // Don't emit a message on init, just move to listening state
                Some((
                    Message::RefreshDevices, // Trigger a refresh to pick up any pending state
                    SmsSubscriptionState::Listening { conn, stream },
                ))
            }
            SmsSubscriptionState::Listening { conn, mut stream } => {
                // Wait for conversationUpdated signals
                loop {
                    match stream.next().await {
                        Some(Ok(msg)) => {
                            if msg.header().message_type() == zbus::message::Type::Signal {
                                if let (Some(interface), Some(member)) =
                                    (msg.header().interface(), msg.header().member())
                                {
                                    let iface_str = interface.as_str();
                                    let member_str = member.as_str();

                                    // Only process conversationUpdated signals
                                    if iface_str == "org.kde.kdeconnect.device.conversations"
                                        && member_str == "conversationUpdated"
                                    {
                                        // Extract device ID from the path
                                        // Path format: /modules/kdeconnect/devices/{device_id}
                                        if let Some(path) = msg.header().path() {
                                            let path_str = path.as_str();
                                            if let Some(device_id) = path_str
                                                .strip_prefix("/modules/kdeconnect/devices/")
                                            {
                                                // Extract the device_id (may contain more path components)
                                                let device_id = device_id
                                                    .split('/')
                                                    .next()
                                                    .unwrap_or(device_id);

                                                // Parse the message body to get SMS data
                                                let body = msg.body();
                                                if let Ok(value) =
                                                    body.deserialize::<zbus::zvariant::OwnedValue>()
                                                {
                                                    if let Some(sms_msg) = parse_sms_message(&value)
                                                    {
                                                        // Only notify for received messages
                                                        // Standard Android SMS semantics: Inbox (1) = received from others
                                                        if sms_msg.message_type
                                                            == MessageType::Inbox
                                                        {
                                                            // Cross-process deduplication:
                                                            // COSMIC spawns multiple applet processes,
                                                            // so use file-based locking to ensure only one shows the notification
                                                            if !should_show_sms_notification(
                                                                sms_msg.thread_id,
                                                                sms_msg.date,
                                                            ) {
                                                                continue;
                                                            }

                                                            tracing::debug!(
                                                                "SMS received from {} on device {}: {}",
                                                                sms_msg.primary_address(),
                                                                device_id,
                                                                &sms_msg.body[..sms_msg.body.len().min(30)]
                                                            );
                                                            return Some((
                                                                Message::SmsNotificationReceived(
                                                                    device_id.to_string(),
                                                                    sms_msg,
                                                                ),
                                                                SmsSubscriptionState::Listening {
                                                                    conn,
                                                                    stream,
                                                                },
                                                            ));
                                                        }
                                                    }
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                        }
                        Some(Err(e)) => {
                            tracing::warn!("D-Bus SMS stream error: {}", e);
                        }
                        None => {
                            tracing::warn!("D-Bus SMS stream ended, reconnecting...");
                            return Some((Message::RefreshDevices, SmsSubscriptionState::Init));
                        }
                    }
                }
            }
        }
    })
}

/// State for call notification subscription.
#[allow(clippy::large_enum_variant)]
enum CallSubscriptionState {
    Init,
    Listening {
        conn: Connection,
        stream: zbus::MessageStream,
    },
}

/// Create a stream that listens for incoming/missed calls via D-Bus signals.
pub fn call_notification_subscription() -> impl futures_util::Stream<Item = Message> {
    futures_util::stream::unfold(CallSubscriptionState::Init, |state| async move {
        match state {
            CallSubscriptionState::Init => {
                // Connect to D-Bus
                let conn = match Connection::session().await {
                    Ok(c) => c,
                    Err(e) => {
                        tracing::error!("Failed to connect to D-Bus for call signals: {}", e);
                        tokio::time::sleep(std::time::Duration::from_secs(RETRY_DELAY_SECS)).await;
                        return Some((
                            Message::Error("D-Bus connection failed for calls".to_string()),
                            CallSubscriptionState::Init,
                        ));
                    }
                };

                // Create DBus proxy for adding match rules
                let dbus_proxy = match zbus::fdo::DBusProxy::new(&conn).await {
                    Ok(p) => p,
                    Err(e) => {
                        tracing::error!("Failed to create DBus proxy for calls: {}", e);
                        return Some((
                            Message::Error("D-Bus proxy failed for calls".to_string()),
                            CallSubscriptionState::Init,
                        ));
                    }
                };

                // Subscribe to telephony callReceived signals
                let rule_result = zbus::MatchRule::builder()
                    .msg_type(zbus::message::Type::Signal)
                    .interface("org.kde.kdeconnect.device.telephony")
                    .and_then(|b| b.member("callReceived"))
                    .map(|b| b.build());

                if let Ok(rule) = rule_result {
                    if let Err(e) = dbus_proxy.add_match_rule(rule).await {
                        tracing::warn!("Failed to add call match rule: {}", e);
                    } else {
                        tracing::debug!("Added match rule for telephony callReceived signals");
                    }
                }

                tracing::debug!("Call notification subscription started");

                // Create message stream
                let stream = zbus::MessageStream::from(&conn);

                Some((
                    Message::RefreshDevices,
                    CallSubscriptionState::Listening { conn, stream },
                ))
            }
            CallSubscriptionState::Listening { conn, mut stream } => {
                // Wait for callReceived signals
                loop {
                    match stream.next().await {
                        Some(Ok(msg)) => {
                            if msg.header().message_type() == zbus::message::Type::Signal {
                                if let (Some(interface), Some(member)) =
                                    (msg.header().interface(), msg.header().member())
                                {
                                    let iface_str = interface.as_str();
                                    let member_str = member.as_str();

                                    // Only process callReceived signals from telephony
                                    if iface_str == "org.kde.kdeconnect.device.telephony"
                                        && member_str == "callReceived"
                                    {
                                        // Extract device ID from the path
                                        // Path format: /modules/kdeconnect/devices/{device_id}/telephony
                                        if let Some(path) = msg.header().path() {
                                            let path_str = path.as_str();
                                            if let Some(rest) = path_str
                                                .strip_prefix("/modules/kdeconnect/devices/")
                                            {
                                                let device_id =
                                                    rest.split('/').next().unwrap_or(rest);

                                                // Parse the signal arguments: (event, phone_number, contact_name)
                                                let body = msg.body();
                                                if let Ok((event, phone_number, contact_name)) =
                                                    body.deserialize::<(String, String, String)>()
                                                {
                                                    // Cross-process deduplication:
                                                    // COSMIC spawns multiple applet processes,
                                                    // so use file-based locking to ensure only one shows the notification
                                                    if !should_show_call_notification(
                                                        &event,
                                                        &phone_number,
                                                    ) {
                                                        continue;
                                                    }

                                                    tracing::debug!(
                                                        "Call signal: {} from {} ({}) on device {}",
                                                        event,
                                                        contact_name,
                                                        phone_number,
                                                        device_id
                                                    );

                                                    // Get device name from D-Bus
                                                    let device_name =
                                                        match DeviceProxy::builder(&conn)
                                                            .path(format!(
                                                                "{}/devices/{}",
                                                                kdeconnect_dbus::BASE_PATH,
                                                                device_id
                                                            ))
                                                            .ok()
                                                            .map(|b| b.build())
                                                        {
                                                            Some(fut) => match fut.await {
                                                                Ok(proxy) => proxy
                                                                    .name()
                                                                    .await
                                                                    .unwrap_or_else(|_| {
                                                                        device_id.to_string()
                                                                    }),
                                                                Err(_) => device_id.to_string(),
                                                            },
                                                            None => device_id.to_string(),
                                                        };

                                                    return Some((
                                                        Message::CallNotification {
                                                            device_name,
                                                            event,
                                                            phone_number,
                                                            contact_name,
                                                        },
                                                        CallSubscriptionState::Listening {
                                                            conn,
                                                            stream,
                                                        },
                                                    ));
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                        }
                        Some(Err(e)) => {
                            tracing::warn!("D-Bus call stream error: {}", e);
                        }
                        None => {
                            tracing::warn!("D-Bus call stream ended, reconnecting...");
                            return Some((Message::RefreshDevices, CallSubscriptionState::Init));
                        }
                    }
                }
            }
        }
    })
}

/// State for conversation message subscription (incremental message loading).
#[allow(clippy::large_enum_variant)]
enum ConversationMessageState {
    Init {
        thread_id: i64,
        device_id: String,
        messages_per_page: u32,
    },
    Listening {
        #[allow(dead_code)]
        conn: Connection,
        stream: zbus::MessageStream,
        thread_id: i64,
        device_id: String,
        #[allow(dead_code)]
        messages_per_page: u32,
        /// When we started listening (for hard timeout safety net)
        start_time: tokio::time::Instant,
        /// Set when conversationLoaded arrives; switches to deadline-based timeout
        /// to wait for phone response data (local store may be sparse after reboot)
        local_store_done: bool,
        /// Total message count from conversationLoaded signal (for final emission)
        total_message_count: Option<u64>,
        /// Deadline for phone response activity timeout. Set when conversationLoaded
        /// arrives, extended when a matching message is received. Must be in the state
        /// struct because each `unfold` yield exits and re-enters the function.
        phone_deadline: Option<tokio::time::Instant>,
    },
    /// Terminal state - subscription is complete
    Done,
}

/// Create a stream that listens for conversation messages during loading.
///
/// This subscription handles incremental message loading by:
/// 1. Setting up D-Bus match rules for signals
/// 2. Firing the request_conversation D-Bus call (AFTER rules are set up)
/// 3. Listening for `conversationUpdated` signals (individual messages)
/// 4. Emitting `ConversationLoadComplete` when `conversationLoaded` signal arrives
///
/// The request is fired from within the subscription to avoid race conditions
/// where signals arrive before we're ready to receive them.
pub fn conversation_message_subscription(
    thread_id: i64,
    device_id: String,
    messages_per_page: u32,
) -> impl futures_util::Stream<Item = Message> {
    futures_util::stream::unfold(
        ConversationMessageState::Init {
            thread_id,
            device_id,
            messages_per_page,
        },
        |state| async move {
            match state {
                ConversationMessageState::Init {
                    thread_id,
                    device_id,
                    messages_per_page,
                } => {
                    // Connect to D-Bus
                    let conn = match Connection::session().await {
                        Ok(c) => c,
                        Err(e) => {
                            tracing::error!(
                                "Failed to connect to D-Bus for conversation messages: {}",
                                e
                            );
                            tokio::time::sleep(std::time::Duration::from_secs(RETRY_DELAY_SECS))
                                .await;
                            return Some((
                                Message::SmsError("D-Bus connection failed for conversation".to_string()),
                                ConversationMessageState::Init {
                                    thread_id,
                                    device_id,
                                    messages_per_page,
                                },
                            ));
                        }
                    };

                    // Add match rule for conversationUpdated signals
                    let dbus_proxy = match zbus::fdo::DBusProxy::new(&conn).await {
                        Ok(p) => p,
                        Err(e) => {
                            tracing::error!("Failed to create DBus proxy for conversation: {}", e);
                            return Some((
                                Message::SmsError("D-Bus proxy failed for conversation".to_string()),
                                ConversationMessageState::Init {
                                    thread_id,
                                    device_id,
                                    messages_per_page,
                                },
                            ));
                        }
                    };

                    // Subscribe to conversationUpdated signals (individual messages)
                    let updated_rule = zbus::MatchRule::builder()
                        .msg_type(zbus::message::Type::Signal)
                        .interface("org.kde.kdeconnect.device.conversations")
                        .and_then(|b| b.member("conversationUpdated"))
                        .map(|b| b.build());

                    if let Ok(rule) = updated_rule {
                        if let Err(e) = dbus_proxy.add_match_rule(rule).await {
                            tracing::warn!("Failed to add conversationUpdated match rule: {}", e);
                        } else {
                            tracing::debug!(
                                "Added match rule for conversation {} message signals",
                                thread_id
                            );
                        }
                    }

                    // Subscribe to conversationLoaded signals (completion marker)
                    let loaded_rule = zbus::MatchRule::builder()
                        .msg_type(zbus::message::Type::Signal)
                        .interface("org.kde.kdeconnect.device.conversations")
                        .and_then(|b| b.member("conversationLoaded"))
                        .map(|b| b.build());

                    if let Ok(rule) = loaded_rule {
                        if let Err(e) = dbus_proxy.add_match_rule(rule).await {
                            tracing::warn!("Failed to add conversationLoaded match rule: {}", e);
                        } else {
                            tracing::debug!(
                                "Added match rule for conversation {} loaded signal",
                                thread_id
                            );
                        }
                    }

                    // Create message stream BEFORE firing request
                    let stream = zbus::MessageStream::from(&conn);

                    // NOW fire D-Bus requests - after match rules are set up
                    // This ensures we don't miss any signals
                    let device_path = format!(
                        "{}/devices/{}",
                        kdeconnect_dbus::BASE_PATH,
                        device_id
                    );

                    // Fire TWO requests:
                    // 1. SMS plugin's requestConversation → sends network packet to phone →
                    //    response goes through addMessages() → populates m_conversations
                    //    (required for replyToConversation to look up addresses)
                    // 2. Conversations interface's requestConversation → reads from local
                    //    store via RequestConversationWorker → emits per-message signals
                    //    (required for our subscription to receive all messages)
                    //
                    // The SMS plugin request primes the daemon cache; the Conversations
                    // request provides the per-message signals for UI display.
                    let sms_path = format!(
                        "{}/devices/{}/sms",
                        kdeconnect_dbus::BASE_PATH,
                        device_id
                    );

                    // Fire SMS plugin request first (cache priming, async - phone responds later)
                    match kdeconnect_dbus::plugins::SmsProxy::builder(&conn)
                        .path(sms_path.as_str())
                        .ok()
                        .map(|b| b.build())
                    {
                        Some(fut) => match fut.await {
                            Ok(sms_proxy) => {
                                if let Err(e) = sms_proxy
                                    .request_conversation(thread_id, 0, messages_per_page as i64)
                                    .await
                                {
                                    tracing::warn!("SMS plugin request_conversation failed (non-fatal): {}", e);
                                } else {
                                    tracing::debug!(
                                        "SMS plugin request_conversation fired for thread {} (cache priming)",
                                        thread_id
                                    );
                                }
                            }
                            Err(e) => {
                                tracing::warn!("Failed to create SMS proxy (non-fatal): {}", e);
                            }
                        },
                        None => {
                            tracing::warn!("Failed to build SMS proxy path (non-fatal)");
                        }
                    }

                    // Fire Conversations interface request (provides per-message signals)
                    match kdeconnect_dbus::plugins::ConversationsProxy::builder(&conn)
                        .path(device_path.as_str())
                        .ok()
                        .map(|b| b.build())
                    {
                        Some(fut) => match fut.await {
                            Ok(conversations_proxy) => {
                                tracing::debug!(
                                    "Firing request_conversation for thread {} (messages 0-{})",
                                    thread_id,
                                    messages_per_page
                                );
                                if let Err(e) = conversations_proxy
                                    .request_conversation(thread_id, 0, messages_per_page as i32)
                                    .await
                                {
                                    tracing::warn!("Failed to request conversation: {}", e);
                                    return Some((
                                        Message::SmsError(format!(
                                            "Failed to request conversation: {}",
                                            e
                                        )),
                                        ConversationMessageState::Init {
                                            thread_id,
                                            device_id,
                                            messages_per_page,
                                        },
                                    ));
                                }
                                tracing::info!(
                                    "Conversation {} request sent, listening for signals",
                                    thread_id
                                );
                            }
                            Err(e) => {
                                tracing::warn!("Failed to create conversations proxy: {}", e);
                                return Some((
                                    Message::SmsError(format!(
                                        "Failed to create conversations proxy: {}",
                                        e
                                    )),
                                    ConversationMessageState::Init {
                                        thread_id,
                                        device_id,
                                        messages_per_page,
                                    },
                                ));
                            }
                        },
                        None => {
                            return Some((
                                Message::SmsError(
                                    "Failed to build conversations proxy path".to_string(),
                                ),
                                ConversationMessageState::Init {
                                    thread_id,
                                    device_id,
                                    messages_per_page,
                                },
                            ));
                        }
                    }

                    // Move to listening state, emit started message
                    Some((
                        Message::ConversationLoadStarted { thread_id },
                        ConversationMessageState::Listening {
                            conn,
                            stream,
                            thread_id,
                            device_id,
                            messages_per_page,
                            start_time: tokio::time::Instant::now(),
                            local_store_done: false,
                            total_message_count: None,
                            phone_deadline: None,
                        },
                    ))
                }
                ConversationMessageState::Listening {
                    conn,
                    mut stream,
                    thread_id,
                    device_id,
                    messages_per_page,
                    start_time,
                    mut local_store_done,
                    mut total_message_count,
                    mut phone_deadline,
                } => {
                    // Two-phase timeout strategy:
                    //
                    // Phase 1 (before conversationLoaded): Wait for the hard timeout.
                    //   The local store read emits conversationUpdated per message, then
                    //   conversationLoaded when done. No activity timeout needed here.
                    //
                    // Phase 2 (after conversationLoaded): Keep listening with a deadline-
                    //   based activity timeout for phone response data. The local store may
                    //   be empty/sparse after a reboot, so the phone response (via SMS
                    //   plugin → addMessages) provides the actual messages. The deadline
                    //   resets only when a MATCHING signal arrives (message for our thread),
                    //   not on unrelated D-Bus traffic.
                    //
                    // Hard timeout: absolute safety net for both phases.
                    let hard_timeout = std::time::Duration::from_secs(MESSAGE_SUBSCRIPTION_TIMEOUT_SECS);
                    let phone_timeout = std::time::Duration::from_millis(PHONE_RESPONSE_TIMEOUT_MS);
                    let hard_deadline = start_time + hard_timeout;

                    loop {
                        let now = tokio::time::Instant::now();

                        // Hard timeout check (absolute)
                        if now >= hard_deadline {
                            tracing::info!(
                                "Subscription: hard timeout reached for thread {} after {:?}",
                                thread_id,
                                start_time.elapsed()
                            );
                            return Some((
                                Message::ConversationLoadComplete {
                                    thread_id,
                                    total_count: total_message_count.unwrap_or(0),
                                },
                                ConversationMessageState::Done,
                            ));
                        }

                        // Compute wait duration based on phase:
                        // Phase 1: wait until hard deadline
                        // Phase 2: wait until phone deadline (capped by hard deadline)
                        let effective_deadline = if let Some(pd) = phone_deadline {
                            pd.min(hard_deadline)
                        } else {
                            hard_deadline
                        };

                        // Check if phone deadline already passed
                        if local_store_done {
                            if let Some(pd) = phone_deadline {
                                if now >= pd {
                                    tracing::info!(
                                        "Subscription: phone response timeout for thread {} \
                                         (no matching signals for {:?} after conversationLoaded)",
                                        thread_id,
                                        phone_timeout
                                    );
                                    return Some((
                                        Message::ConversationLoadComplete {
                                            thread_id,
                                            total_count: total_message_count.unwrap_or(0),
                                        },
                                        ConversationMessageState::Done,
                                    ));
                                }
                            }
                        }

                        let wait_duration = effective_deadline.saturating_duration_since(now);

                        tokio::select! {
                            biased;

                            // Priority: D-Bus signals
                            msg_result = stream.next() => {
                                match msg_result {
                                    Some(Ok(msg)) => {
                                        if msg.header().message_type() == zbus::message::Type::Signal {
                                            if let (Some(interface), Some(member)) =
                                                (msg.header().interface(), msg.header().member())
                                            {
                                                let iface_str = interface.as_str();
                                                let member_str = member.as_str();

                                                // Handle conversationUpdated signals (individual messages)
                                                if iface_str == "org.kde.kdeconnect.device.conversations"
                                                    && member_str == "conversationUpdated"
                                                {
                                                    let body = msg.body();
                                                    if let Ok(value) =
                                                        body.deserialize::<zbus::zvariant::OwnedValue>()
                                                    {
                                                        if let Some(sms_msg) = parse_sms_message(&value) {
                                                            // Only process messages for our thread
                                                            if sms_msg.thread_id == thread_id {
                                                                tracing::debug!(
                                                                    "Subscription: received message uid={} for thread {}",
                                                                    sms_msg.uid,
                                                                    thread_id
                                                                );
                                                                // Reset phone deadline on matching signal
                                                                if local_store_done {
                                                                    phone_deadline = Some(
                                                                        tokio::time::Instant::now() + phone_timeout,
                                                                    );
                                                                }
                                                                return Some((
                                                                    Message::ConversationMessageReceived {
                                                                        thread_id,
                                                                        message: sms_msg,
                                                                    },
                                                                    ConversationMessageState::Listening {
                                                                        conn,
                                                                        stream,
                                                                        thread_id,
                                                                        device_id,
                                                                        messages_per_page,
                                                                        start_time,
                                                                        local_store_done,
                                                                        total_message_count,
                                                                        phone_deadline,
                                                                    },
                                                                ));
                                                            }
                                                        }
                                                    }
                                                }

                                                // Handle conversationLoaded signals (local store done)
                                                if iface_str == "org.kde.kdeconnect.device.conversations"
                                                    && member_str == "conversationLoaded"
                                                {
                                                    let body = msg.body();
                                                    // Signal args: (conversationId: i64, messageCount: u64)
                                                    if let Ok((conv_id, message_count)) =
                                                        body.deserialize::<(i64, u64)>()
                                                    {
                                                        if conv_id == thread_id {
                                                            tracing::info!(
                                                                "Subscription: conversationLoaded for thread {}, {} messages in store. \
                                                                 Starting phone response window ({:?})...",
                                                                thread_id,
                                                                message_count,
                                                                phone_timeout
                                                            );
                                                            local_store_done = true;
                                                            total_message_count = Some(message_count);
                                                            // Start phone activity deadline
                                                            phone_deadline = Some(
                                                                tokio::time::Instant::now() + phone_timeout,
                                                            );
                                                            // Yield scroll event, then continue
                                                            // in phase 2 (deadline-based timeout for phone data)
                                                            return Some((
                                                                Message::ConversationStoreLoaded {
                                                                    thread_id,
                                                                    total_count: message_count,
                                                                },
                                                                ConversationMessageState::Listening {
                                                                    conn,
                                                                    stream,
                                                                    thread_id,
                                                                    device_id,
                                                                    messages_per_page,
                                                                    start_time,
                                                                    local_store_done,
                                                                    total_message_count,
                                                                    phone_deadline,
                                                                },
                                                            ));
                                                        }
                                                    }
                                                }
                                            }
                                        }
                                        // Non-matching signals: continue loop WITHOUT
                                        // resetting the phone deadline. This is critical —
                                        // unrelated D-Bus traffic must not extend the timeout.
                                    }
                                    Some(Err(e)) => {
                                        tracing::warn!("D-Bus conversation stream error: {}", e);
                                    }
                                    None => {
                                        tracing::warn!(
                                            "D-Bus conversation stream ended for thread {}",
                                            thread_id
                                        );
                                        return Some((
                                            Message::ConversationLoadComplete {
                                                thread_id,
                                                total_count: total_message_count.unwrap_or(0),
                                            },
                                            ConversationMessageState::Done,
                                        ));
                                    }
                                }
                            }

                            // Timeout — either phone deadline or hard deadline
                            _ = tokio::time::sleep(wait_duration) => {
                                if local_store_done {
                                    tracing::info!(
                                        "Subscription: phone response timeout for thread {} \
                                         (no matching signals for {:?} after conversationLoaded)",
                                        thread_id,
                                        phone_timeout
                                    );
                                } else {
                                    tracing::info!(
                                        "Subscription: hard timeout for thread {} \
                                         (no conversationLoaded received)",
                                        thread_id
                                    );
                                }
                                return Some((
                                    Message::ConversationLoadComplete {
                                        thread_id,
                                        total_count: total_message_count.unwrap_or(0),
                                    },
                                    ConversationMessageState::Done,
                                ));
                            }
                        }
                    }
                }
                ConversationMessageState::Done => {
                    // Terminal state - subscription is complete
                    None
                }
            }
        },
    )
}
