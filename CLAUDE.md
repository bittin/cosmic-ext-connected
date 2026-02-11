# CLAUDE.md

Guidance for Claude Code when working with cosmic-ext-connected.

## Project Overview

Connected is a panel applet for the COSMIC™ desktop environment providing phone-to-desktop connectivity. It uses KDE Connect's daemon (`kdeconnectd`) as a D-Bus backend while providing a native libcosmic UI.

**Key Principle:** This project does NOT modify KDE Connect. It consumes kdeconnectd as a D-Bus service.

## Release Management

- **`main`** — sole development branch (no stable release branches yet)
- **`v0.1.0`** tag exists as a historical marker for the first GitHub release
- All work (features and fixes) goes to `main` directly or via `feature/*` branches
- A release branch will be created when the project is published to a Flatpak repository or package archive

## Architecture

```
┌─────────────────────────────────────────────────────────────────┐
│  Connected Applet (Rust)                                        │
│  ├── cosmic-ext-connected/     (UI layer - libcosmic)          │
│  └── kdeconnect-dbus/          (D-Bus client - zbus)           │
└──────────────────────┬──────────────────────────────────────────┘
                       │ D-Bus (org.kde.kdeconnect.*)
                       ▼
┌─────────────────────────────────────────────────────────────────┐
│  kdeconnectd (system package: apt install kdeconnect)          │
└──────────────────────┬──────────────────────────────────────────┘
                       │ TCP/UDP/Bluetooth
                       ▼
┌─────────────────────────────────────────────────────────────────┐
│  Android Phone (KDE Connect app)                                │
└─────────────────────────────────────────────────────────────────┘
```

## Build Commands

```bash
cargo build                              # Build all crates
cargo build --release                    # Build release
cargo run -p cosmic-ext-connected        # Run (requires COSMIC)
cargo test && cargo clippy               # Test and lint
just install                             # Install to system
just uninstall                           # Uninstall
```

**Development cycle:** `cargo build --release && sudo just install && killall cosmic-panel`

**Flatpak build:**
```bash
flatpak-builder --user --install --force-clean build-dir io.github.nwxnw.cosmic-ext-connected.json
gtk-update-icon-cache -f ~/.local/share/flatpak/exports/share/icons/hicolor/  # Force icon cache refresh
killall cosmic-panel                     # Reload panel
```

**Flatpak sandbox permissions** (in `finish-args` of manifest):
- `--filesystem=xdg-config/cosmic:rw` — read/write COSMIC config
- `--filesystem=xdg-data/kpeoplevcard:ro` — read contacts for SMS name resolution
- `--filesystem=xdg-cache/kdeconnect.daemon:ro` — read MMS attachment cache (daemon uses Qt app name `"kdeconnect.daemon"` → cache at `~/.cache/kdeconnect.daemon/`)

**Debug logs:** `journalctl --user -f | grep cosmic-ext-connected`

## Project Structure

```
cosmic-ext-connected/
├── cosmic-ext-connected/src/
│   ├── app.rs              # Core: ConnectApplet, Message enum, update()
│   ├── config.rs           # User preferences (cosmic_config)
│   ├── notifications.rs    # Cross-process notification deduplication
│   ├── subscriptions.rs    # D-Bus signal subscriptions
│   ├── device/             # Device fetch and actions
│   ├── sms/                # SMS conversations, views, subscriptions
│   │   ├── send.rs                       # SMS sending (sendWithoutConversation for both replies and new messages)
│   │   ├── conversation_subscription.rs  # Dual D-Bus request + incremental conversation loading
│   │   ├── fetch.rs                      # Conversation fetching and caching
│   │   └── views.rs                      # SMS conversation list and thread views
│   ├── media/              # Media player controls
│   └── views/              # Shared UI components
│
├── data/
│   ├── io.github.nwxnw.cosmic-ext-connected.desktop
│   ├── io.github.nwxnw.cosmic-ext-connected.metainfo.xml
│   └── icons/hicolor/scalable/apps/
│       ├── io.github.nwxnw.cosmic-ext-connected.svg          # App icon (128x128, #BEBEBE fill)
│       ├── cosmic-ext-connected-symbolic.svg                  # Panel: connected state
│       └── cosmic-ext-connected-disconnected-symbolic.svg     # Panel: disconnected state
│
├── io.github.nwxnw.cosmic-ext-connected.json  # Flatpak manifest
│
├── kdeconnect-dbus/src/
│   ├── daemon.rs           # org.kde.kdeconnect.daemon proxy
│   ├── device.rs           # Device interface proxy
│   ├── contacts.rs         # ContactLookup: vCard parsing, name resolution, group display names
│   └── plugins/            # Per-plugin D-Bus proxies
│
└── docs/                   # Detailed implementation docs
    ├── DBUS.md             # D-Bus interface reference
    ├── SMS.md              # SMS implementation details
    ├── NOTIFICATIONS.md    # Notification systems
    ├── MEDIA.md            # Media controls
    ├── UI_PATTERNS.md      # libcosmic UI patterns
    └── KNOWN_ISSUES.md     # Known issues and workarounds
```

