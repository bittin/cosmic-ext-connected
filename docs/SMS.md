# SMS Implementation

Details on SMS messaging functionality in Connected.

## Signal-Based Data Fetching

Both conversation lists and individual messages are fetched using D-Bus signals rather than polling. This provides reliable loading regardless of phone response time.

### Conversation List Loading (Subscription-Based)

Conversation lists use a **subscription-based** approach for incremental display, mirroring the pattern used by KDE Connect SMS app:

```
OpenSmsView → Set state, activate subscription
                        ↓
         Subscription sets up D-Bus match rules
                        ↓
         Load cached conversations via activeConversations()
                        ↓
         Emit ConversationReceived for each cached conversation
                        ↓
         Fire requestAllConversationThreads() D-Bus call
                        ↓
         conversationCreated/Updated signals → ConversationReceived messages
                        ↓
         Activity timeout (500ms) or hard timeout (20s) → ConversationSyncComplete
```

**Key implementation details:**

1. `conversation_list_subscription` in `sms/conversation_subscription.rs` handles the entire flow
2. Uses a state machine with three states: `Init`, `EmittingCached`, `Listening`
3. Match rules are set up BEFORE firing the D-Bus request (prevents race conditions)
4. Cached conversations emitted one-at-a-time for immediate UI updates
5. New signals processed incrementally - each conversation appears as it arrives
6. "Syncing conversations..." indicator shown while subscription is active

```rust
// State machine for incremental loading
enum ConversationListState {
    Init { device_id: String },
    EmittingCached { pending_conversations: Vec<ConversationSummary>, ... },
    Listening { stream: MessageStream, ... },
}
```

```rust
// In app.rs - incremental display
Message::ConversationReceived { device_id, conversation } => {
    // Update or insert by thread_id
    if let Some(existing) = self.conversations.iter_mut()
        .find(|c| c.thread_id == conversation.thread_id)
    {
        if conversation.timestamp > existing.timestamp {
            *existing = conversation;
        }
    } else {
        self.conversations.push(conversation);
    }
    // Re-sort by timestamp (newest first)
    self.conversations.sort_by(|a, b| b.timestamp.cmp(&a.timestamp));
}
```

**Benefits over batch loading:**
- Conversations appear immediately as they arrive (no waiting for timeout)
- Visual feedback during sync ("Syncing conversations...")
- Mirrors KDE Connect SMS app's proven approach

### Message Thread Loading (Subscription-Based)

Individual message threads use a **subscription-based** approach for incremental display:

```
OpenConversation → Set state, activate subscription
                            ↓
         Subscription sets up D-Bus match rules
                            ↓
         Subscription fires requestConversation() D-Bus call
                            ↓
         conversationUpdated signals → ConversationMessageReceived messages
                            ↓
         conversationLoaded signal → ConversationLoadComplete message
```

**Key implementation details:**

1. `conversation_message_subscription` in `subscriptions.rs` handles the entire flow
2. Match rules are set up BEFORE firing the D-Bus request (prevents race conditions)
3. Messages arrive via `ConversationMessageReceived` and are inserted sorted by date
4. After each message insert, scroll-to-bottom keeps newest messages visible
5. `ConversationLoadComplete` finalizes state when `conversationLoaded` signal arrives

```rust
// In subscriptions.rs - subscription fires requests after setup
let stream = zbus::MessageStream::from(&conn);

// Fire TWO requests after match rules are ready:
// 1. SMS plugin (cache priming): sends network packet to phone → response flows
//    through addMessages() → populates m_conversations (required for replyToConversation)
sms_proxy.request_conversation(thread_id, 0, count).await?;
// 2. Conversations interface (UI signals): reads from local store via worker →
//    emits per-message conversationUpdated signals for our subscription
conversations_proxy.request_conversation(thread_id, 0, count).await?;

// Listen for conversationUpdated/conversationLoaded signals...
```

```rust
// In app.rs - incremental display with scroll anchoring
Message::ConversationMessageReceived { thread_id, message } => {
    // Insert sorted by date
    let insert_pos = self.messages.iter()
        .position(|m| m.date > message.date)
        .unwrap_or(self.messages.len());
    self.messages.insert(insert_pos, message);

    // Scroll to bottom after each insert (keeps newest visible)
    return scrollable::snap_to(Id::new("message-thread"), RelativeOffset::END);
}
```

