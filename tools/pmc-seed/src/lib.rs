//! The development command's library face: the sample workspace's seed now
//! lives in `pmc_application::sample_workspace` (item ⑨), where the desktop
//! host can call it. Re-exported so the command and its tests keep one name.

#![forbid(unsafe_code)]

pub use pmc_application::sample_workspace::*;
