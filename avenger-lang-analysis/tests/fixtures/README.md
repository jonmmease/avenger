# Editor Analysis Corpus

These fixtures freeze representative valid and incomplete editing states for
the Avenger analysis and LSP implementation. `⟦cursor⟧` is a harness-only
marker: tests remove it and use its UTF-8 byte offset. No production parser
accepts the marker.

The corpus grows additively. Update or remove a fixture only when a reviewed
language-specification change invalidates its source contract.
