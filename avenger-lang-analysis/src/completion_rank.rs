use crate::{CompletionInvocation, CompletionItem};

/// Broad provider-side admission used before the invocation-sensitive final
/// ranker. Providers may construct exact, prefix, and subsequence matches;
/// [`rank_and_deduplicate`] is the sole authority that retains fuzzy matches
/// only for explicitly invoked completion.
pub(crate) fn candidate_matches(candidate: &str, typed: &str) -> bool {
    let typed = typed.trim_start_matches('$').to_ascii_lowercase();
    let candidate = candidate.to_ascii_lowercase();
    typed.is_empty() || candidate.starts_with(&typed) || fuzzy_subsequence(&candidate, &typed)
}

/// Apply the shared invocation-sensitive matcher, deterministic semantic
/// ranking, and exact-identity deduplication used by both structural and SQL
/// completion domains.
pub(crate) fn rank_and_deduplicate(
    items: &mut Vec<CompletionItem>,
    prefix: &str,
    invocation: &CompletionInvocation,
) {
    let typed = prefix.trim_start_matches('$').to_ascii_lowercase();
    items.retain_mut(|item| {
        let label = item.match_text.trim_start_matches('$').to_ascii_lowercase();
        let match_rank = if label == typed {
            0
        } else if label.starts_with(&typed) {
            1
        } else if matches!(invocation, CompletionInvocation::Invoked)
            && fuzzy_subsequence(&label, &typed)
        {
            2
        } else {
            return false;
        };
        let confidence = u16::MAX - item.confidence;
        let proximity = u16::MAX - item.semantic_proximity;
        let prevalence = u32::MAX - item.usage_prevalence;
        let type_rank = match item.expected_type_compatible {
            Some(true) => 0,
            None => 1,
            Some(false) => 2,
        };
        item.sort_key = format!(
            "{match_rank}:{type_rank}:{confidence:05}:{proximity:05}:{prevalence:010}:{}",
            item.sort_key
        );
        true
    });
    items.sort_by(|left, right| {
        left.sort_key
            .cmp(&right.sort_key)
            .then_with(|| left.label.cmp(&right.label))
            .then_with(|| left.semantic_identity.cmp(&right.semantic_identity))
            .then_with(|| left.insert_text.cmp(&right.insert_text))
    });
    items.dedup_by(|left, right| {
        left.semantic_identity == right.semantic_identity
            && left.replacement == right.replacement
            && left.insert_text == right.insert_text
            && left.insert_text_format == right.insert_text_format
    });
}

fn fuzzy_subsequence(candidate: &str, typed: &str) -> bool {
    let mut typed = typed.chars();
    let mut expected = typed.next();
    for character in candidate.chars() {
        if Some(character) == expected {
            expected = typed.next();
            if expected.is_none() {
                return true;
            }
        }
    }
    expected.is_none()
}

#[cfg(test)]
mod tests {
    use avenger_lang_core::{ByteSpan, SourceId, SourceSpan};

    use super::*;
    use crate::{
        CompletionKind, CompletionOrigin, CompletionQualification, CompletionSemanticKind,
        CompletionTextFormat, CompletionValidity,
    };

    fn item(identity: &str, insert: &str) -> CompletionItem {
        CompletionItem {
            label: "id".to_owned(),
            replacement: SourceSpan {
                source: SourceId::new(0),
                range: ByteSpan { start: 3, end: 5 },
            },
            insert_text: insert.to_owned(),
            insert_text_format: CompletionTextFormat::PlainText,
            kind: CompletionKind::Field,
            semantic_kind: CompletionSemanticKind::DataColumn,
            semantic_identity: identity.to_owned(),
            match_text: "id".to_owned(),
            qualification: CompletionQualification::Unqualified,
            data_type: None,
            nullable: None,
            source_stage: None,
            expected_type_compatible: None,
            confidence: 100,
            semantic_proximity: 100,
            usage_prevalence: 0,
            validity: CompletionValidity::Strict,
            detail: None,
            documentation: None,
            filter_text: Some("\"id\"".to_owned()),
            sort_key: "00:id".to_owned(),
            origin: CompletionOrigin::QueryScope,
            deprecated: false,
        }
    }

    #[test]
    fn deduplication_uses_semantic_identity_and_exact_edit() {
        let mut items = vec![
            item("left.id", "left.\"id\""),
            item("right.id", "right.\"id\""),
            item("left.id", "left.\"id\""),
        ];
        rank_and_deduplicate(&mut items, "i", &CompletionInvocation::Invoked);
        assert_eq!(items.len(), 2);
        assert_eq!(items[0].semantic_identity, "left.id");
        assert_eq!(items[1].semantic_identity, "right.id");
    }
}
