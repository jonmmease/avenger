//! Channel reference resolution for mark encodings
//!
//! This module provides functionality to resolve references between channels in mark encodings.
//! Channels can reference other channels using the `:channel_name` syntax, allowing for
//! derived encodings and shared expressions.
//!
//! # Channel Reference Syntax
//!
//! Channel references use a colon prefix to indicate a reference to another channel:
//! - `:x` - references the x channel
//! - `:color` - references the color channel
//! - `:size` - references the size channel
//!
//! # Example
//!
//! ```no_run
//! use avenger_chart::channel::ChannelValue;
//! use avenger_chart::serialization::LogicalExprNodeExt;
//! use datafusion::prelude::*;
//! use datafusion_proto::protobuf::LogicalExprNode;
//! use indexmap::IndexMap;
//!
//! # fn example() -> Result<(), Box<dyn std::error::Error>> {
//! // Define channels where y2 references the y channel
//! let mut channels = IndexMap::new();
//! channels.insert("y".to_string(), ChannelValue::Scaled {
//!     expr: LogicalExprNode::from_expr(col("value")).expect("Failed to serialize expr"),
//!     scale_name: None,
//!     band: None,
//!     scale_config: None,
//!     legend_config: None,
//!     share_mode: None,
//! });
//! channels.insert("y2".to_string(), ChannelValue::Scaled {
//!     expr: LogicalExprNode::from_expr(col(":y") + lit(10.0)).expect("Failed to serialize expr"),  // References y channel
//!     scale_name: None,
//!     band: None,
//!     scale_config: None,
//!     legend_config: None,
//!     share_mode: None,
//! });
//!
//! // Resolve references (function would be imported from this module)
//! // let resolved = resolve_all_channel_refs(&channels)?;
//! // y2 now contains: col("value") + lit(10.0)
//! # Ok(())
//! # }
//! ```
//!
//! # Dependency Resolution
//!
//! The system automatically handles complex dependency chains using topological sorting:
//! - Detects and reports cyclic dependencies
//! - Validates that all referenced channels exist
//! - Resolves channels in dependency order
//!
//! # Error Handling
//!
//! The resolution process validates:
//! - **Self-references**: A channel cannot reference itself
//! - **Undefined channels**: All referenced channels must exist
//! - **Cyclic dependencies**: Circular reference chains are not allowed
//!
//! # Performance Considerations
//!
//! The resolution algorithm uses topological sorting with O(V + E) complexity where:
//! - V = number of channels
//! - E = number of channel references
//!
//! For typical visualizations with < 20 channels, this is very efficient.

use super::value::ChannelValue;
use datafusion::logical_expr::Expr;
use datafusion::prelude::SessionContext;
use datafusion_common::tree_node::{TransformedResult, TreeNode};
use datafusion_proto::protobuf::LogicalExprNode;
use indexmap::IndexMap;
use std::collections::{HashMap, HashSet, VecDeque};
use strsim::levenshtein;

/// Error types for channel resolution
#[derive(Debug, Clone)]
pub enum ChannelResolutionError {
    CyclicDependency {
        cycle: Vec<String>,
        all_channels: Vec<String>,
    },
    SelfReference {
        channel: String,
    },
    UndefinedChannel {
        channel: String,
        referenced_by: String,
        available_channels: Vec<String>,
    },
    ConditionalChannelReference {
        channel: String,
        referenced_by: String,
    },
}