## Key Conventions

### Internationalization
All user-visible text must use `fl!()` macro:
```rust
text(fl!("devices"))
text(fl!("battery-level", level = battery_percent))
```

### D-Bus Property Naming
KDE Connect uses camelCase. Always specify explicit names in zbus:
```rust
#[zbus(property, name = "isCharging")]
fn is_charging(&self) -> zbus::Result<bool>;
```

### Module Organization
- `Message` enum stays in `app.rs` - all modules import from app
- `ConnectApplet` struct stays in `app.rs` - centralized state
- View functions take params structs, return `Element<Message>`
- Async functions are standalone, return `Message` variants

### Icons
- **Panel icons** use `fill="currentColor"` (symbolic) so COSMIC themes them automatically
- **App icon** uses hardcoded `fill="#BEBEBE"` because COSMIC Settings does not theme non-symbolic app icons
- Device icon everywhere is `"cosmic-ext-connected-symbolic"` (our custom mobile phone icon)
- Panel connected state: `"cosmic-ext-connected-symbolic"`, disconnected: `"cosmic-ext-connected-disconnected-symbolic"`
- Disconnected icon follows Pop convention: main element at `opacity="0.35"`, X indicator at full opacity

### UI Theming
- Back navigation buttons use `cosmic::theme::Button::Link` (accent-colored icon and text, no background)
- Headings adjacent to back buttons use `cosmic::theme::Text::Accent`
- To accent-color an icon widget, convert `Named` to `Icon` first, then apply `Svg::custom`:
  ```rust
  icon::from_name("icon-name").size(48).icon()
      .class(theme::Svg::custom(|theme| svg::Style {
          color: Some(theme.cosmic().accent_text_color().into()),
      }))
  ```

### Code Style
- Follow rustfmt and clippy
- Prefer explicit error handling over `.unwrap()`
- Use `fl!()` for all UI strings

## Detailed Documentation

For implementation details, see docs/:
- **[docs/DBUS.md](docs/DBUS.md)** - D-Bus interfaces, testing commands, signal subscription
- **[docs/SMS.md](docs/SMS.md)** - SMS fetching, caching, loading states, message types
- **[docs/NOTIFICATIONS.md](docs/NOTIFICATIONS.md)** - SMS/call/file notifications, deduplication
- **[docs/MEDIA.md](docs/MEDIA.md)** - Media player controls, sendAction pattern
- **[docs/UI_PATTERNS.md](docs/UI_PATTERNS.md)** - libcosmic patterns, ViewMode, popups
- **[docs/KNOWN_ISSUES.md](docs/KNOWN_ISSUES.md)** - Known issues and workarounds

## D-Bus Interface Pitfalls

### Two `requestConversation` methods (different behavior)
The daemon exposes `requestConversation` on two interfaces with **different behavior**:

- **`org.kde.kdeconnect.device.sms`** (at `/devices/{id}/sms`): Sends a network packet to the phone. The response flows through `addMessages()` which populates the daemon's in-memory `m_conversations` cache. Use this to prime the cache.
- **`org.kde.kdeconnect.device.conversations`** (at `/devices/{id}`): Creates a `RequestConversationWorker` that reads from a local persistent store and emits `conversationUpdated` D-Bus signals, but does **NOT** populate `m_conversations`.

We fire both: SMS plugin first (cache priming), then Conversations interface (per-message signals for UI).

### Contact loading is per-device and loaded once
`ContactLookup` parses vCard files from `~/.local/share/kpeoplevcard/kdeconnect-{device-id}/`. Contacts are loaded once when the SMS view opens and preserved across view re-opens for the same device. They are only cleared when switching to a different device. This avoids a race condition where async contact loading completes after the conversation list has already rendered with phone numbers.

### Group display names
`ContactLookup::get_group_display_name(&addresses, limit)` resolves multiple addresses into comma-separated contact names (e.g. "Alice, Bob, Charlie, ..."). Used in the conversation list, thread header, and SMS notifications. Per-message sender labels use `get_name_or_number` for the individual message sender.

### `conversationLoaded` signal count is unreliable
`conversationLoaded(threadId, messageCount)` fires when the Conversations interface finishes reading its local persistent store. The `messageCount` reflects **how many messages were in the local store**, not the phone's total. After a reboot, the local store may have 0-1 messages even for conversations with hundreds. Never use this count for pagination decisions — use `messages.len() >= messages_per_page` as a heuristic instead.

