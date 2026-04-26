# SMS Implementation Notes

Current SMS behavior in Connected, plus known limitations and possible follow-up work.

## Current Status

Implemented:

- Conversation-list bootstrap is cache-first and primarily signal-driven.
- Bootstrap re-reads `activeConversations()` during initial sync, settles after a quiet window, and retries once on suspiciously small cold-start results.
- Warm starts preserve cached timeout semantics instead of fabricating activity.
- Conversation threads use a long-lived subscription with deferred initial scroll, pagination, and optimistic reply reconciliation.
- Device selection can prefetch cached conversation heads before the SMS view opens.

Possible next steps:

- Move SMS state out of `app.rs` and per-view subscriptions into a shared SMS session/store.
- Reduce the differences between conversation-list and thread-loading synchronization models.
- Keep reviewing thread correctness around deduplication, pagination heuristics, notification interaction, scroll preservation, and reply/cache-priming assumptions.
- Keep bootstrap logging focused on cache size, signal activity, retry use, and final settled counts.

## Architecture Overview

### Conversation List Loading

Conversation-list loading is cache-first and long-lived:

1. Opening the SMS view starts `conversation_list_subscription`.
2. The subscription installs D-Bus match rules before firing bootstrap requests.
3. Cached conversations from `activeConversations()` are emitted immediately.
4. Bootstrap requests plus follow-up cache polls merge newer data into the list.
5. A quiet window or bootstrap deadline dismisses the sync indicator.
6. The subscription remains alive while the SMS view is open.

Important details:

- Warm starts use cached rows immediately and a shorter bootstrap window.
- Cold starts use a longer wait and one bounded retry.
- The flow is primarily signal-driven, but bootstrap also re-reads daemon cache to catch late-arriving data.

### Message Thread Loading

Thread loading uses a long-lived subscription with distinct startup phases:

1. Opening a thread starts `conversation_message_subscription`.
2. Match rules are installed before requests are fired.
3. Two `requestConversation()` calls are made:
   - SMS plugin request for daemon cache priming
   - Conversations request for per-message UI signals
4. The local persistent-store phase ends at `conversationLoaded`.
5. A phone-response window stays open after local-store completion to catch delayed phone data.
6. The subscription then continues listening for new incoming messages and sent-message echoes until the thread closes.

Important details:

- The list is rendered oldest-first, so an unscrolled scrollable lands on the oldest message. Auto-scroll-to-bottom is dispatched on `ConversationStoreLoaded`, on `ConversationLoadComplete`, on each `ConversationMessageReceived` while `!initial_load_complete`, and on confirmed sent-message echoes. The per-message dispatch covers cached-store hits where the daemon's worker satisfies the request entirely from `m_conversations` and never emits `conversationLoaded` (see `requestconversationworker.cpp`'s `numHandled >= howMany` branch), which would otherwise leave the user pinned at the top of a long thread.
- `conversationLoaded` reflects local-store count, not authoritative phone total.
- `initial_load_complete` gates scroll-based loading of older messages, and bounds the per-message auto-scroll so a new incoming SMS arriving while the user is reading older content doesn't yank them down.
- The daemon writes phone-supplied messages to the local store asynchronously, so the first-open Conversations worker may finish before that data lands. The daemon's `addMessages()` only emits `conversationUpdated` for the latest message in a thread, so historical backfill from the phone arrives silently — observable only via a second `conversationLoaded(count)` emission with a higher count than we've received. Recovery is bounded to one re-issued `requestConversation` per thread open, with two triggers:
  - **Primary** (Option 1): a duplicate `conversationLoaded` arrives with `store_count > received_message_count` while we're still under-filled (`received < messages_per_page`). The retry fires immediately. Catches both the original "received only 1 of N" truncation and the off-by-one "received N-1 of N" case where the daemon's worker emits one fewer per-message signal than its store reports. The page-size guard avoids firing on natural scroll-pagination boundaries.
  - **Fallback**: `phone_deadline` expires with `received <= 1` — used if the daemon doesn't re-emit `conversationLoaded` (e.g. the phone added no new UIDs, or a signal-ordering race). Narrow gate kept here on purpose: if no duplicate fired, retry against an unchanged store would just re-deliver what we already have.
  The retry uses `requestConversation(threadId, received, received + page)` — same offset shape as KDE Connect's `ConversationModel::requestMoreMessages`. The resulting per-message signals merge via `known_message_ids` dedup in `app.rs`.

