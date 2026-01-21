# UI Patterns

libcosmic patterns and UI conventions used in Connected.

## ViewMode Enum

The applet tracks current view:

```rust
pub enum ViewMode {
    DeviceList,       // Main device list
    DevicePage,       // Individual device details
    SendTo,           // "Send to device" submenu
    ConversationList, // SMS conversations
    MessageThread,    // SMS message thread
    NewMessage,       // Compose new SMS
    Settings,         // Settings panel
    MediaControls,    // Media player controls
}
```

## Popup Width Management

Two popup widths:

```rust
const DEFAULT_POPUP_WIDTH: f32 = 360.0;  // Standard libcosmic
const WIDE_POPUP_WIDTH: f32 = 450.0;     // SMS/media views
```

**Wide popup (450px):** ConversationList, MessageThread, NewMessage, MediaControls

**Default popup (360px):** DeviceList, DevicePage, Settings, SendTo

## Async Tasks

Use `Task::perform` with `cosmic::Action::App` wrapper:

```rust
cosmic::app::Task::perform(
    async { /* async work */ },
    |result| cosmic::Action::App(Message::from(result)),
)
```

## Popup Windows

Use standard COSMIC applet popup helpers:

```rust
use cosmic::iced::platform_specific::shell::wayland::commands::popup::{destroy_popup, get_popup};

Message::TogglePopup => {
    return if let Some(popup_id) = self.popup.take() {
        destroy_popup(popup_id)
    } else {
        let new_id = window::Id::unique();
        self.popup.replace(new_id);

        let popup_settings = self.core.applet.get_popup_settings(
            self.core.main_window_id().unwrap(),
            new_id,
            None, None, None,
        );

        get_popup(popup_settings)
    };
}

Message::PopupClosed(id) => {
    if self.popup == Some(id) {
        self.popup = None;
    }
}
```

**Important:** Use `destroy_popup()` and `get_popup()` helpers. Manual runtime actions cause issues where clicking panel icon to close prevents reopening.

## View Lifetimes

Use explicit lifetime annotations:

```rust
fn view(&self) -> Element<'_, Self::Message>
fn view_window(&self, _id: window::Id) -> Element<'_, Self::Message>
```

## Clickable List Item Pattern

Actions use clickable list item style:

```rust
// Navigation items (with chevron)
let row = row![
    icon::from_name("icon-name").size(24),
    text(fl!("label")).size(14),
    widget::horizontal_space(),
    icon::from_name("go-next-symbolic").size(16),  // Chevron
]
.spacing(12)
.align_y(Alignment::Center);

// Action items (no chevron)
let row = row![
    icon::from_name("icon-name").size(24),
    text(fl!("label")).size(14),
    widget::horizontal_space(),
]
.spacing(12)
.align_y(Alignment::Center);

// Button wrapper
widget::button::custom(
    widget::container(row).padding(8).width(Length::Fill),
)
.class(cosmic::theme::Button::Text)
.on_press(Message::SomeAction)
.width(Length::Fill)
```

**When to use chevrons:** Items that navigate to another view (SMS Messages, Media Controls). Omit for immediate actions (Share file, Send Ping, Find Phone).

## Device Page Layout

1. **Header** - Back button, device icon, name, type, status, battery
2. **Actions** (list items):
   - SMS Messages → ConversationList (chevron)
   - Send to [device] → SendTo (chevron)
   - Media Controls → MediaControls (chevron)
   - Find Phone → rings device (no chevron)
3. **Pairing section** - Pair/unpair buttons
4. **Notifications section** - Device notifications list

## fl!() Macro Lifetime Handling

`fl!()` returns owned `String`, not `&'static str`:

```rust
// Text widgets - pass directly
text(fl!("label"))
widget::button::standard(fl!("button-text"))

// text_input - pass directly without &
widget::text_input(fl!("placeholder"), &self.input_value)  // Correct
// widget::text_input(&fl!("placeholder"), &self.input_value)  // Won't compile!

// Fallback values - pre-compute
let default_name = fl!("unknown");
let name = self.device_name.as_deref().unwrap_or(&default_name);
```

## Configuration System

Uses `cosmic_config` for persistent settings:

```rust
// Location: ~/.config/cosmic/io.github.nwxnw.connected/v4/

// Load
let config = Config::load();

// Save
self.config.save()?;

// Watch for external changes
self.core.watch_config::<Config>(APP_ID)
    .map(|update| Message::ConfigChanged(update.config))
```
