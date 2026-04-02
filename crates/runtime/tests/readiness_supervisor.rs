use tokio::time::{Duration, timeout};
use venturi_runtime::readiness::ReadinessBarrier;
use venturi_runtime::{RuntimeEvent, RuntimeSupervisor};

#[tokio::test]
async fn wait_ready_returns_immediately_when_already_ready() {
    let barrier = ReadinessBarrier::new();
    barrier.mark_ready();

    timeout(Duration::from_millis(50), barrier.wait_ready())
        .await
        .unwrap();
}

#[tokio::test]
async fn wait_ready_unblocks_after_mark_ready() {
    let barrier = ReadinessBarrier::new();
    let waiter = tokio::spawn({
        let barrier = barrier.clone();
        async move {
            barrier.wait_ready().await;
        }
    });

    tokio::time::sleep(Duration::from_millis(10)).await;
    barrier.mark_ready();

    timeout(Duration::from_secs(1), waiter)
        .await
        .unwrap()
        .unwrap();
}

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

#[tokio::test]
async fn supervisor_start_succeeds_with_zero_subscribers() {
    let supervisor = RuntimeSupervisor::new_for_test();
    supervisor.start().await.unwrap();
}
