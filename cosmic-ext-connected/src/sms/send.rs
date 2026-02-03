//! SMS sending functionality.

use crate::app::Message;
use kdeconnect_dbus::plugins::ConversationsProxy;
use std::sync::Arc;
use tokio::sync::Mutex;
use zbus::zvariant::{Structure, Value};
use zbus::Connection;

/// Send an SMS reply to an existing conversation using sendWithoutConversation.
///
/// Uses the Conversations D-Bus interface with explicit addresses. This avoids
/// the reliability issue with replyToConversation, which silently fails when the
/// daemon's in-memory m_conversations cache is not populated (the cache is only
/// filled by phone-push responses through addMessages(), not by the local-store
/// reads that requestConversation on the Conversations interface performs).
pub async fn send_sms_async(
    conn: Arc<Mutex<Connection>>,
    device_id: String,
    recipients: Vec<String>,
    message: String,
) -> Message {
    let conn = conn.lock().await;
    let device_path = format!("{}/devices/{}", kdeconnect_dbus::BASE_PATH, device_id);

    let conversations_proxy = match ConversationsProxy::builder(&conn)
        .path(device_path.as_str())
        .ok()
        .map(|b| b.build())
    {
        Some(fut) => match fut.await {
            Ok(p) => p,
            Err(e) => {
                return Message::SmsSendResult(Err(format!("Failed to create proxy: {}", e)));
            }
        },
        None => {
            return Message::SmsSendResult(Err("Failed to build proxy path".to_string()));
        }
    };

    // Format addresses as D-Bus structs matching KDE Connect's ConversationAddress: (s)
    let addresses: Vec<Value<'_>> = recipients
        .iter()
        .map(|addr| Value::Structure(Structure::from((addr.clone(),))))
        .collect();
    let empty_attachments: Vec<Value<'_>> = vec![];

    tracing::info!(
        "Sending SMS via sendWithoutConversation to {} recipient(s)",
        addresses.len()
    );

    match conversations_proxy
        .send_without_conversation(addresses, &message, empty_attachments)
        .await
    {
        Ok(()) => {
            tracing::info!("SMS sent successfully via sendWithoutConversation");
            Message::SmsSendResult(Ok(message))
        }
        Err(e) => {
            tracing::error!("SMS send failed: {}", e);
            Message::SmsSendResult(Err(format!("Send failed: {}", e)))
        }
    }
}

/// Send an SMS to a new recipient (creates or adds to existing conversation).
pub async fn send_new_sms_async(
    conn: Arc<Mutex<Connection>>,
    device_id: String,
    recipient: String,
    message: String,
) -> Message {
    let conn = conn.lock().await;
    let device_path = format!("{}/devices/{}", kdeconnect_dbus::BASE_PATH, device_id);

    let conversations_proxy = match ConversationsProxy::builder(&conn)
        .path(device_path.as_str())
        .ok()
        .map(|b| b.build())
    {
        Some(fut) => match fut.await {
            Ok(p) => p,
            Err(e) => {
                return Message::NewMessageSendResult(Err(format!(
                    "Failed to create proxy: {}",
                    e
                )));
            }
        },
        None => {
            return Message::NewMessageSendResult(Err("Failed to build proxy path".to_string()));
        }
    };

    // Format address as D-Bus struct for KDE Connect
    // KDE Connect's ConversationAddress is a struct containing a single string: (s)
    let addresses: Vec<Value<'_>> = vec![Value::Structure(Structure::from((recipient.clone(),)))];
    let empty_attachments: Vec<Value<'_>> = vec![];

    match conversations_proxy
        .send_without_conversation(addresses, &message, empty_attachments)
        .await
    {
        Ok(()) => Message::NewMessageSendResult(Ok("Message sent".to_string())),
        Err(e) => Message::NewMessageSendResult(Err(format!("Send failed: {}", e))),
    }
}
