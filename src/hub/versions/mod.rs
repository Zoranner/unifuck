pub(crate) mod commonjs;
pub(crate) mod esm;

use crate::hub::asar::AsarArchive;

/// Trait for version-specific patch strategies.
///
/// Each Hub version (or version range) implements this trait to define:
/// - Which files to patch
/// - How to patch each file
/// - Version detection logic
pub(crate) trait PatchStrategy: Send + Sync {
    /// Human-readable version name for logging.
    fn version_name(&self) -> &'static str;

    /// Check if this strategy applies to the given asar archive.
    fn applies_to(&self, asar: &AsarArchive) -> bool;

    /// Apply patches to the asar archive.
    /// Returns a list of (file_path, patched_content) pairs.
    fn apply_patches(&self, asar: &AsarArchive) -> anyhow::Result<Vec<(String, Vec<u8>)>>;
}

/// Detect Hub version and return appropriate patch strategy.
pub(crate) fn detect_strategy(asar: &AsarArchive) -> anyhow::Result<Box<dyn PatchStrategy>> {
    // Try ESM bundled format first (app-*.js files)
    let esm_strategy = esm::EsmStrategy;
    if esm_strategy.applies_to(asar) {
        return Ok(Box::new(esm_strategy));
    }

    // Try CommonJS separate files format
    let commonjs_strategy = commonjs::CommonJsStrategy;
    if commonjs_strategy.applies_to(asar) {
        return Ok(Box::new(commonjs_strategy));
    }

    anyhow::bail!("Unsupported Hub version. Could not detect patch strategy.")
}