impl std::fmt::Display for ChannelResolutionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ChannelResolutionError::CyclicDependency {
                cycle,
                all_channels,
            } => {
                write!(
                    f,
                    "Cyclic channel dependency detected: {}\n\n\
                     The following channels form a circular reference chain:\n  {}\n\n\
                     To fix this, ensure that channel references do not form a loop.\n\
                     Available channels: {}",
                    cycle.join(" → "),
                    cycle.join(" → "),
                    all_channels.join(", ")
                )
            }
            ChannelResolutionError::SelfReference { channel } => {
                write!(
                    f,
                    "Channel '{}' references itself.\n\n\
                     A channel cannot reference itself. Use a different channel name \
                     or reference a different source channel.",
                    channel
                )
            }
            ChannelResolutionError::UndefinedChannel {
                channel,
                referenced_by,
                available_channels,
            } => {
                let mut msg = format!(
                    "Channel '{}' references undefined channel ':{}'\n\n\
                     The expression col(\":{}\") uses a channel reference that doesn't exist.",
                    referenced_by, channel, channel
                );

                if !available_channels.is_empty() {
                    msg.push_str("\n\nAvailable channels:\n");
                    for ch in available_channels {
                        msg.push_str(&format!("  - {}\n", ch));
                    }

                    // Suggest similar channel names if any exist
                    let similar = find_similar_channel(channel, available_channels);
                    if let Some(suggestion) = similar {
                        msg.push_str(&format!("\nDid you mean ':{}' instead?", suggestion));
                    }
                } else {
                    msg.push_str("\n\nNo channels are currently defined.");
                }

                write!(f, "{}", msg)
            }
            ChannelResolutionError::ConditionalChannelReference {
                channel,
                referenced_by,
            } => {
                write!(
                    f,
                    "Cannot reference conditional channel '{}' from '{}'\n\n\
                     Channel '{}' uses conditional encoding (when_value/when_scaled), \
                     which cannot be referenced by other channels.\n\n\
                     To fix this:\n\
                     • Use a direct data expression instead of referencing '{}'\n\
                     • Extract the common expression to a variable\n\
                     • Consider using a non-conditional channel for the shared value",
                    channel, referenced_by, channel, channel
                )
            }
        }
    }
}

impl std::error::Error for ChannelResolutionError {}

/// Find the most similar channel name using Levenshtein edit distance
fn find_similar_channel(target: &str, channels: &[String]) -> Option<String> {
    // First check for exact case-insensitive match
    let target_lower = target.to_lowercase();
    for channel in channels {
        if channel.to_lowercase() == target_lower {
            return Some(channel.clone());
        }
    }

    // Find the best match using Levenshtein distance
    channels
        .iter()
        .map(|channel| {
            // Use case-insensitive comparison for better suggestions
            let distance = levenshtein(&target_lower, &channel.to_lowercase());
            (channel, distance)
        })
        .filter(|(_, dist)| {
            // Only suggest if edit distance is reasonable (max 3 edits or 40% of target length)
            let max_distance = std::cmp::max(3, target.len() * 2 / 5);
            *dist <= max_distance
        })
        .min_by_key(|(_, dist)| *dist)
        .map(|(channel, _)| channel.clone())
}

/// Extract channel references from an expression
///
/// Finds all column references that start with ':' which indicate
/// references to other channels (e.g., ":x" references the x channel)
fn extract_channel_refs(expr: &Expr) -> HashSet<String> {
    let mut refs = HashSet::new();

    // Use TreeNode's apply method for a clean traversal
    let _ = expr.apply(&mut |e: &Expr| {
        if let Expr::Column(c) = e {
            if c.name.starts_with(':') {
                refs.insert(c.name[1..].to_string());
            }
        }
        Ok(datafusion_common::tree_node::TreeNodeRecursion::Continue)
    });

    refs
}

/// Validate channel references for self-references and undefined channels
fn validate_channel_refs(
    channels: &IndexMap<String, ChannelValue>,
    ctx: &SessionContext,
) -> Result<(), ChannelResolutionError> {
    for (name, value) in channels {
        // Get all expressions from the channel value (handles conditionals too)
        let exprs = value.all_exprs(ctx);
        let mut all_refs = HashSet::new();

        for expr in exprs {
            let refs = extract_channel_refs(&expr);
            all_refs.extend(refs);
        }

        let refs = all_refs;

        // Check for self-reference
        if refs.contains(name) {
            return Err(ChannelResolutionError::SelfReference {
                channel: name.clone(),
            });
        }

        // Check for undefined channels and conditional references
        for ref_name in &refs {
            // Check for empty channel name (e.g., from col(":"))
            if ref_name.is_empty() {
                return Err(ChannelResolutionError::UndefinedChannel {
                    channel: "(empty)".to_string(),
                    referenced_by: name.clone(),
                    available_channels: channels.keys().cloned().collect(),
                });
            }

            if let Some(_referenced_channel) = channels.get(ref_name) {
                // Conditional channels are now allowed - they will resolve to their 'otherwise' expression
                // No validation error needed here
            } else {
                let available_channels: Vec<String> = channels.keys().cloned().collect();
                return Err(ChannelResolutionError::UndefinedChannel {
                    channel: ref_name.clone(),
                    referenced_by: name.clone(),
                    available_channels,
                });
            }
        }
    }

    Ok(())
}

