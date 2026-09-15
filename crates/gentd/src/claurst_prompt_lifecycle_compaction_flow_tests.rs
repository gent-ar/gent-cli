use super::*;

#[tokio::test]
async fn compact_records_one_summary_and_notice_starts_no_acp_prompt_and_seeds_the_next_turn() {
    let summarizer = summarizer(81_888, vec![summary("The user planted LARK-7.")]);
    let mut harness = Harness::new(LocalRuntime(Arc::clone(&summarizer))).await;
    let planted = harness
        .provider_turn("Remember LARK-7", "Noted LARK-7")
        .await;
    let committed = harness.transcript();

    let compact = harness.save("/compact");
    assert_eq!(harness.settle(&compact).await, DurableTurnPhase::Completed);

    assert_eq!(harness.bridge.starts().len(), 1);
    let requests = summarizer.requests.lock().unwrap().clone();
    assert_eq!(requests.len(), 1);
    assert!(requests[0].content.contains("Remember LARK-7"));
    assert!(requests[0].content.contains("Noted LARK-7"));
    assert!(!requests[0].content.contains("/compact"));
    assert!(matches!(
        harness.facts().as_slice(),
        [ContextCompactionFact::Compacted {
            trigger: ContextCompactionTrigger::Command,
            covers_through_ordinal: 2,
            summary,
            ..
        }] if summary == "The user planted LARK-7."
    ));
    assert_eq!(
        harness.notices(&compact),
        [PROVIDER_CONTEXT_COMPACTED_NOTICE]
    );
    assert_eq!(
        &harness.transcript()[..committed.len()],
        committed.as_slice()
    );

    let recall = harness.save("Which code did I plant?");
    harness.expect_provider_turn(&recall);
    assert_eq!(harness.settle(&recall).await, DurableTurnPhase::Completed);
    let starts = harness.bridge.starts();
    let context = &starts[1].context;
    assert_eq!(
        context
            .summary
            .as_ref()
            .map(|summary| summary.text.as_str()),
        Some("The user planted LARK-7.")
    );
    assert!(context.entries.is_empty());
    assert!(!context.earlier_history_omitted);
    assert_eq!(planted.message.text, "Remember LARK-7");
}

#[tokio::test]
async fn a_prompt_over_budget_compacts_first_then_starts_from_the_summary_without_recompacting() {
    let summarizer = summarizer(15_000, vec![summary("Earlier: six long notes.")]);
    let mut harness = Harness::new(LocalRuntime(Arc::clone(&summarizer))).await;
    for index in 1..=5 {
        harness
            .provider_turn(&format!("note {index} {}", "n".repeat(2_500)), "ok")
            .await;
    }
    assert!(summarizer.requests.lock().unwrap().is_empty());

    let over = harness.save(&format!("continue {}", "c".repeat(5_000)));
    harness.expect_provider_turn(&over);
    assert_eq!(harness.settle(&over).await, DurableTurnPhase::Completed);

    assert_eq!(summarizer.requests.lock().unwrap().len(), 1);
    assert_eq!(harness.notices(&over), [PROVIDER_CONTEXT_COMPACTED_NOTICE]);
    let starts = harness.bridge.starts();
    let seeded = &starts.last().unwrap().context;
    let covered = seeded.summary.as_ref().unwrap().covers_through_ordinal;
    assert_eq!(covered, 4);
    assert!(seeded.entries.iter().all(|entry| entry.ordinal > covered));
    assert!(!seeded.earlier_history_omitted);
    assert!(matches!(
        harness.facts().as_slice(),
        [ContextCompactionFact::Compacted {
            trigger: ContextCompactionTrigger::Budget,
            ..
        }]
    ));

    let next = harness.save("and again");
    harness.expect_provider_turn(&next);
    assert_eq!(harness.settle(&next).await, DurableTurnPhase::Completed);
    assert_eq!(summarizer.requests.lock().unwrap().len(), 1);
    assert!(harness.notices(&next).is_empty());
    assert_eq!(harness.bridge.starts().len(), starts.len() + 1);
}

#[tokio::test]
async fn a_failed_summary_never_fails_the_prompt_and_backs_off_with_a_typed_notice() {
    let summarizer = summarizer(
        15_000,
        vec![Some(Err(ContextCompactionFailure::OutputLimit))],
    );
    let mut harness = Harness::new(LocalRuntime(Arc::clone(&summarizer))).await;
    for index in 1..=5 {
        harness
            .provider_turn(&format!("note {index} {}", "n".repeat(2_500)), "ok")
            .await;
    }
    let over = harness.save(&format!("continue {}", "c".repeat(5_000)));
    harness.expect_provider_turn(&over);
    assert_eq!(harness.settle(&over).await, DurableTurnPhase::Completed);
    assert_eq!(harness.notices(&over), [CONTEXT_COMPACTION_FALLBACK_NOTICE]);
    assert!(matches!(
        harness.facts().as_slice(),
        [ContextCompactionFact::Failed {
            failure: ContextCompactionFailure::OutputLimit,
            ..
        }]
    ));
    assert!(
        harness
            .bridge
            .starts()
            .last()
            .unwrap()
            .context
            .summary
            .is_none()
    );

    let next = harness.save("still going");
    harness.expect_provider_turn(&next);
    assert_eq!(harness.settle(&next).await, DurableTurnPhase::Completed);
    assert_eq!(summarizer.requests.lock().unwrap().len(), 1);
    assert_eq!(harness.notices(&next), [CONTEXT_COMPACTION_FALLBACK_NOTICE]);
}

#[tokio::test]
async fn interrupting_a_running_compaction_settles_the_turn_without_a_fact() {
    let summarizer = summarizer(81_888, vec![None]);
    let mut harness = Harness::new(LocalRuntime(Arc::clone(&summarizer))).await;
    harness.provider_turn("Remember LARK-7", "Noted").await;
    let compact = harness.save("/compact");
    for _ in 0..20 {
        harness.lifecycle.drive_once().await.unwrap();
        tokio::time::sleep(Duration::from_millis(2)).await;
    }
    assert_eq!(summarizer.requests.lock().unwrap().len(), 1);
    assert!(!harness.phase(&compact).is_terminal());
    harness
        .lifecycle
        .interrupt_run(&compact.run_id.0)
        .await
        .unwrap();
    assert_eq!(
        harness.settle(&compact).await,
        DurableTurnPhase::Interrupted
    );
    assert!(harness.facts().is_empty());
    assert!(harness.notices(&compact).is_empty());
    assert_eq!(harness.bridge.cancellations().len(), 0);
}

#[tokio::test]
async fn a_compact_without_a_local_summarizer_fails_its_turn_with_the_typed_notice() {
    let mut harness = Harness::new(ReadyClaurstRuntime).await;
    harness.provider_turn("Remember LARK-7", "Noted").await;
    let compact = harness.save("/compact");
    assert_eq!(harness.settle(&compact).await, DurableTurnPhase::Failed);
    assert_eq!(
        harness.notices(&compact),
        [PROVIDER_CONTEXT_COMPACTION_FAILED_NOTICE]
    );
    assert!(matches!(
        harness.facts().as_slice(),
        [ContextCompactionFact::Failed {
            trigger: ContextCompactionTrigger::Command,
            failure: ContextCompactionFailure::RuntimeUnavailable,
            ..
        }]
    ));
    assert_eq!(harness.bridge.starts().len(), 1);
}
