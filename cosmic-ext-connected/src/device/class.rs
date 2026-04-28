//! Device classification by form factor.
//!
//! Derived from KDE Connect's `deviceType` D-Bus property. Used to route
//! class-specific UI (action list, icon, localized caption).

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeviceClass {
    Phone,
    Tablet,
    Desktop,
    Laptop,
    Tv,
    Unknown,
}

impl DeviceClass {
    /// Classify a KDE Connect `deviceType` string.
    ///
    /// KDE Connect's wire values: `"smartphone"` or `"phone"`, `"tablet"`,
    /// `"desktop"`, `"laptop"`, `"tv"` (see reference
    /// `deviceinfo.h::DeviceType::FromString`).
    pub fn from_device_type(device_type: &str) -> Self {
        match device_type {
            "smartphone" | "phone" => Self::Phone,
            "tablet" => Self::Tablet,
            "desktop" => Self::Desktop,
            "laptop" => Self::Laptop,
            "tv" => Self::Tv,
            _ => Self::Unknown,
        }
    }

    /// Freedesktop icon name for this device class.
    ///
    /// Phone (and Unknown) use the applet's custom phone icon so the device
    /// list stays visually consistent with the panel icon. Tablet uses the
    /// freedesktop `tablet-symbolic` slab. Desktop and Laptop collapse to
    /// `computer-symbolic` — KDE Connect's own Linux daemon doesn't reliably
    /// distinguish the two, and the Cosmic theme provides a single icon.
    pub fn icon_name(self) -> &'static str {
        match self {
            Self::Phone | Self::Unknown => "io.github.nwxnw.cosmic-ext-connected-symbolic",
            Self::Tablet => "tablet-symbolic",
            Self::Desktop | Self::Laptop => "computer-symbolic",
            Self::Tv => "tv-symbolic",
        }
    }

    /// Whether this class is a phone or tablet.
    pub fn is_mobile(self) -> bool {
        matches!(self, Self::Phone | Self::Tablet)
    }
}