/// Build dependency graph for channels
fn build_dependency_graph(
    channels: &IndexMap<String, ChannelValue>,
    ctx: &SessionContext,
) -> HashMap<String, HashSet<String>> {
    let mut graph = HashMap::new();

    for (name, value) in channels {
        // Get all expressions from the channel value (handles conditionals too)
        let exprs = value.all_exprs(ctx);
        let mut all_deps = HashSet::new();

        for expr in exprs {
            let deps = extract_channel_refs(&expr);
            all_deps.extend(deps);
        }

        graph.insert(name.clone(), all_deps);
    }

    graph
}

/// Perform topological sort on channels based on dependencies
/// Returns the channels in dependency order or an error if there's a cycle
fn topological_sort(
    channels: &IndexMap<String, ChannelValue>,
    ctx: &SessionContext,
) -> Result<Vec<String>, ChannelResolutionError> {
    let graph = build_dependency_graph(channels, ctx);
    let mut in_degree: HashMap<String, usize> = HashMap::new();
    let mut result = Vec::new();

    // Build reverse dependency map for O(1) lookups
    // This maps each channel to the channels that depend on it
    let mut reverse_deps: HashMap<String, Vec<String>> = HashMap::new();

    // Initialize in-degrees and build reverse dependency map
    for name in channels.keys() {
        in_degree.insert(name.clone(), 0);
        reverse_deps.insert(name.clone(), Vec::new());
    }

    // Calculate in-degrees and populate reverse dependency map
    for (channel, deps) in &graph {
        // deps are the channels that 'channel' depends on
        // So 'channel' has an in-degree equal to the number of its dependencies
        *in_degree.get_mut(channel).unwrap() = deps.len();

        // For each dependency, add 'channel' to its reverse deps
        for dep in deps {
            if let Some(rev_deps) = reverse_deps.get_mut(dep) {
                rev_deps.push(channel.clone());
            }
        }
    }

    // Find nodes with no dependencies (in-degree 0)
    let mut queue: VecDeque<String> = VecDeque::new();
    for (name, &degree) in &in_degree {
        if degree == 0 {
            queue.push_back(name.clone());
        }
    }

    // Process nodes
    while let Some(node) = queue.pop_front() {
        result.push(node.clone());

        // Use reverse dependency map for O(1) lookup of dependent channels
        if let Some(dependents) = reverse_deps.get(&node) {
            for dependent in dependents {
                if let Some(count) = in_degree.get_mut(dependent) {
                    *count -= 1;
                    if *count == 0 {
                        queue.push_back(dependent.clone());
                    }
                }
            }
        }
    }

    // Check if all nodes were processed (no cycle)
    if result.len() == channels.len() {
        Ok(result)
    } else {
        // Find a cycle for better error reporting
        let processed: HashSet<_> = result.into_iter().collect();
        let unprocessed: Vec<_> = channels
            .keys()
            .filter(|k| !processed.contains(*k))
            .cloned()
            .collect();

        // Try to find a simple cycle path for clearer error message
        let all_channels: Vec<String> = channels.keys().cloned().collect();
        if let Some(cycle) = find_cycle_path(&graph, &unprocessed) {
            Err(ChannelResolutionError::CyclicDependency {
                cycle,
                all_channels,
            })
        } else {
            // Fallback: just report the nodes involved in the cycle
            Err(ChannelResolutionError::CyclicDependency {
                cycle: unprocessed,
                all_channels,
            })
        }
    }
}

