# Known Issues

Known issues and workarounds in Connected.

## Group MMS Reply Creates New Thread on Sender's Phone

Replying to a group MMS conversation delivers the message to recipients but creates a new thread on the sender's phone. This is a KDE Connect protocol limitation, not an applet bug — the same behavior occurs in the native KDE Connect SMS desktop app.

**Upstream issue:** [KDE Bug 501835](https://bugs.kde.org/show_bug.cgi?id=501835)

### Symptoms

- Reply is delivered to all group recipients
- Recipients see the reply in the correct existing thread
- Recipients may receive duplicate copies of the message
- On the sender's phone, the reply appears in a **new** thread instead of the original
- If a recipient replies back, their message appears in the original thread (not the new one)
- 1-on-1 replies are unaffected — they work correctly in all cases

### Technical Details

The applet uses `replyToConversation(threadId, message, attachments)` for replies. The daemon looks up addresses and `subID` from its cached `m_conversations[threadId]` and calls `sendSms()` internally, which sends a `kdeconnect.sms.request` protocol packet to the phone. This packet contains:
- Recipient addresses
- Message body
- SIM subscription ID (`subID`)
- **No thread ID**

Without a thread ID in the packet, the Android KDE Connect app cannot associate the outgoing message with the existing group thread. For 1-on-1 conversations, Android matches by address and finds the unique thread. For groups, the address-set lookup is less reliable, so Android creates a new outgoing thread.

The duplicate messages on recipients' devices may result from `subID` handling or the Android MMS stack's multi-recipient send path.

### Workaround

Use the phone directly to reply to group messages if thread continuity on the sender's phone is important.

## Conversation List Scroll Position

When returning from viewing a message thread to the conversation list, scroll position defaults to bottom (oldest) instead of top (most recent).

### Attempted Solutions

- `scrollable::snap_to` with `RelativeOffset { x: 0.0, y: 0.0 }`
- `scrollable::scroll_to` with `AbsoluteOffset { x: 0.0, y: 0.0 }`
- Changing scrollable ID via key counter to force widget recreation
- Setting explicit `direction` with `Scrollbar::new().anchor(Anchor::Start)`

### Analysis

Issue appears related to how iced/libcosmic preserves scrollable widget state across view changes. Message thread scroll-to-bottom (using `RelativeOffset::END`) works correctly, suggesting problem is specific to scroll-to-top or timing.

### Potential Solutions to Investigate

- Use subscription to trigger scroll after view renders
- Restructure view to not reuse scrollable widget
- Store and restore scroll position manually
- File issue with libcosmic/iced if this is a bug
