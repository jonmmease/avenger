use crate::marks::ChannelValue;
use datafusion::logical_expr::Expr;
use datafusion_common::tree_node::{Transformed, TransformedResult, TreeNode};
use indexmap::{IndexMap, IndexSet};
use std::collections::{HashMap, HashSet, VecDeque};

/// Error types for channel resolution
#[derive(Debug, Clone)]
pub enum ChannelResolutionError {
    CyclicDependency(Vec<String>),
}

impl std::fmt::Display for ChannelResolutionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ChannelResolutionError::CyclicDependency(cycle) => {
                write!(
                    f,
                    "Cyclic channel dependency detected: {}",
                    cycle.join(" -> ")
                )
            }
        }
    }
}

impl std::error::Error for ChannelResolutionError {}

/// Extract channel references from an expression
fn extract_channel_refs(expr: &Expr) -> HashSet<String> {
    let mut refs = HashSet::new();

    // Use a cell to allow mutation inside the closure
    let refs_cell = std::cell::RefCell::new(&mut refs);

    expr.apply(&|e: &Expr| {
        if let Expr::Column(c) = e {
            if c.name.starts_with(':') {
                refs_cell.borrow_mut().insert(c.name[1..].to_string());
            }
        }
        Ok(datafusion_common::tree_node::TreeNodeRecursion::Continue)
    })
    .ok();

    refs
}

/// Build dependency graph for channels
fn build_dependency_graph(
    channels: &IndexMap<String, ChannelValue>,
) -> HashMap<String, HashSet<String>> {
    let mut graph = HashMap::new();

    for (name, value) in channels {
        let deps = extract_channel_refs(value.expr());
        graph.insert(name.clone(), deps);
    }

    graph
}

/// Perform topological sort on channels based on dependencies
/// Returns the channels in dependency order or an error if there's a cycle
fn topological_sort(
    channels: &IndexMap<String, ChannelValue>,
) -> Result<Vec<String>, ChannelResolutionError> {
    let graph = build_dependency_graph(channels);
    let mut in_degree: HashMap<String, usize> = HashMap::new();
    let mut result = Vec::new();

    // Initialize in-degrees
    for name in channels.keys() {
        in_degree.insert(name.clone(), 0);
    }

    // Calculate in-degrees
    // If channel A references channel B (A depends on B), then B should come before A
    // So we increase the in-degree of A for each channel it depends on
    for (channel, deps) in &graph {
        // deps are the channels that 'channel' depends on
        // So 'channel' has an in-degree equal to the number of its dependencies
        *in_degree.get_mut(channel).unwrap() = deps.len();
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

        // Find all channels that depend on this node and reduce their in-degree
        for (channel, deps) in &graph {
            if deps.contains(&node) {
                if let Some(count) = in_degree.get_mut(channel) {
                    *count -= 1;
                    if *count == 0 {
                        queue.push_back(channel.clone());
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
        if let Some(cycle) = find_cycle_path(&graph, &unprocessed) {
            Err(ChannelResolutionError::CyclicDependency(cycle))
        } else {
            // Fallback: just report the nodes involved in the cycle
            Err(ChannelResolutionError::CyclicDependency(unprocessed))
        }
    }
}

/// Find a cycle path starting from any of the given nodes
fn find_cycle_path(
    graph: &HashMap<String, HashSet<String>>,
    nodes: &[String],
) -> Option<Vec<String>> {
    for start_node in nodes {
        let mut visited = IndexSet::new();
        let mut path = Vec::new();

        if find_cycle_dfs(graph, start_node, &mut visited, &mut path) {
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

/// DFS helper to find cycles
fn find_cycle_dfs(
    graph: &HashMap<String, HashSet<String>>,
    node: &str,
    visited: &mut IndexSet<String>,
    path: &mut Vec<String>,
) -> bool {
    if visited.contains(node) {
        path.push(node.to_string());
        return true;
    }

    visited.insert(node.to_string());
    path.push(node.to_string());

    if let Some(deps) = graph.get(node) {
        for dep in deps {
            if find_cycle_dfs(graph, dep, visited, path) {
                return true;
            }
        }
    }

    path.pop();
    false
}

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
                    // Return the expression directly (already resolved)
                    Ok(Transformed::yes(channel_value.expr().clone()))
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
    // Get topological order
    let order = match topological_sort(channels) {
        Ok(order) => order,
        Err(_) => {
            // On error, return original channels (graceful degradation)
            return channels.clone();
        }
    };

    // Resolve channels in topological order
    let mut resolved_channels = IndexMap::new();

    for name in &order {
        if let Some(value) = channels.get(name) {
            // Resolve references using already-resolved channels
            let resolved_expr = resolve_channel_refs(value.expr().clone(), &resolved_channels);

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

            resolved_channels.insert(name.clone(), resolved_value);
        }
    }

    resolved_channels
}
