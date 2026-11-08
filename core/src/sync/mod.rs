//! Orttaai Link: this PC syncing with Orttaai on a Mac, and through it with
//! iCloud and Orttakey (`docs/orttaai-link.md`).
//!
//! - [`model`]: the records every app shares (a port of `OrttaaiSyncCore`).
//! - [`wire`]: how they travel, and the request bodies.
//! - [`store`]: the sync ledger in this PC's database.
//! - [`client`]: HTTPS that trusts only the linked Mac's key.
//! - [`discovery`]: finding the Mac on the home network.
//! - [`engine`]: linking, and syncing in the background.

pub mod client;
pub mod discovery;
pub mod engine;
pub mod model;
pub mod store;
pub mod wire;

pub use discovery::{discover, DiscoveredMac};
pub use engine::{link_with_code, link_with_invite, LinkEngine, LinkError, LinkStatus};
pub use store::{ApplyResult, DeviceInfo, LinkedMac, SyncCounts};