### Message loading uses a deadline-based subscription with three phases
`conversation_message_subscription` in `subscriptions.rs` uses a state machine (`Init` → `Listening` → `Done`) with three phases:
- **Phase 1:** Wait for local store signals (`conversationUpdated` per message, then `conversationLoaded`). No activity timeout — just the hard timeout safety net (20s).
- **Phase 2a:** After `conversationLoaded`, `phone_deadline` (8s, `PHONE_RESPONSE_TIMEOUT_MS`) waits for the phone to START responding. Only checked when `activity_deadline` is `None` (no phone messages yet).
- **Phase 2b:** After the first phone message, `activity_deadline` (3s, `PHONE_ACTIVITY_TIMEOUT_MS`) takes over. Reset on each matching signal. Once set, `phone_deadline` is ignored. This prevents the previous issue where each message extended the wait by 8s.

Both deadlines reset **only on matching signals** (messages for our thread), not on unrelated D-Bus traffic. Both must be stored in the `Listening` state struct (not local variables) because each `unfold` yield exits and re-enters the function.

### Scroll prefetch must wait for subscription to complete
`MessageThreadScrolled` triggers `fetch_older_messages_async` when the user scrolls near the top. This fires a separate `requestConversation` which picks up D-Bus signals on the same session bus. If the initial message subscription is still running, the prefetch collects the **same signals** and `OlderMessagesLoaded` prepends duplicates. Guard: `!self.conversation_load_active` in the prefetch condition. Safety net: `OlderMessagesLoaded` filters `older_msgs` against `known_message_ids` before prepending.

### Conversation list loading uses deadline-based subscription
`conversation_list_subscription` in `conversation_subscription.rs` uses a state machine (`Init` → `EmittingCached` → `Listening` → `Done`) with deadline-based timeouts:
- **`phone_deadline`** — absolute `Instant` for how long to wait for the phone to start responding. 8s on cold start (no cache), 3s on warm start (has cache). Only checked when no live signals have arrived yet.
- **`activity_deadline`** — `Option<Instant>`, initially `None`. Set/reset to `now + 3s` on each live D-Bus signal (`conversationCreated`, `conversationUpdated`, `conversationLoaded`). Once set, `phone_deadline` is no longer checked.

Cached conversations from `activeConversations()` are emitted immediately via `EmittingCached` for instant UI display, but they do **not** arm the activity timeout. This prevents the subscription from dying before the phone responds (network RTT is 1-5s).

### COSMIC's notification daemon ignores `expire_timeout`
COSMIC's notification daemon does not respect the `expire_timeout` hint from the freedesktop notification spec. All notifications display for the daemon's fixed duration regardless of the value passed. To implement user-configurable notification duration, notifications must be created with `Timeout::Never` (expire_timeout=0) to prevent the daemon from auto-closing them, then explicitly closed via `NotificationHandle::close()` after the configured timeout. This is handled by `show_and_auto_close()` in `app.rs`.

### Ping notifications cannot be intercepted via D-Bus
KDE Connect's ping plugin (`kdeconnect_ping`) does not emit D-Bus signals for incoming pings. When a ping is received, `kdeconnectd` handles it internally and sends a desktop notification directly via `KNotification`, bypassing any D-Bus signal mechanism. The applet cannot detect or replace incoming ping notifications. The ping plugin only exposes `sendPing()` methods (outgoing), not incoming signals.

### SMS sending: `replyToConversation` for replies, `sendWithoutConversation` for new messages
Replies use `replyToConversation(threadId, message, attachments)`, which looks up addresses from the daemon's `m_conversations[threadId]` cache. This cache is reliably primed when the user opens a conversation (our SMS plugin `requestConversation` call populates it via `addMessages()`). Using `replyToConversation` preserves thread context for group messages.

**Caveat:** `replyToConversation` silently no-ops if the cache is empty (no D-Bus error). The KDE Connect desktop SMS app uses it the same way with no fallback. See `reference/reply-to-conversation-analysis.md` for detection/fallback options if this proves unreliable.

New messages (no existing thread) use `sendWithoutConversation(addresses, message, attachments)` with explicit addresses. Both methods are on the same Conversations D-Bus interface.

### KDE Connect daemon cache path is `kdeconnect.daemon`, not `kdeconnect`
KDE Connect's daemon sets its Qt application name to `"kdeconnect.daemon"` in `kdeconnectd.cpp`. Qt's `QStandardPaths::CacheLocation` resolves to `~/.cache/<applicationName>/`, so MMS attachments are cached at `~/.cache/kdeconnect.daemon/<device-name>/<uniqueIdentifier>` (e.g. `PART_1762553269778`). Files have no extension — the MIME type comes from the message's attachment metadata. The Flatpak manifest must include `--filesystem=xdg-cache/kdeconnect.daemon:ro` for the applet to read cached attachments.

## External Resources

- [COSMIC Applets](https://github.com/pop-os/cosmic-applets) - Reference implementations
- [libcosmic](https://github.com/pop-os/libcosmic) - UI toolkit
- [zbus Book](https://dbus2.github.io/zbus/) - D-Bus client docs
- [KDE Connect](https://invent.kde.org/network/kdeconnect-kde) - Protocol reference
