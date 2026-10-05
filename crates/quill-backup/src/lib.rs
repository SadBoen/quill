#![forbid(unsafe_code)]
#![deny(missing_debug_implementations)]

pub mod backup;
pub mod digest;
pub mod error;
pub mod manifest;

pub use backup::{create_backup, restore_backup, BackupReport, BackupSource, RestoreReport};
pub use digest::{is_digest_hex, sha256_bytes, sha256_file, DIGEST_HEX_LEN};
pub use error::BackupError;
pub use manifest::{ExcludedEntry, Manifest, ManifestEntry, MANIFEST_NAME, MANIFEST_VERSION};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crate_compiles_and_test_runs() {
        assert_eq!(2 + 2, 4);
    }

    #[test]
    fn public_surface_is_reachable() {
        assert_eq!(MANIFEST_NAME, "MANIFEST");
        assert_eq!(MANIFEST_VERSION, "v1");
        assert!(is_digest_hex(&sha256_bytes(b"quill")));
    }
}