**Benefits over blocking approach:**
- Messages appear immediately as they arrive (no timeout delay)
- Scroll stays anchored to newest messages during loading
- No arbitrary timeouts - completion signaled by `conversationLoaded`

### Scroll-Based Lazy Loading (Older Messages)

When viewing a conversation, older messages load automatically as the user scrolls up:

```
User scrolls near top (< 100px) → MessageThreadScrolled
                                          ↓
                    Store scroll position for later adjustment
                                          ↓
                    fetch_older_messages_async(offset, count)
                                          ↓
                    OlderMessagesLoaded → Prepend messages
                                          ↓
                    Calculate new scroll position → snap_to()
```

**Key implementation details:**

1. `MessageThreadScrolled` in `app.rs` triggers when user scrolls within 100px of top
2. Scroll offset and content height stored before loading (for position preservation)
3. `fetch_older_messages_async` requests messages with pagination offset
4. When messages arrive, they're prepended and scroll position is adjusted

```rust
// In app.rs - scroll-based loading trigger
Message::MessageThreadScrolled(viewport) => {
    const PREFETCH_THRESHOLD_PX: f32 = 100.0;
    let scroll_offset = viewport.absolute_offset().y;

    if scroll_offset < PREFETCH_THRESHOLD_PX
        && self.messages_has_more
        && !self.is_loading_more_messages()
    {
        // Store scroll state for position preservation
        self.scroll_offset_before_load = Some(scroll_offset);
        self.content_height_before_load = Some(viewport.content_bounds().height);

        // Trigger async load
        return Task::perform(fetch_older_messages_async(...), ...);
    }
}
```

**Scroll position preservation:** When older messages are prepended, the scroll position is adjusted so the user stays at the same messages they were viewing:

```rust
// In OlderMessagesLoaded handler
let prepended_height = prepended_count as f32 * ESTIMATED_MSG_HEIGHT;
let new_content_height = old_height + prepended_height;
let new_offset = old_offset + prepended_height;
let relative_y = (new_offset / new_content_height).clamp(0.0, 1.0);

return scrollable::snap_to(
    widget::Id::new("message-thread"),
    scrollable::RelativeOffset { x: 0.0, y: relative_y },
);
```

**Loading indicator:** A subtle "Loading..." indicator appears at the top while fetching, but no manual button is required - loading is entirely scroll-driven.

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
5. `Idle` → `LoadingMoreMessages` - User scrolls near top of message thread
6. `LoadingMoreMessages` → `Idle` - Older messages received and prepended

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
// Keep: sms_device_id, conversations, contacts
// Clear: messages, current_thread_id, sms_compose_text
```

**Send flow:** Both replies and new messages use `sendWithoutConversation` (Conversations D-Bus interface) with explicit addresses. This avoids `replyToConversation` which silently fails when the daemon's in-memory `m_conversations` cache is not populated (see Cache Priming below). On success, the conversation list preview updates immediately (last_message + timestamp), but no fake message is inserted into the thread. A delayed refresh (~2s) fetches the real sent message from the phone.

**Cache priming:** When loading a conversation, two `requestConversation` calls are fired in sequence: (1) the SMS plugin's version (on `/devices/{id}/sms`) sends a network packet to the phone — the response flows through `addMessages()` which populates the daemon's in-memory `m_conversations` cache; (2) the Conversations interface's version reads from a local persistent store and emits per-message `conversationUpdated` signals for the UI. Both are needed because `replyToConversation` looks up addresses from `m_conversations` (only populated by the SMS plugin path), while the UI needs per-message signals (only provided by the Conversations interface path).

```rust
// Conversation list preview updates immediately
if let Some(conv) = self.conversations.iter_mut().find(|c| c.thread_id == thread_id) {
    conv.last_message = sent_body;
    conv.timestamp = now_ms;
}
self.conversations.sort_by(|a, b| b.timestamp.cmp(&a.timestamp));
// Delayed refresh brings the real sent message into the thread
```

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
