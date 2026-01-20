# SMS Implementation

Details on SMS messaging functionality in COSMIC Connected.

## Signal-Based Data Fetching

Both conversation lists and individual messages are fetched using D-Bus signals rather than polling. This provides reliable loading regardless of phone response time.

### Conversation List Loading

`fetch_conversations_async` in `sms/fetch.rs`:

1. Subscribes to `conversationCreated`, `conversationUpdated`, and `conversationLoaded` signals
2. Loads cached conversations from `activeConversations()` first (instant display)
3. Calls `requestAllConversationThreads()` to trigger fresh data from the phone
4. Collects conversations from signals using activity-based timeout:
   - Stops 500ms after the last signal (once data starts arriving)
   - Hard timeout of 15 seconds maximum
5. Falls back to polling if signal subscription fails

```rust
// Signal-based loading with activity timeout
let activity_timeout = Duration::from_millis(500);
let overall_timeout = Duration::from_secs(15);

loop {
    tokio::select! {
        Some(signal) = created_stream.next() => {
            last_activity = Instant::now();
        }
        Some(signal) = updated_stream.next() => {
            last_activity = Instant::now();
        }
        Some(_) = loaded_stream.next() => {
            loaded_signal_received = true;
            last_activity = Instant::now();
        }
        _ = sleep(Duration::from_millis(50)) => {
            if loaded_signal_received && last_activity.elapsed() >= activity_timeout {
                break; // Done - no signals for 500ms
            }
            if start_time.elapsed() >= overall_timeout {
                break; // Hard timeout
            }
        }
    }
}
```

### Message Thread Loading

Individual message threads use a similar pattern:

1. Subscribe to `conversationUpdated` and `conversationLoaded` signals
2. Call `requestConversation(thread_id, start, count)` to request messages
3. Collect messages from `conversationUpdated` signals as they arrive
4. Stop collecting when `conversationLoaded` signal is received for that thread

## Loading States

The applet uses a phase-based enum to track SMS loading progress:

```rust
pub enum SmsLoadingState {
    Idle,
    LoadingConversations(LoadingPhase),
    LoadingMessages(LoadingPhase),
    LoadingMoreMessages,
}

pub enum LoadingPhase {
    Connecting,  // Setting up D-Bus connection
    Requesting,  // Request sent, waiting for response
}
```

**Phase transitions:**
1. `Idle` → `LoadingConversations(Connecting)` - Opening SMS view without cache
2. `LoadingConversations(Connecting)` → `LoadingConversations(Requesting)` - D-Bus ready
3. `LoadingConversations(Requesting)` → `Idle` - Data received
4. `Idle` → `LoadingConversations(Requesting)` - Opening with cache (skip Connecting)

**Translation strings:**
```ftl
loading-connecting = Connecting...
loading-requesting = Fetching from phone...
```

## Conversation List Caching

Cached in memory to provide instant display when returning to SMS view.

**Behavior:**
- Navigating back preserves conversations in memory
- Re-opening SMS for **same device** shows cache immediately + background refresh
- Switching to **different device** clears cache and loads fresh

```rust
// OpenSmsView checks for cached data
let same_device = self.sms_device_id.as_ref() == Some(&device_id);
let has_cache = same_device && !self.conversations.is_empty();

// CloseSmsView preserves cache
// Keep: sms_device_id, conversations, contacts, message_cache
// Clear: messages, current_thread_id, sms_compose_text
```

**Optimistic updates:** When sending a reply, the conversation list updates immediately:

```rust
if let Some(conv) = self.conversations.iter_mut().find(|c| c.thread_id == thread_id) {
    conv.last_message = sent_body.clone();
    conv.timestamp = now_ms;
}
self.conversations.sort_by(|a, b| b.timestamp.cmp(&a.timestamp));
```

**Message cache:** Individual threads cached in LRU cache (`message_cache: LruCache<i64, Vec<SmsMessage>>`).

## Contact Name Resolution

KDE Connect syncs contacts as vCard files to `~/.local/share/kpeoplevcard/kdeconnect-{device-id}/`.

```rust
let contacts = ContactLookup::load_for_device(&device_id);
let name = contacts.get_name_or_number("+15551234567"); // Returns "John Doe" or the number
```

## Message Type Constants

Android SMS type values (from `msg.message_type`):
- `1` = MESSAGE_TYPE_INBOX (received)
- `2` = MESSAGE_TYPE_SENT
- `3` = MESSAGE_TYPE_DRAFT
- `4` = MESSAGE_TYPE_OUTBOX
- `5` = MESSAGE_TYPE_FAILED
- `6` = MESSAGE_TYPE_QUEUED

## D-Bus Struct Field Order

The message struct from KDE Connect (from `conversationmessage.h`):
- Field 0: `eventField` (i32) - Event flags
- Field 1: `body` (string) - Message text
- Field 2: `addresses` (array) - List of phone numbers
- Field 3: `date` (i64) - Timestamp
- Field 4: `type` (i32) - **Message type** (1=received, 2=sent)
- Field 5: `read` (i32) - Read status
- Field 6: `threadID` (i64) - Conversation thread ID
- Field 7: `uID` (i32) - Unique message ID
- Field 8: `subID` (i64) - SIM ID
- Field 9: `attachments` (array) - Attachment list

Direction determined by field 4:
```rust
let is_received = msg.message_type == MessageType::Inbox; // type == 1
```
