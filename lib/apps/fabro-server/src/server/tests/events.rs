use super::*;

#[tokio::test]
async fn get_events_not_found() {
    let app = test_app_with();
    let missing_run_id = fixtures::RUN_64;

    let req = Request::builder()
        .method("GET")
        .uri(api(&format!("/runs/{missing_run_id}/events")))
        .body(Body::empty())
        .unwrap();

    let response = app.oneshot(req).await.unwrap();
    assert_status!(response, StatusCode::NOT_FOUND).await;
}

#[tokio::test]
async fn filtered_global_events_streams_only_matching_run_ids() {
    let run_one = fixtures::RUN_1;
    let run_two = fixtures::RUN_2;
    let (event_tx, _) = broadcast::channel(8);

    let stream = filtered_global_events(event_tx.subscribe(), Some(HashSet::from([run_one])));

    event_tx
        .send(test_event_envelope(
            1,
            run_two,
            EventBody::RunRunnable(fabro_types::run_event::RunRunnableProps {
                source: fabro_types::RunRunnableSource::StartRequested,
            }),
        ))
        .unwrap();
    event_tx
        .send(test_event_envelope(
            2,
            run_one,
            EventBody::RunRunnable(fabro_types::run_event::RunRunnableProps {
                source: fabro_types::RunRunnableSource::StartRequested,
            }),
        ))
        .unwrap();
    drop(event_tx);

    let events = stream.collect::<Vec<_>>().await;
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].seq, 2);
    assert_eq!(events[0].event.run_id, run_one);
}