/// Find a cycle path starting from any of the given nodes
fn find_cycle_path(
    graph: &HashMap<String, HashSet<String>>,
    nodes: &[String],
) -> Option<Vec<String>> {
    for start_node in nodes {
        let mut visited = HashSet::new();
        let mut rec_stack = HashSet::new();
        let mut path = Vec::new();

        if find_cycle_dfs(graph, start_node, &mut visited, &mut rec_stack, &mut path) {
            // Find where the cycle starts in the path
            if let Some(last) = path.last() {
                if let Some(cycle_start) = path.iter().position(|n| n == last) {
                    let mut cycle: Vec<_> = path[cycle_start..].to_vec();
                    cycle.push(path[cycle_start].clone()); // Add first node again to show cycle
                    return Some(cycle);
                }
            }
        }
    }
    None
}

/// DFS helper to find cycles using recursion stack for directed graphs
fn find_cycle_dfs(
    graph: &HashMap<String, HashSet<String>>,
    node: &str,
    visited: &mut HashSet<String>,
    rec_stack: &mut HashSet<String>,
    path: &mut Vec<String>,
) -> bool {
    // If node is in the recursion stack, we found a cycle
    if rec_stack.contains(node) {
        path.push(node.to_string());
        return true;
    }

    // If already visited but not in recursion stack, no cycle on this path
    if visited.contains(node) {
        return false;
    }

    visited.insert(node.to_string());
    rec_stack.insert(node.to_string());
    path.push(node.to_string());

    if let Some(deps) = graph.get(node) {
        for dep in deps {
            if find_cycle_dfs(graph, dep, visited, rec_stack, path) {
                return true;
            }
        }
    }

    // Remove from recursion stack before returning
    rec_stack.remove(node);
    path.pop();
    false
}

/// Resolve channel references in an expression
///
/// Replaces column references like ":x" with the actual expression
/// from the corresponding channel.
pub fn resolve_channel_refs(
    expr: LogicalExprNode,
    channels: &IndexMap<String, ChannelValue>,
    ctx: &SessionContext,
) -> LogicalExprNode {
    use crate::serialization::LogicalExprNodeExt;
    use datafusion::common::tree_node::Transformed;

    // Convert to Expr for transformation
    let expr_value = match expr.to_expr(ctx) {
        Ok(e) => e,
        Err(_) => return expr, // Return original if conversion fails
    };

    let original = expr_value.clone();
    let resolved = expr_value
        .transform(&|e| {
            match &e {
                Expr::Column(c) if c.name.starts_with(':') => {
                    // This is a channel reference like ":x"
                    let channel_name = &c.name[1..]; // Remove the ":"

                    // Look up the channel
                    if let Some(channel_value) = channels.get(channel_name) {
                        // For conditional channels, extract the 'otherwise' expression
                        if let ChannelValue::Conditional { otherwise, .. } = channel_value {
                            // Extract the expression from the 'otherwise' ConditionalValue
                            let otherwise_node = match otherwise {
                                crate::channel::value::ConditionalValue::Scaled { expr } => expr,
                                crate::channel::value::ConditionalValue::Value { expr } => expr,
                            };
                            // Convert LogicalExprNode to Expr using to_expr
                            if let Ok(otherwise_datafusion_expr) = otherwise_node.to_expr(ctx) {
                                Ok(Transformed::yes(otherwise_datafusion_expr))
                            } else {
                                // Failed to convert, keep as-is
                                Ok(Transformed::no(e))
                            }
                        } else if let Some(resolved_expr) = channel_value.expr(ctx) {
                            // Non-conditional channel - use expr() method
                            Ok(Transformed::yes(resolved_expr))
                        } else {
                            // No expression available - keep as-is
                            Ok(Transformed::no(e))
                        }
                    } else {
                        // Channel not found, keep as-is (will error later)
                        Ok(Transformed::no(e))
                    }
                }
                _ => Ok(Transformed::no(e)),
            }
        })
        .data()
        .unwrap_or(original);

    // Convert back to LogicalExprNode
    LogicalExprNode::from_expr(resolved).unwrap_or(expr)
}