### Older Message Loading

Older messages are loaded automatically when the user scrolls near the top of the thread.

- Scroll position and content height are captured before the fetch.
- Older messages are prepended when they arrive.
- Scroll offset is adjusted so the user stays anchored near the same visible messages.

## Sending Behavior

### Replies

Replies use `replyToConversation(threadId, message, attachments)` on the Conversations D-Bus interface. This preserves thread context, including group conversations, but depends on the daemon's in-memory `m_conversations` cache being primed first.

On success:

- the conversation preview updates immediately with the latest body and timestamp
- an optimistic sent bubble is inserted into the open thread
- the long-lived message subscription reconciles that optimistic entry when the phone echoes back the real sent message

### New Messages

New-message compose uses `sendWithoutConversation(addresses, message, attachments)` with explicit recipients. On success, the compose flow returns to the conversation list and keeps the conversation-list subscription active so the phone can sync back the resulting thread.

### Cache Priming

Thread loading fires two requests because they serve different purposes:

- the SMS plugin request populates the daemon's in-memory `m_conversations` cache
- the Conversations request emits the per-message signals used by the UI

Reply sending depends on the first; thread rendering depends on the second.

## Loading and Caching

Loading state is tracked with `SmsLoadingState`:

- `Idle`
- `LoadingConversations(Connecting|Requesting)`
- `LoadingMessages(Connecting|Requesting)`
- `LoadingMoreMessages`

Caching behavior:

- Re-opening SMS for the same device reuses in-memory conversation data and refreshes in the background.
- Switching devices clears device-specific SMS state as needed.
- Contacts are loaded per device from KDE Connect's synced vCard directory and reused for same-device reopens.

## Known Constraints

- `conversationLoaded` reports the local persistent-store count, not the phone's authoritative total.
- Reply sending still depends on daemon cache priming before `replyToConversation` can work reliably.
- Group-message behavior remains subject to KDE Connect limitations documented in `docs/KNOWN_ISSUES.md`.
- Notification correctness depends on careful `last_seen_sms` handling when opening threads and merging incoming data.

## Reference

### Key Symbols

Messages (see `app.rs`):

- `ConversationReceived` — cached or newly discovered conversation summary
- `ConversationSyncStarted` / `ConversationSyncComplete` — spinner lifecycle for the list
- `ConversationMessageReceived` — individual message during thread load or live updates
- `ConversationStoreLoaded` — local persistent-store phase finished (triggers initial scroll)
- `ConversationLoadComplete` — phone-response window elapsed (sets `initial_load_complete`)

Timeout constants (see `constants.rs`):

- `CONVERSATION_LIST_PHONE_WAIT_MS` — cold-start bootstrap ceiling
- `CONVERSATION_TIMEOUT_CACHED_SECS` — warm-start bootstrap window
- `CONVERSATION_LIST_QUIET_MS` — quiet-window settle after bootstrap activity
- `CONVERSATION_LIST_CACHE_POLL_MS` — cache re-read interval during bootstrap
- `CONVERSATION_LIST_RETRY_THRESHOLD` / `CONVERSATION_LIST_RETRY_WAIT_MS` — cold-start retry gate and window
- `PHONE_RESPONSE_TIMEOUT_MS` — thread phone-response window after `conversationLoaded`
- `CONVERSATION_RETRY_WAIT_MS` — settle window for the one-shot Conversations-interface re-read fired when first-open truncation is suspected
- `MESSAGE_SUBSCRIPTION_TIMEOUT_SECS` — Phase 1 local-store safety-net timeout

D-Bus surface:

- Device base path: `/modules/kdeconnect/devices/{id}`
- Conversations interface: `org.kde.kdeconnect.device.conversations` (signals: `conversationCreated`, `conversationUpdated`, `conversationLoaded`)
- SMS plugin path: `/modules/kdeconnect/devices/{id}/sms` (`org.kde.kdeconnect.device.sms`) — used for cache priming via `requestConversation` / `requestAllConversations`

### Message Types

- Message types: `1 = inbox`, `2 = sent`, `3 = draft`, `4 = outbox`, `5 = failed`, `6 = queued`
- Message fields relied on by the app: body, addresses, date, type, read, thread ID, UID, sub ID, attachments

## Related Docs

- `docs/KNOWN_ISSUES.md`
- `docs/NOTIFICATIONS.md`
- `docs/DBUS.md`
