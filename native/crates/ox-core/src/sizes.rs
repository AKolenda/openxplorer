// SPDX-License-Identifier: AGPL-3.0-only
//! On-demand folder sizes: a bounded, read-only scan that totals the
//! logical size of the files below a folder.
//!
//! Ports `v2.0.0:desktop/folder_sizes.py` (`scan_folder`, `LocalSizeProvider` and
//! `GioSizeProvider`) and the mount-point part of
//! `v2.0.0:desktop/mount_support.py`. A scan reads metadata only, never file
//! contents, and is separate from the filename search index.
//!
//! The rules this module keeps:
//!
//! - PROP-028: links are never followed, special files never opened, and
//!   nested mounts, other filesystems and snapshot collections never
//!   entered. Hard links count once. Anything left out or unreadable makes
//!   the result partial; an unknown size is never counted as zero.
//! - PROP-029: a scan stops as partial after [`MAX_ENTRIES`] items or
//!   [`MAX_DURATION`], a scan cancelled after its folder was read returns
//!   what it counted, and an error on the scanned folder itself reaches
//!   the caller so that an unmounted share can be mounted and scanned
//!   again.
//! - PROP-030: network shares are read through GIO metadata only.
//!
//! | Module | Responsibility |
//! |---|---|
//! | `scan` | [`FolderSizeScan`]: the walk, its limits and progress |
//! | `run` | [`scan_folder_size`] and its background variant |
//! | `result` | [`FolderSize`], [`ScanStatus`] and [`PartialReason`] |
//! | `provider` | [`SizeProvider`] and the [`SizeEntry`] it reads |
//! | `local_provider` | [`LocalSizeProvider`], for `file://` folders |
//! | `gio_provider` | [`GioSizeProvider`], for shares and other GIO locations |
//! | `mounts` | The mount points of this process |
//! | `error` | [`SizeError`] |

mod error;
mod gio_provider;
mod local_provider;
mod mounts;
mod provider;
mod result;
mod run;
mod scan;

pub use error::SizeError;
pub use gio_provider::GioSizeProvider;
pub use local_provider::LocalSizeProvider;
pub use provider::{FileIdentity, SizeEntry, SizeEntryKind, SizeProvider};
pub use result::{FolderSize, PartialReason, ScanStatus};
pub use run::{scan_folder_size, scan_folder_size_in_background};
pub use scan::{FolderSizeScan, ScanLimits, MAX_DURATION, MAX_ENTRIES};
