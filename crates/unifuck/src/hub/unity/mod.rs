//! Unity Hub patch strategies (ESM bundled and CommonJS separate files).

pub(crate) mod commonjs;
pub(crate) mod esm;
pub(crate) mod vite;

use crate::hub::asar::AsarArchive;
use crate::hub::strategy::PatchStrategy;

/// Detect the Unity Hub format and return the appropriate patch strategy.
///
/// Tries ESM (most specific marker: `build/main/app-*.js` bundle) first,
/// then falls back to CommonJS (separate service files).
pub(crate) fn detect_strategy(asar: &AsarArchive) -> anyhow::Result<Box<dyn PatchStrategy>> {
    let esm_strategy = esm::EsmStrategy;
    if esm_strategy.applies_to(asar) {
        return Ok(Box::new(esm_strategy));
    }

    let commonjs_strategy = commonjs::CommonJsStrategy;
    if commonjs_strategy.applies_to(asar) {
        return Ok(Box::new(commonjs_strategy));
    }

    let vite_strategy = vite::ViteStrategy;
    if vite_strategy.applies_to(asar) {
        return Ok(Box::new(vite_strategy));
    }

    anyhow::bail!("Unsupported Unity Hub format")
}
