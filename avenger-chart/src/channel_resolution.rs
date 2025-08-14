use crate::marks::ChannelValue;
use datafusion::logical_expr::Expr;
use datafusion_common::tree_node::{Transformed, TransformedResult, TreeNode};
use indexmap::IndexMap;

/// Resolve channel references in an expression
///
/// Replaces column references like ":x" with the actual expression
/// from the corresponding channel.
pub fn resolve_channel_refs(expr: Expr, channels: &IndexMap<String, ChannelValue>) -> Expr {
    let original = expr.clone();
    let result = expr.transform(&|e| {
        match &e {
            Expr::Column(c) if c.name.starts_with(':') => {
                // This is a channel reference like ":x"
                let channel_name = &c.name[1..]; // Remove the ":"

                // Look up the channel
                if let Some(channel_value) = channels.get(channel_name) {
                    // Recursively resolve in case the channel itself has references
                    let resolved = resolve_channel_refs(channel_value.expr().clone(), channels);
                    Ok(Transformed::yes(resolved))
                } else {
                    // Channel not found, keep as-is (will error later)
                    Ok(Transformed::no(e))
                }
            }
            _ => Ok(Transformed::no(e)),
        }
    });

    // Extract the transformed expression or return the original on error
    result.data().unwrap_or(original)
}

/// Resolve all channel references in a mark's channels
pub fn resolve_all_channel_refs(
    channels: &IndexMap<String, ChannelValue>,
) -> IndexMap<String, ChannelValue> {
    let mut resolved = IndexMap::new();

    // First pass: collect all channels without resolution
    // This ensures we have all channels available for resolution
    for (name, value) in channels {
        resolved.insert(name.clone(), value.clone());
    }

    // Debug: print channels only if DEBUG_CHANNEL_REFS is set
    if std::env::var("DEBUG_CHANNEL_REFS").is_ok() {
        eprintln!("Channel references debug - input channels:");
        for (name, value) in &resolved {
            eprintln!("  {}: {:?}", name, value.expr());
        }
    }

    // Second pass: resolve references in each channel
    let mut final_channels = IndexMap::new();
    for (name, value) in &resolved {
        let resolved_expr = resolve_channel_refs(value.expr().clone(), &resolved);

        if std::env::var("DEBUG_CHANNEL_REFS").is_ok() {
            eprintln!(
                "  {} resolved: {:?} -> {:?}",
                name,
                value.expr(),
                resolved_expr
            );
        }

        // Preserve the channel value structure (Scaled vs Identity)
        let resolved_value = match value {
            ChannelValue::Scaled {
                scale_name, band, ..
            } => ChannelValue::Scaled {
                expr: resolved_expr,
                scale_name: scale_name.clone(),
                band: *band,
            },
            ChannelValue::Identity { .. } => ChannelValue::Identity {
                expr: resolved_expr,
            },
        };

        final_channels.insert(name.clone(), resolved_value);
    }

    if std::env::var("DEBUG_CHANNEL_REFS").is_ok() {
        eprintln!("Channel references debug - output channels:");
        for (name, value) in &final_channels {
            eprintln!("  {}: {:?}", name, value.expr());
        }
    }

    final_channels
}

