pub mod colorbar;
pub mod line;
pub mod symbol;

use crate::error::AvengerGuidesError;

fn compute_encoding_length(lengths: &[usize]) -> Result<usize, AvengerGuidesError> {
    let lengths = lengths
        .iter()
        .cloned()
        .filter(|&len| len > 1)
        .collect::<std::collections::HashSet<_>>();

    let len = if lengths.len() > 1 {
        return Err(AvengerGuidesError::InvalidLegendLength(lengths));
    } else if lengths.is_empty() {
        1
    } else {
        // Only one unique length greater than 1
        lengths.into_iter().next().unwrap_or(1)
    };

    Ok(len)
}
