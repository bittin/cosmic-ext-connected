# Notification Systems

Desktop notification implementations for SMS, calls, and file transfers.

## SMS Notifications

Shows desktop notifications when new SMS messages are received.

### Implementation

1. **D-Bus Signal**: Separate subscription listens for `conversationUpdated` signals from `org.kde.kdeconnect.device.conversations`

2. **Message Filtering**: Only incoming messages notified (MessageType::Inbox)

3. **Deduplication**: `last_seen_sms: HashMap<i64, i64>` tracks latest timestamp per thread_id

4. **Contact Resolution**: Sender names resolved via `ContactLookup` using synced vCards

5. **Privacy Settings**:
   - `sms_notifications` - Master toggle
   - `sms_notification_show_sender` - Show/hide sender name
   - `sms_notification_show_content` - Show/hide message preview

### Display

```rust
notify_rust::Notification::new()
    .summary(&summary)  // "New SMS" or "New SMS from {name}"
    .body(&body)        // Message content or "Message received"
    .icon("phone-symbolic")
    .appname("Connected")
    .show()
```

### Subscription Lifecycle

Active when:
- `config.sms_notifications` is enabled
- At least one device is both reachable AND paired

Auto-reconnects on D-Bus disconnection.

## Call Notifications

Shows notifications for incoming and missed phone calls.

### D-Bus Signal

The telephony plugin emits `callReceived` signal with:
- `event` - "callReceived" or "missedCall"
- `phone_number` - Caller's phone number
- `contact_name` - Contact name if available

### Privacy Settings

- `call_notifications` - Master toggle
- `call_notification_show_name` - Show/hide contact name
- `call_notification_show_number` - Show/hide phone number

### Display

```rust
notify_rust::Notification::new()
    .summary(&summary)  // "Incoming Call" or "Incoming call from {name}"
    .body(&device_name) // Which device received the call
    .icon("call-start-symbolic")  // or "call-missed-symbolic"
    .appname("Connected")
    .urgency(notify_rust::Urgency::Critical)
    .show()
```

### Limitation: Mute Ringer

KDE Connect handles ringer muting internally via KNotification. No D-Bus method exposed for external muting - would require upstream changes.

## File Receive Notifications

Shows notifications when files are received from connected devices.

### D-Bus Signal

Share plugin emits `shareReceived` signal with:
- `file_url` - file:// URL of received file

### Privacy Settings

- `file_notifications` - Master toggle

### Display

```rust
notify_rust::Notification::new()
    .summary(&fl!("file-received-from", device = device_name))
    .body(&file_name)
    .icon("folder-download-symbolic")
    .appname("Connected")
    .timeout(notify_rust::Timeout::Milliseconds(5000))
    .show()
```

## Cross-Process Deduplication

COSMIC spawns multiple applet processes. KDE Connect sends 3 duplicate signals per file. Traditional in-process deduplication doesn't work.

### Solution: File-Based Locking

Uses temp file with POSIX file locking (`/tmp/cosmic-connected-file-dedup`):

```rust
fn should_show_file_notification(file_url: &str) -> bool {
    // Open /tmp/cosmic-connected-file-dedup
    // Acquire exclusive lock with flock(fd, LOCK_EX)
    // Check if same file URL within 2 second window
    // Update file with new URL and timestamp
    // Release lock with flock(fd, LOCK_UN)
}
```

Key points:
- Uses `libc::flock()` for atomic locking across processes
- 2-second deduplication window
- Static variables NOT shared between applet instances