/// Resolve all channel references in a mark's channels,
/// replacing channel references (e.g. `:x`) with the actual expressions.
///
/// Returns the resolved channels or an error if resolution fails
pub fn resolve_all_channel_refs(
    channels: &IndexMap<String, ChannelValue>,
    ctx: &SessionContext,
) -> Result<IndexMap<String, ChannelValue>, ChannelResolutionError> {
    // First validate all channel references
    validate_channel_refs(channels, ctx)?;

    // Get topological order
    let order = topological_sort(channels, ctx)?;

    // Resolve channels in topological order
    let mut resolved_channels = IndexMap::new();

    for name in &order {
        if let Some(value) = channels.get(name) {
            // Preserve the channel value structure (Scaled vs Identity vs Conditional)
            let resolved_value = match value {
                ChannelValue::Scaled {
                    expr,
                    scale_name,
                    band,
                    scale_config,
                    legend_config,
                    ..
                } => {
                    let resolved_expr = resolve_channel_refs(expr.clone(), &resolved_channels, ctx);
                    ChannelValue::Scaled {
                        expr: resolved_expr,
                        scale_name: scale_name.clone(),
                        band: *band,
                        scale_config: scale_config.clone(),
                        legend_config: legend_config.clone(),
                        share_mode: None,
                    }
                }
                ChannelValue::Value { expr } => {
                    let resolved_expr = resolve_channel_refs(expr.clone(), &resolved_channels, ctx);
                    ChannelValue::Value {
                        expr: resolved_expr,
                    }
                }
                ChannelValue::Conditional {
                    conditions,
                    otherwise,
                    scale_config,
                    legend_config,
                    ..
                } => {
                    // Resolve channel references in conditions and otherwise
                    use crate::channel::ConditionalValue;

                    let resolved_conditions = conditions
                        .iter()
                        .map(|(test, value)| {
                            let resolved_test =
                                resolve_channel_refs(test.clone(), &resolved_channels, ctx);
                            let resolved_value = match value {
                                ConditionalValue::Scaled { expr } => ConditionalValue::Scaled {
                                    expr: resolve_channel_refs(
                                        expr.clone(),
                                        &resolved_channels,
                                        ctx,
                                    ),
                                },
                                ConditionalValue::Value { expr } => ConditionalValue::Value {
                                    expr: resolve_channel_refs(
                                        expr.clone(),
                                        &resolved_channels,
                                        ctx,
                                    ),
                                },
                            };
                            (resolved_test, resolved_value)
                        })
                        .collect();

                    let resolved_otherwise = match otherwise {
                        ConditionalValue::Scaled { expr } => ConditionalValue::Scaled {
                            expr: resolve_channel_refs(expr.clone(), &resolved_channels, ctx),
                        },
                        ConditionalValue::Value { expr } => ConditionalValue::Value {
                            expr: resolve_channel_refs(expr.clone(), &resolved_channels, ctx),
                        },
                    };

                    ChannelValue::Conditional {
                        conditions: resolved_conditions,
                        otherwise: resolved_otherwise,
                        scale_config: scale_config.clone(),
                        legend_config: legend_config.clone(),
                        share_mode: None,
                    }
                }
            };

            resolved_channels.insert(name.clone(), resolved_value);
        }
    }

    Ok(resolved_channels)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::serialization::LogicalExprNodeExt;
    use datafusion::logical_expr::{col, lit};
    use datafusion::prelude::SessionContext;
    use datafusion_proto::protobuf::LogicalExprNode;

    #[test]
    fn test_simple_channel_reference() {
        let ctx = SessionContext::new();
        let mut channels = IndexMap::new();
        channels.insert(
            "x".to_string(),
            ChannelValue::Scaled {
                expr: LogicalExprNode::from_expr(col("value")).expect("Failed to serialize expr"),
                scale_name: None,
                band: None,
                scale_config: None,
                legend_config: None,
                share_mode: None,
            },
        );
        channels.insert(
            "x2".to_string(),
            ChannelValue::Scaled {
                expr: LogicalExprNode::from_expr(col(":x") + lit(10.0))
                    .expect("Failed to serialize expr"),
                scale_name: None,
                band: None,
                scale_config: None,
                legend_config: None,
                share_mode: None,
            },
        );

        let resolved = resolve_all_channel_refs(&channels, &ctx).unwrap();

        // x should remain unchanged
        assert_eq!(
            resolved.get("x").unwrap().expr(&ctx).unwrap().to_string(),
            "value"
        );

        // x2 should have :x replaced with col("value")
        assert_eq!(
            resolved.get("x2").unwrap().expr(&ctx).unwrap().to_string(),
            "value + Float64(10)"
        );
    }

    #[test]
    fn test_chain_references() {
        let ctx = SessionContext::new();
        let mut channels = IndexMap::new();
        channels.insert(
            "a".to_string(),
            ChannelValue::Scaled {
                expr: LogicalExprNode::from_expr(col("base")).expect("Failed to serialize expr"),
                scale_name: None,
                band: None,
                scale_config: None,
                legend_config: None,
                share_mode: None,
            },
        );
        channels.insert(
            "b".to_string(),
            ChannelValue::Scaled {
                expr: LogicalExprNode::from_expr(col(":a") * lit(2.0))
                    .expect("Failed to serialize expr"),
                scale_name: None,
                band: None,
                scale_config: None,
                legend_config: None,
                share_mode: None,
            },
        );
        channels.insert(
            "c".to_string(),
            ChannelValue::Scaled {
                expr: LogicalExprNode::from_expr(col(":b") + lit(5.0))
                    .expect("Failed to serialize expr"),
                scale_name: None,
                band: None,
                scale_config: None,
                legend_config: None,
                share_mode: None,
            },
        );

        let resolved = resolve_all_channel_refs(&channels, &ctx).unwrap();

        // a should remain unchanged
        assert_eq!(
            resolved.get("a").unwrap().expr(&ctx).unwrap().to_string(),
            "base"
        );

        // b should have :a replaced
        assert_eq!(
            resolved.get("b").unwrap().expr(&ctx).unwrap().to_string(),
            "base * Float64(2)"
        );

        // c should have :b fully resolved
        assert_eq!(
            resolved.get("c").unwrap().expr(&ctx).unwrap().to_string(),
            "base * Float64(2) + Float64(5)"
        );
    }

    #[test]
    fn test_self_reference_error() {
        let ctx = SessionContext::new();
        let mut channels = IndexMap::new();
        channels.insert(
            "x".to_string(),
            ChannelValue::Scaled {
                expr: LogicalExprNode::from_expr(col(":x") + lit(1.0))
                    .expect("Failed to serialize expr"),
                scale_name: None,
                band: None,
                scale_config: None,
                legend_config: None,
                share_mode: None,
            },
        );

        let result = resolve_all_channel_refs(&channels, &ctx);
        assert!(result.is_err());

        match result.unwrap_err() {
            ChannelResolutionError::SelfReference { channel } => {
                assert_eq!(channel, "x");
            }
            _ => panic!("Expected SelfReference error"),
        }
    }

    #[test]
    fn test_cyclic_dependency_error() {
        let ctx = SessionContext::new();
        let mut channels = IndexMap::new();
        channels.insert(
            "x".to_string(),
            ChannelValue::Scaled {
                expr: LogicalExprNode::from_expr(col(":y")).expect("Failed to serialize expr"),
                scale_name: None,
                band: None,
                scale_config: None,
                legend_config: None,
                share_mode: None,
            },
        );
        channels.insert(
            "y".to_string(),
            ChannelValue::Scaled {
                expr: LogicalExprNode::from_expr(col(":x")).expect("Failed to serialize expr"),
                scale_name: None,
                band: None,
                scale_config: None,
                legend_config: None,
                share_mode: None,
            },
        );

        let result = resolve_all_channel_refs(&channels, &ctx);
        assert!(result.is_err());

        match result.unwrap_err() {
            ChannelResolutionError::CyclicDependency {
                cycle,
                all_channels,
            } => {
                // Should detect the cycle
                assert!(cycle.len() >= 2);
                assert_eq!(all_channels.len(), 2);
                assert!(all_channels.contains(&"x".to_string()));
                assert!(all_channels.contains(&"y".to_string()));
            }
            _ => panic!("Expected CyclicDependency error"),
        }
    }

    #[test]
    fn test_undefined_channel_error() {
        let ctx = SessionContext::new();
        let mut channels = IndexMap::new();
        channels.insert(
            "x".to_string(),
            ChannelValue::Scaled {
                expr: LogicalExprNode::from_expr(col("value")).expect("Failed to serialize expr"),
                scale_name: None,
                band: None,
                scale_config: None,
                legend_config: None,
                share_mode: None,
            },
        );
        channels.insert(
            "y".to_string(),
            ChannelValue::Scaled {
                expr: LogicalExprNode::from_expr(col(":bogus")).expect("Failed to serialize expr"),
                scale_name: None,
                band: None,
                scale_config: None,
                legend_config: None,
                share_mode: None,
            },
        );

        let result = resolve_all_channel_refs(&channels, &ctx);
        assert!(result.is_err());

        match result.unwrap_err() {
            ChannelResolutionError::UndefinedChannel {
                channel,
                referenced_by,
                available_channels,
            } => {
                assert_eq!(channel, "bogus");
                assert_eq!(referenced_by, "y");
                assert_eq!(available_channels.len(), 2);
                assert!(available_channels.contains(&"x".to_string()));
                assert!(available_channels.contains(&"y".to_string()));
            }
            _ => panic!("Expected UndefinedChannel error"),
        }
    }

    #[test]
    fn test_multiple_references_in_expression() {
        let ctx = SessionContext::new();
        let mut channels = IndexMap::new();
        channels.insert(
            "x".to_string(),
            ChannelValue::Scaled {
                expr: LogicalExprNode::from_expr(col("a")).expect("Failed to serialize expr"),
                scale_name: None,
                band: None,
                scale_config: None,
                legend_config: None,
                share_mode: None,
            },
        );
        channels.insert(
            "y".to_string(),
            ChannelValue::Scaled {
                expr: LogicalExprNode::from_expr(col("b")).expect("Failed to serialize expr"),
                scale_name: None,
                band: None,
                scale_config: None,
                legend_config: None,
                share_mode: None,
            },
        );
        channels.insert(
            "z".to_string(),
            ChannelValue::Scaled {
                expr: LogicalExprNode::from_expr(col(":x") + col(":y"))
                    .expect("Failed to serialize expr"),
                scale_name: None,
                band: None,
                scale_config: None,
                legend_config: None,
                share_mode: None,
            },
        );

        let resolved = resolve_all_channel_refs(&channels, &ctx).unwrap();

        // z should have both :x and :y resolved
        assert_eq!(
            resolved.get("z").unwrap().expr(&ctx).unwrap().to_string(),
            "a + b"
        );
    }

    #[test]
    fn test_identity_channel_preservation() {
        let ctx = SessionContext::new();
        let mut channels = IndexMap::new();
        channels.insert(
            "x".to_string(),
            ChannelValue::Value {
                expr: LogicalExprNode::from_expr(col("value")).expect("Failed to serialize expr"),
            },
        );
        channels.insert(
            "y".to_string(),
            ChannelValue::Value {
                expr: LogicalExprNode::from_expr(col(":x") * lit(2.0))
                    .expect("Failed to serialize expr"),
            },
        );

        let resolved = resolve_all_channel_refs(&channels, &ctx).unwrap();

        // Both should remain Identity variants
        assert!(matches!(
            resolved.get("x").unwrap(),
            ChannelValue::Value { .. }
        ));
        assert!(matches!(
            resolved.get("y").unwrap(),
            ChannelValue::Value { .. }
        ));

        // y should have :x resolved
        assert_eq!(
            resolved.get("y").unwrap().expr(&ctx).unwrap().to_string(),
            "value * Float64(2)"
        );
    }

    #[test]
    fn test_complex_dependency_graph() {
        let ctx = SessionContext::new();
        let mut channels = IndexMap::new();
        // Create a diamond dependency pattern
        //     a
        //    / \
        //   b   c
        //    \ /
        //     d
        channels.insert(
            "a".to_string(),
            ChannelValue::Scaled {
                expr: LogicalExprNode::from_expr(col("base")).expect("Failed to serialize expr"),
                scale_name: None,
                band: None,
                scale_config: None,
                legend_config: None,
                share_mode: None,
            },
        );
        channels.insert(
            "b".to_string(),
            ChannelValue::Scaled {
                expr: LogicalExprNode::from_expr(col(":a") * lit(2.0))
                    .expect("Failed to serialize expr"),
                scale_name: None,
                band: None,
                scale_config: None,
                legend_config: None,
                share_mode: None,
            },
        );
        channels.insert(
            "c".to_string(),
            ChannelValue::Scaled {
                expr: LogicalExprNode::from_expr(col(":a") * lit(3.0))
                    .expect("Failed to serialize expr"),
                scale_name: None,
                band: None,
                scale_config: None,
                legend_config: None,
                share_mode: None,
            },
        );
        channels.insert(
            "d".to_string(),
            ChannelValue::Scaled {
                expr: LogicalExprNode::from_expr(col(":b") + col(":c"))
                    .expect("Failed to serialize expr"),
                scale_name: None,
                band: None,
                scale_config: None,
                legend_config: None,
                share_mode: None,
            },
        );

        let resolved = resolve_all_channel_refs(&channels, &ctx).unwrap();

        // d should have both paths resolved
        assert_eq!(
            resolved.get("d").unwrap().expr(&ctx).unwrap().to_string(),
            "base * Float64(2) + base * Float64(3)"
        );
    }

    #[test]
    fn test_error_message_formatting() {
        let ctx = SessionContext::new();
        let mut channels = IndexMap::new();
        channels.insert(
            "fill".to_string(),
            ChannelValue::Scaled {
                expr: LogicalExprNode::from_expr(col("color")).expect("Failed to serialize expr"),
                scale_name: None,
                band: None,
                scale_config: None,
                legend_config: None,
                share_mode: None,
            },
        );
        channels.insert(
            "stroke".to_string(),
            ChannelValue::Scaled {
                expr: LogicalExprNode::from_expr(col(":bogus")).expect("Failed to serialize expr"),
                scale_name: None,
                band: None,
                scale_config: None,
                legend_config: None,
                share_mode: None,
            },
        );

        let result = resolve_all_channel_refs(&channels, &ctx);
        let err = result.unwrap_err();
        let err_msg = err.to_string();

        // Check that error message contains helpful information
        assert!(err_msg.contains("stroke"));
        assert!(err_msg.contains(":bogus"));
        assert!(err_msg.contains("Available channels"));
        assert!(err_msg.contains("fill"));
    }

    #[test]
    fn test_similar_channel_suggestion() {
        let channels = vec!["fill".to_string(), "stroke".to_string(), "size".to_string()];

        // Test exact case-insensitive match
        assert_eq!(
            find_similar_channel("Fill", &channels),
            Some("fill".to_string())
        );

        // Test with typo (Levenshtein distance 1)
        assert_eq!(
            find_similar_channel("fil", &channels),
            Some("fill".to_string())
        );

        // Test with another typo (Levenshtein distance 2)
        assert_eq!(
            find_similar_channel("strke", &channels),
            Some("stroke".to_string())
        );

        // Test no match for very different string
        assert_eq!(
            find_similar_channel("completely_different", &channels),
            None
        );
    }
}
