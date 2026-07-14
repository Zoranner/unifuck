//! Tuanjie Hub patch strategy.

pub(crate) mod strategy;

use crate::hub::asar::AsarArchive;
use crate::hub::strategy::PatchStrategy;

/// Detect the Tuanjie Hub format and return the appropriate patch strategy.
///
/// Currently there is only a single Tuanjie variant. We still call its
/// `applies_to` check to bail loudly when the archive does not look like a
/// Tuanjie Hub.
pub(crate) fn detect_strategy(asar: &AsarArchive) -> anyhow::Result<Box<dyn PatchStrategy>> {
    let strategy = strategy::TuanjieHubStrategy;
    if strategy.applies_to(asar) {
        return Ok(Box::new(strategy));
    }
    anyhow::bail!("Unsupported Tuanjie Hub format")
}
