//! Centralized constants for timeouts, intervals, and limits.
//!
//! This module provides a single location for all tunable values used
//! throughout the applet, making them easy to discover and adjust.

/// D-Bus connection and signal handling constants.
pub mod dbus {
    /// Delay before retrying D-Bus connection after failure (seconds).
    pub const RETRY_DELAY_SECS: u64 = 5;

    /// Debounce interval for device refresh after D-Bus signals (seconds).
    /// Prevents rapid refreshes when multiple signals arrive in quick succession.
    pub const SIGNAL_REFRESH_DEBOUNCE_SECS: u64 = 3;
}

/// SMS conversation and message loading constants.
pub mod sms {
    /// Timeout for conversation loading when cache exists (seconds).
    /// Shorter since we only need incremental updates.
    pub const CONVERSATION_TIMEOUT_CACHED_SECS: u64 = 3;

    /// Timeout for conversation loading on initial load (seconds).
    /// Longer to allow phone time to send all data.
    pub const CONVERSATION_TIMEOUT_INITIAL_SECS: u64 = 15;

    /// Activity timeout - stop collecting if no signals received (milliseconds).
    /// After receiving data, we stop waiting this long after the last signal.
    pub const SIGNAL_ACTIVITY_TIMEOUT_MS: u64 = 500;

    /// Interval for checking timeout conditions during signal collection (milliseconds).
    pub const TIMEOUT_CHECK_INTERVAL_MS: u64 = 50;

    /// Timeout for draining remaining buffered signals (milliseconds).
    pub const SIGNAL_DRAIN_TIMEOUT_MS: u64 = 5;

    /// Timeout for loading messages in a conversation thread (seconds).
    pub const MESSAGE_FETCH_TIMEOUT_SECS: u64 = 10;

    /// Hard timeout for message loading subscription (seconds).
    /// Safety net if conversationLoaded signal never arrives.
    pub const MESSAGE_SUBSCRIPTION_TIMEOUT_SECS: u64 = 20;

    /// Activity timeout after conversationLoaded for phone response data (milliseconds).
    /// After the local store read completes (conversationLoaded signal), we keep listening
    /// this long for additional messages from the phone. Resets on each D-Bus signal.
    /// Needed because the local store may be empty/sparse after a reboot.
    pub const PHONE_RESPONSE_TIMEOUT_MS: u64 = 8000;

    /// How long to wait for the phone to start responding with conversation
    /// list signals on cold start when no cache exists (milliseconds).
    pub const CONVERSATION_LIST_PHONE_WAIT_MS: u64 = 8000;

    /// Activity timeout for conversation list sync (milliseconds).
    /// After the first live signal, if no new signals arrive within this
    /// duration, sync is complete. Longer than SIGNAL_ACTIVITY_TIMEOUT_MS
    /// (500ms) because conversation list signals arrive with larger gaps.
    pub const CONVERSATION_LIST_ACTIVITY_TIMEOUT_MS: u64 = 3000;

    /// Interval for polling in fallback mode (milliseconds).
    pub const FALLBACK_POLLING_INTERVAL_MS: u64 = 500;

    /// Polling delays for fallback conversation loading (milliseconds).
    /// We poll multiple times with increasing delays to give the phone time to sync.
    pub const FALLBACK_POLLING_DELAYS_MS: &[u64] = &[500, 1000, 1500, 2000, 3000];

}

/// Refresh and polling interval constants.
pub mod refresh {
    /// Delay after sending SMS before refreshing the thread (seconds).
    pub const POST_SEND_DELAY_SECS: u64 = 2;

    /// Interval for refreshing media player state (seconds).
    pub const MEDIA_INTERVAL_SECS: u64 = 2;
}

/// Notification display constants.
pub mod notifications {
    /// Default notification timeout (seconds).
    pub const DEFAULT_TIMEOUT_SECS: u32 = 5;
    /// Minimum notification timeout slider value (seconds).
    pub const MIN_TIMEOUT_SECS: u32 = 1;
    /// Maximum notification timeout slider value (seconds).
    pub const MAX_TIMEOUT_SECS: u32 = 30;
}
