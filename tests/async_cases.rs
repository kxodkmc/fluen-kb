#![cfg(feature = "async")]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod common;

use common::src;
use fluen_kb::ids::WikiType::Summary;
use fluen_kb::{QueryParams, WikiType};

#[tokio::test]
async fn async_handle_end_to_end() {
    let dir = tempfile::tempdir().unwrap();
    let kb = fluen_kb::KbBuilder::new(dir.path()).open().unwrap();
    let akb = kb.into_async();

    let s = src("ref-1111111111111111");
    let outcome = akb
        .create(
            Summary,
            "异步综述".into(),
            format!("<{s}>异步写入的内容。</{s}>"),
            vec![],
            Some(s.clone()),
            vec![],
        )
        .await
        .unwrap();
    let id = match outcome {
        fluen_kb::CreateOutcome::Created(id) => id,
        fluen_kb::CreateOutcome::MergedInto(_) => panic!("expected Created"),
    };

    let metas = akb.list_entries(Some(WikiType::Summary)).await.unwrap();
    assert_eq!(metas.len(), 1);
    assert_eq!(metas[0].id, id);
    assert_eq!(metas[0].sources, vec![s]);

    let hits = akb
        .query(QueryParams {
            query: "异步写入".into(),
            ..Default::default()
        })
        .await
        .unwrap();
    assert_eq!(hits.len(), 1);

    akb.delete(id.clone()).await.unwrap();
    assert!(akb.get_entry(id).await.is_err());

    // 并发读写不死锁
    let a = akb.clone();
    let b = akb.clone();
    let (ra, rb) = tokio::join!(
        a.list_entries(None),
        b.query(QueryParams {
            query: "任意查询".into(),
            ..Default::default()
        })
    );
    assert!(ra.is_ok());
    assert!(rb.is_ok());
}
