# CLAUDE.md

Guidance for Claude Code when working with cosmic-ext-connected.

## Project Overview

Connected is a panel applet for the COSMIC™ desktop environment providing phone-to-desktop connectivity. It uses KDE Connect's daemon (`kdeconnectd`) as a D-Bus backend while providing a native libcosmic UI.

**Key Principle:** This project does NOT modify KDE Connect. It consumes kdeconnectd as a D-Bus service.

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
│   │   ├── send.rs                       # SMS sending (replyToConversation / sendWithoutConversation)
│   │   └── conversation_subscription.rs  # Incremental conversation loading
│   ├── media/              # Media player controls
│   └── views/              # Shared UI components
│
├── kdeconnect-dbus/src/
│   ├── daemon.rs           # org.kde.kdeconnect.daemon proxy
│   ├── device.rs           # Device interface proxy
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

## External Resources

- [COSMIC Applets](https://github.com/pop-os/cosmic-applets) - Reference implementations
- [libcosmic](https://github.com/pop-os/libcosmic) - UI toolkit
- [zbus Book](https://dbus2.github.io/zbus/) - D-Bus client docs
- [KDE Connect](https://invent.kde.org/network/kdeconnect-kde) - Protocol reference
