//! Device-related operations for KDE Connect devices.

pub mod actions;
pub mod class;
pub mod fetch;

pub use actions::*;
pub use class::DeviceClass;
pub use fetch::*;
