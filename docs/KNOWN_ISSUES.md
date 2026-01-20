# Known Issues

Known issues and workarounds in COSMIC Connected.

## Group MMS Sending Not Supported

Sending messages to group MMS conversations (multiple recipients) does not work reliably with KDE Connect.

**Upstream issue:** [KDE Bug 501835](https://bugs.kde.org/show_bug.cgi?id=501835)

### Symptoms

- Replying to a group message thread silently fails
- D-Bus call to `sendSms` returns success but message never appears on phone
- Affects COSMIC Connected, native KDE Connect SMS app, and kdeconnect-cli alike

### Technical Details

- KDE Connect can receive and display group MMS messages
- `sendSms` D-Bus method accepts multiple addresses but Android app doesn't process them correctly for MMS groups
- Issue may be device/ROM-specific
- MMS group identity on Android tied to internal thread IDs, not participant list

### Current Handling

The applet detects group conversations (multiple unique recipients) and shows "Group messaging not supported" when attempting to send.

### Workaround

Use the phone directly to reply to group messages.

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
