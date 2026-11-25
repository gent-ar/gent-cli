use super::*;

#[tokio::test]
async fn an_old_generations_completion_never_reaches_a_recreated_team() {
    let root = tempfile::tempdir().unwrap();
    let home = root.path().join("alpha");
    let old = create_generation(&home).await.unwrap();
    publish(&old, "results.json", "[]").await.unwrap();

    delete_generations(&generations(&home)).await.unwrap();
    let recreated = create_generation(&home).await.unwrap();
    publish(&recreated, "results.json", "[]").await.unwrap();
    let late = publish(&old, "results.json", r#"[{"agent":"old"}]"#).await;

    assert!(late.is_err());
    assert_eq!(
        std::fs::read_to_string(recreated.join("results.json")).unwrap(),
        "[]"
    );
    assert_eq!(generations(&home), [recreated]);
}

#[tokio::test]
async fn a_stale_delete_removes_only_the_generation_it_saw() {
    let root = tempfile::tempdir().unwrap();
    let home = root.path().join("alpha");
    let old = create_generation(&home).await.unwrap();
    let token = CancellationToken::new();
    ACTIVE_TEAMS.insert(old.clone(), vec![token.clone()]);
    let seen = generations(&home);

    let recreated = create_generation(&home).await.unwrap();
    let kept = CancellationToken::new();
    ACTIVE_TEAMS.insert(recreated.clone(), vec![kept.clone()]);
    let cancelled = delete_generations(&seen).await.unwrap();

    assert_eq!(cancelled, 1);
    assert!(token.is_cancelled());
    assert!(!kept.is_cancelled());
    assert!(!old.exists());
    assert!(recreated.is_dir());
    ACTIVE_TEAMS.remove(&recreated);
}
