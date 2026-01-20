# D-Bus Interface Reference

KDE Connect D-Bus interfaces and testing commands.

## Interface Reference

| Interface | Path | Purpose |
|-----------|------|---------|
| `org.kde.kdeconnect.daemon` | `/modules/kdeconnect` | Device discovery, announcements |
| `org.kde.kdeconnect.device` | `/modules/kdeconnect/devices/<id>` | Per-device operations, pairing |
| `org.kde.kdeconnect.device.battery` | (same + /battery) | Battery status (charge, isCharging) |
| `org.kde.kdeconnect.device.clipboard` | (same + /clipboard) | Clipboard sync |
| `org.kde.kdeconnect.device.findmyphone` | (same + /findmyphone) | Trigger phone to ring |
| `org.kde.kdeconnect.device.mprisremote` | (same + /mprisremote) | Media player control |
| `org.kde.kdeconnect.device.ping` | (same + /ping) | Send ping to device |
| `org.kde.kdeconnect.device.notifications` | (same + /notifications) | List active notifications |
| `org.kde.kdeconnect.device.share` | (same + /share) | File/URL sharing |
| `org.kde.kdeconnect.device.sms` | (same + /sms) | Request SMS conversations |
| `org.kde.kdeconnect.device.conversations` | `/modules/kdeconnect/devices/<id>` | SMS data and signals |
| `org.kde.kdeconnect.device.telephony` | (same + /telephony) | Call notifications |

## Property Naming Convention

KDE Connect uses camelCase for D-Bus property names. In zbus, explicitly specify names:

```rust
#[zbus(property, name = "isCharging")]
fn is_charging(&self) -> zbus::Result<bool>;

#[zbus(property, name = "isPairRequested")]
fn is_pair_requested(&self) -> zbus::Result<bool>;
```

## Signal Subscription

To receive real-time updates, subscribe to D-Bus signals using match rules:

```rust
use zbus::fdo::DBusProxy;

let dbus_proxy = DBusProxy::new(&conn).await?;
let rule = zbus::MatchRule::builder()
    .msg_type(zbus::message::Type::Signal)
    .sender("org.kde.kdeconnect.daemon")
    .map(|b| b.build())?;
dbus_proxy.add_match_rule(rule).await?;

let stream = zbus::MessageStream::from(&conn);
```

Without explicit match rules, D-Bus signals may not be delivered.

## Testing Commands

### Basic Operations

```bash
# List paired devices
dbus-send --session --print-reply \
  --dest=org.kde.kdeconnect.daemon \
  /modules/kdeconnect \
  org.kde.kdeconnect.daemon.devices

# Introspect daemon interface
dbus-send --session --print-reply \
  --dest=org.kde.kdeconnect.daemon \
  /modules/kdeconnect \
  org.freedesktop.DBus.Introspectable.Introspect

# Ping a device
dbus-send --session --print-reply \
  --dest=org.kde.kdeconnect.daemon \
  /modules/kdeconnect/devices/<device-id>/ping \
  org.kde.kdeconnect.device.ping.sendPing
```

### Device Operations

```bash
# Get device name
dbus-send --session --print-reply \
  --dest=org.kde.kdeconnect.daemon \
  /modules/kdeconnect/devices/<device-id> \
  org.freedesktop.DBus.Properties.Get \
  string:org.kde.kdeconnect.device string:name

# Check if device is reachable
dbus-send --session --print-reply \
  --dest=org.kde.kdeconnect.daemon \
  /modules/kdeconnect/devices/<device-id> \
  org.freedesktop.DBus.Properties.Get \
  string:org.kde.kdeconnect.device string:isReachable

# Request pairing
dbus-send --session --print-reply \
  --dest=org.kde.kdeconnect.daemon \
  /modules/kdeconnect/devices/<device-id> \
  org.kde.kdeconnect.device.requestPairing
```

### Battery Plugin

```bash
# Get battery charge level
dbus-send --session --print-reply \
  --dest=org.kde.kdeconnect.daemon \
  /modules/kdeconnect/devices/<device-id>/battery \
  org.freedesktop.DBus.Properties.Get \
  string:org.kde.kdeconnect.device.battery string:charge

# Check if charging
dbus-send --session --print-reply \
  --dest=org.kde.kdeconnect.daemon \
  /modules/kdeconnect/devices/<device-id>/battery \
  org.freedesktop.DBus.Properties.Get \
  string:org.kde.kdeconnect.device.battery string:isCharging
```

### Monitoring Signals

```bash
# Watch all KDE Connect signals
dbus-monitor --session "sender='org.kde.kdeconnect.daemon'"

# Watch specific device signals
dbus-monitor --session "path='/modules/kdeconnect/devices/<device-id>'"
```

### Using busctl

```bash
# List all KDE Connect objects
busctl --user tree org.kde.kdeconnect.daemon

# Introspect a device
busctl --user introspect org.kde.kdeconnect.daemon \
  /modules/kdeconnect/devices/<device-id>

# Call a method
busctl --user call org.kde.kdeconnect.daemon \
  /modules/kdeconnect/devices/<device-id>/ping \
  org.kde.kdeconnect.device.ping sendPing
```
