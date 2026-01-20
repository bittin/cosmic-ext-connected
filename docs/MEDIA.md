# Media Controls Implementation

Details on media player remote control functionality.

## D-Bus Interface

The MPRIS Remote plugin uses a single `sendAction` method for all playback controls:

```rust
// Correct - use sendAction with action name
proxy.send_action("PlayPause").await?;
proxy.send_action("Next").await?;
proxy.send_action("Previous").await?;

// Incorrect - these methods don't exist
// proxy.play_pause().await?;  // Won't work
// proxy.next().await?;        // Won't work
```

Valid action strings: `"Play"`, `"Pause"`, `"PlayPause"`, `"Stop"`, `"Next"`, `"Previous"`

## Available Properties

### Readable Properties

- `playerList` - List of available media players on device
- `player` - Currently selected player name
- `isPlaying` - Whether playback is active
- `volume` - Current volume (0-100), type: `int32`
- `length` - Track length in milliseconds, type: `int32`
- `position` - Current playback position in milliseconds, type: `int32`
- `title`, `artist`, `album` - Current track metadata
- `canSeek` - Whether player supports seeking

### Writable Properties

- `volume` - Set playback volume
- `position` - Seek to position
- `player` - Select active player

### zbus Property Naming

All properties need explicit `name = "..."` attributes:

```rust
#[zbus(property, name = "volume")]
fn volume(&self) -> zbus::Result<i32>;

#[zbus(property, name = "length")]
fn length(&self) -> zbus::Result<i32>;  // D-Bus returns int32, not int64
```

### Per-Player Properties Not Exposed

`canGoNext`, `canGoPrevious`, `canPlay`, `canPause` are per-player properties not on the main interface. UI defaults these to `true` and lets phone handle unsupported actions.

## Player Selection Persistence

User's player selection must be explicitly applied before each action. The D-Bus `sendAction` operates on whatever player the daemon considers "current", which may not match the user's selection.

### Solution

Track selection locally (`media_selected_player`) and call `set_player()` before every action:

```rust
async fn media_action_async(
    conn: Arc<Mutex<Connection>>,
    device_id: String,
    action: MediaAction,
    ensure_player: Option<String>,  // User's selected player
) -> Message {
    if let Some(ref player) = ensure_player {
        proxy.set_player(player).await.ok();  // Ensure correct player
    }
    proxy.send_action("PlayPause").await?;
}
```

## Future Enhancements

- Album art display (KDE Connect supports binary payload)
- Seek slider for playback position
- Loop and shuffle toggles
