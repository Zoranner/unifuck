use crate::hub::asar::AsarArchive;

/// Trait for vendor- and version-specific patch strategies.
///
/// Each Hub format (Unity ESM, Unity CommonJS, Tuanjie, ...) implements this
/// trait to declare which files to patch and how to patch them.
pub(crate) trait PatchStrategy: Send + Sync {
    /// Human-readable version name for logging.
    fn version_name(&self) -> &'static str;

    /// Check if this strategy applies to the given asar archive.
    fn applies_to(&self, asar: &AsarArchive) -> bool;

    /// Apply patches to the asar archive.
    /// Returns a list of (file_path, patched_content) pairs.
    fn apply_patches(&self, asar: &AsarArchive) -> anyhow::Result<Vec<(String, Vec<u8>)>>;
}
