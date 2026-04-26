# Changelog

All notable changes to Connected will be documented in this file.

## [Unreleased]

### Fixed
- Pair-state updates more reliable: subscribe to the correct upstream D-Bus signal names (`pairStateChanged` and  daemon-level `pairingRequestsChanged`), replacing three names that did not exist in upstream KDE Connect.         Pair-state was previously riding only on the `PropertiesChanged` catch-all
- Pair-state updates no longer silently dropped during signal bursts: a follow-up refresh now fires after each signal-triggered fetch, picking up settled state even when trailing signals fall inside the 3 s debounce window
- Pair acceptance picked up promptly when accepting on the phone right after sending the request from Connected: a 1 s tick now flushes any deferred refresh once the debounce window clears, so the UI no longer hangs on "Waiting for device to accept" until the next ambient signal

### Added
- Desktop / laptop / TV peer support; "Show non-mobile devices" setting; inline share primitives on non-mobile device page; Enter-to-send on share-text inputs.

### Changed
- Accent color restricted to actionable UI; tightened device page layout; Pair/Unpair moved into the actions list; notification count rendered as a badge.

### Fixed
- SMS thread truncation on first open: re-issues requestConversation when the daemon re-emits conversationLoaded with more messages than we received, and as a fallback on phone-response timeout.
- Message thread auto-scrolls to the newest message on open even for cached threads where the daemon doesn't emit   conversationLoaded.

## [0.3.0] - 2026-04-14

### Changed
- Replaced rfd/gtk3 file dialog with libcosmic's native xdg-portal file chooser
- Renamed symbolic panel icons with app ID prefix so Flatpak exports them to the host

### Fixed
- SMS message handling: optimistic send with body-based reconciliation
- Conversation list and message thread subscriptions converted to long-lived (fixes premature termination during bursty phone signals)
- Conversation list bootstrap: settles on a quiet window, retries once on small cold-start results, and re-reads daemon cache to merge late-arriving data
- Cold start false "no conversations" display (spinner shown until data arrives)
- Missing applet icon in COSMIC Panel settings for Flatpak installs
- Conversation list: long message previews no longer push the date and chevron off the right edge

## [0.2.1] - 2026-04-12

### Changed
- Updated for libcosmic API changes (removal of top-level `cosmic::iced_*` re-exports)
- Track `Cargo.lock` to pin dependency versions

### Added
- Swedish translations for attachment-related strings

## [0.2.0] - 2026-04-03

### Added
- MMS attachment viewing (thumbnails in message bubbles, click to open)
- Multi-recipient support for new message compose (group messaging)
- Optimistic SMS sending indicator
- Conversation prefetch on device selection

### Changed
- Switched SMS replies to `replyToConversation` for better thread context
- Replaced optimistic SMS bubbles with simpler sending indicator

### Fixed
- APP_ID mismatch preventing applet from appearing on panel
- Empty conversation list on cold start (increased timeout with fallback)
- Compose row disappearing due to iced Shrink height compression
- Conversation list not updating after sending a new message
- MMS thumbnail display (whitespace in base64 data)

## [0.1.0] - 2026-02-10

### Added
- Native COSMIC desktop applet for phone connectivity via KDE Connect D-Bus
- Device discovery, pairing/unpairing, and management
- SMS messaging with conversation list, message threads, and compose
- Contact name resolution from synced vCards
- Group message sender name display
- Long-press to copy SMS message text
- Scroll-based lazy loading for SMS messages
- Subscription-based incremental conversation loading
- Media controls (play/pause, next/previous, volume, player selection)
- File receive notifications with cross-process deduplication
- Call notifications for incoming and missed calls (with privacy controls)
- SMS desktop notifications (with privacy controls)
- Find My Phone feature to ring connected devices
- Battery status display
- Clipboard sync (send to device)
- File and URL sharing
- Ping functionality
- Settings panel with notification privacy options and configurable timeout
- Custom panel icons (connected/disconnected states)
- Accent color theming
- Swedish translations
- Flatpak packaging support

### Changed
- Renamed applet from "COSMIC Connect" to "Connected"
- Renamed package from `cosmic-applet-connect` to `cosmic-ext-connected`
- SMS compose sends message on Enter key press
