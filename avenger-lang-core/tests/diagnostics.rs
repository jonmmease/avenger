use avenger_lang_core::{
    Diagnostic, SourceFile, SourceId, SourceLabel, SourceMap, SourceOrigin, SourceSpan,
    render_diagnostics, sort_diagnostics,
};

#[test]
fn diagnostic_ordering_and_multi_label_rendering_match_baseline() {
    let first_id = SourceId::new(17);
    let second_id = SourceId::new(3);
    let first = SourceFile::new(
        first_id,
        SourceOrigin::Memory("a.avenger".to_string()),
        "chart demo {\n\tparam width: int64 = 12;\n}\n",
    );
    let second = SourceFile::new(
        second_id,
        SourceOrigin::Memory("z.avenger".to_string()),
        "chart other {}\n",
    );
    let mut sources = SourceMap::default();
    sources.insert(first).unwrap();
    sources.insert(second).unwrap();

    let width_start = sources.get(first_id).unwrap().text().find("width").unwrap();
    let param_start = sources.get(first_id).unwrap().text().find("param").unwrap();
    let mut diagnostics = vec![
        Diagnostic::error(
            "AV0002",
            "later source error",
            SourceLabel::new(SourceSpan::new(second_id, 0, 5).unwrap(), "later"),
        ),
        Diagnostic::error(
            "AV0001",
            "duplicate state name",
            SourceLabel::new(
                SourceSpan::new(first_id, width_start, width_start + 5).unwrap(),
                "`width` is declared again",
            ),
        )
        .with_secondary(SourceLabel::new(
            SourceSpan::new(first_id, param_start, param_start + 5).unwrap(),
            "first declaration starts here",
        ))
        .with_note("params and stores share one namespace"),
    ];

    sort_diagnostics(&mut diagnostics, &sources);
    assert_eq!(diagnostics[0].code.as_str(), "AV0001");

    let rendered = render_diagnostics(&diagnostics, &sources);
    assert_eq!(
        rendered,
        include_str!("baselines/diagnostics/multi_label.txt")
    );
}
