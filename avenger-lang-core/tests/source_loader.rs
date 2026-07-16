use avenger_lang_core::{
    ContentVersion, ImportCapabilities, InMemorySourceLoader, LoadedSource, SourceLoader,
    SourceLoaderError, SourceOrigin,
};

#[tokio::test]
async fn in_memory_loader_is_capability_checked_and_versioned() {
    let origin = SourceOrigin::Memory("chart.avenger".to_string());
    let loader = InMemorySourceLoader::default().with_source(LoadedSource::new(
        origin.clone(),
        "avenger 1;",
        ContentVersion::new("fixture-v1"),
    ));

    let loaded = loader
        .load(&origin, &ImportCapabilities::in_memory("/project"))
        .await
        .unwrap();
    assert_eq!(&*loaded.text, "avenger 1;");
    assert_eq!(loaded.version.as_str(), "fixture-v1");

    let denied = ImportCapabilities::project("/project");
    assert!(matches!(
        loader.load(&origin, &denied).await,
        Err(SourceLoaderError::CapabilityDenied(found)) if found == origin
    ));
}
