use tokio::time::{Duration, timeout};
use venturi_runtime::{RuntimeEvent, RuntimeSupervisor};

#[tokio::test]
async fn supervisor_emits_ready_before_timeout() {
    let supervisor = RuntimeSupervisor::new_for_test();
    let mut rx = supervisor.subscribe();
    supervisor.start().await.unwrap();

    let event = timeout(Duration::from_secs(1), rx.recv())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(event, RuntimeEvent::Ready);
}
