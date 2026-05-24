use strsim::levenshtein;

/// Error types for channel resolution.
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
                    cycle.join(" -> "),
                    cycle.join(" -> "),
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

                    let similar = suggest_similar_channel_name(channel, available_channels);
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
                     - Use a direct data expression instead of referencing '{}'\n\
                     - Extract the common expression to a variable\n\
                     - Consider using a non-conditional channel for the shared value",
                    channel, referenced_by, channel, channel
                )
            }
        }
    }
}

impl std::error::Error for ChannelResolutionError {}

/// Find the most similar channel name using Levenshtein edit distance.
pub fn suggest_similar_channel_name(target: &str, channels: &[String]) -> Option<String> {
    let target_lower = target.to_lowercase();
    for channel in channels {
        if channel.to_lowercase() == target_lower {
            return Some(channel.clone());
        }
    }

    channels
        .iter()
        .map(|channel| {
            let distance = levenshtein(&target_lower, &channel.to_lowercase());
            (channel, distance)
        })
        .filter(|(_, dist)| {
            let max_distance = std::cmp::max(3, target.len() * 2 / 5);
            *dist <= max_distance
        })
        .min_by_key(|(_, dist)| *dist)
        .map(|(channel, _)| channel.clone())
}
