pub mod line;
pub mod symbol;

use crate::error::AvengerGuidesError;

pub fn compute_encoding_length(lengths: &[usize]) -> Result<usize, AvengerGuidesError> {
    let lengths = lengths
        .into_iter()
        .cloned()
        .filter(|&len| len > 1)
        .collect::<std::collections::HashSet<_>>();

    let len = if lengths.len() > 1 {
        return Err(AvengerGuidesError::InvalidLegendLength(lengths));
    } else if lengths.len() == 0 {
        1
    } else {
        // Only one unique length greater than 1
        lengths.into_iter().next().unwrap_or(1)
    };

    Ok(len)
}
