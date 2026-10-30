use super::*;
use std::collections::VecDeque;
use std::sync::atomic::{AtomicUsize, Ordering};
use tokio::sync::oneshot;

type Reply = Result<NetworkAddress, AddressError>;
#[derive(Default)]
struct FakeSource {
    calls: [AtomicUsize; 2],
    context: Mutex<Option<String>>,
    pending: Mutex<VecDeque<oneshot::Receiver<Reply>>>,
    context_pending: Mutex<Option<oneshot::Receiver<Result<String, String>>>>,
    events: Mutex<Vec<Snapshot>>,
}
fn address(family: AddressFamily, suffix: usize) -> NetworkAddress {
    NetworkAddress {
        schema_version: 1,
        family: if family == AddressFamily::Ipv4 { 4 } else { 6 },
        ip: if family == AddressFamily::Ipv4 {
            format!("203.0.113.{suffix}")
        } else {
            format!("2001:db8::{suffix}")
        },
        country_code: Some("SG".into()),
    }
}
impl Source for FakeSource {
    fn context(&self) -> ContextFuture {
        let key = self
            .context
            .lock()
            .unwrap()
            .clone()
            .unwrap_or_else(|| "network-a".into());
        let pending = self.context_pending.lock().unwrap().take();
        Box::pin(async move {
            if let Some(pending) = pending {
                return pending.await.unwrap();
            }
            if key == "error" {
                Err("injected failure".into())
            } else {
                Ok(key)
            }
        })
    }

    fn probe(&self, family: AddressFamily) -> ProbeFuture {
        let index = usize::from(family == AddressFamily::Ipv6);
        let count = self.calls[index].fetch_add(1, Ordering::SeqCst) + 1;
        let receiver = self.pending.lock().unwrap().pop_front();
        Box::pin(async move {
            match receiver {
                Some(receiver) => receiver.await.unwrap(),
                None => Ok(address(family, count)),
            }
        })
    }
    fn publish(&self, snapshot: Snapshot) {
        self.events.lock().unwrap().push(snapshot);
    }
}
fn setup() -> (Arc<PublicNetwork>, Arc<FakeSource>) {
    let source = Arc::new(FakeSource::default());
    (PublicNetwork::new(source.clone()), source)
}
async fn settle() {
    for _ in 0..12 {
        tokio::task::yield_now().await;
    }
}
fn count(source: &FakeSource, family: usize) -> usize {
    source.calls[family].load(Ordering::SeqCst)
}

#[tokio::test(start_paused = true)]
async fn elapsed_time_alone_never_starts_a_query_even_after_the_cache_expires() {
    let (worker, source) = setup();
    worker.configure_policy(true);
    tokio::time::advance(Duration::from_secs(7200)).await;
    settle().await;
    assert_eq!(count(&source, 0), 0);
    worker.request(Request::Open).await;
    settle().await;
    tokio::time::advance(Duration::from_secs(7200)).await;
    worker.request(Request::Observe).await;
    settle().await;
    assert_eq!(count(&source, 0), 1);
    worker.request(Request::Open).await;
    settle().await;
    assert_eq!(count(&source, 0), 2);
}

#[tokio::test(start_paused = true)]
async fn disabling_clears_cache_discards_inflight_results_and_blocks_queries() {
    let (worker, source) = setup();
    let (v4, r4) = oneshot::channel();
    let (v6, r6) = oneshot::channel();
    source.pending.lock().unwrap().extend([r4, r6]);
    worker.configure_policy(true);
    worker.request(Request::Open).await;
    settle().await;
    worker.configure_policy(false);
    v4.send(Ok(address(AddressFamily::Ipv4, 99))).unwrap();
    v6.send(Ok(address(AddressFamily::Ipv6, 99))).unwrap();
    settle().await;
    let snapshot = worker.request(Request::Refresh).await;
    assert!(snapshot.probes.ipv4.address.is_none());
    assert!(!snapshot.probes.ipv4.loading);
    tokio::time::advance(Duration::from_secs(600)).await;
    settle().await;
    assert_eq!(count(&source, 0), 1);
}

#[tokio::test]
async fn manual_refresh_coalesces_with_running_requests_and_publishes_families_independently() {
    let (worker, source) = setup();
    let (v4, r4) = oneshot::channel();
    let (v6, r6) = oneshot::channel();
    source.pending.lock().unwrap().extend([r4, r6]);
    worker.configure_policy(true);
    worker.request(Request::Refresh).await;
    settle().await;
    for _ in 0..5 {
        worker.request(Request::Refresh).await;
    }
    assert_eq!(count(&source, 0), 1);
    v4.send(Ok(address(AddressFamily::Ipv4, 1))).unwrap();
    settle().await;
    assert!(worker.snapshot().probes.ipv4.address.is_some());
    assert!(worker.snapshot().probes.ipv6.loading);
    v6.send(Err(AddressError::Network)).unwrap();
    settle().await;
    assert!(worker.snapshot().probes.ipv6.failed);
    assert_eq!(count(&source, 0), 1);
    worker.request(Request::Refresh).await;
    settle().await;
    assert_eq!(count(&source, 0), 2);
}

#[tokio::test]
async fn repeated_network_changes_discard_old_generations_without_duplicating_the_finished_family()
{
    let (worker, source) = setup();
    let (v4, r4) = oneshot::channel();
    let (v6, r6) = oneshot::channel();
    source.pending.lock().unwrap().extend([r4, r6]);
    worker.configure_policy(true);
    worker.request(Request::Open).await;
    settle().await;
    *source.context.lock().unwrap() = Some("network-b".into());
    worker.request(Request::Observe).await;
    *source.context.lock().unwrap() = Some("network-c".into());
    worker.request(Request::Observe).await;
    assert!(worker.snapshot().probes.ipv4.address.is_none());
    v4.send(Ok(address(AddressFamily::Ipv4, 99))).unwrap();
    settle().await;
    assert_eq!(
        worker.snapshot().probes.ipv4.address.unwrap().ip,
        "203.0.113.2"
    );
    v6.send(Err(AddressError::Timeout)).unwrap();
    settle().await;
    assert_eq!((count(&source, 0), count(&source, 1)), (2, 2));
}

#[tokio::test]
async fn failed_refresh_preserves_same_network_addresses_and_context_failure_recovers() {
    let (worker, source) = setup();
    worker.configure_policy(true);
    worker.request(Request::Open).await;
    settle().await;
    let (v4, r4) = oneshot::channel();
    let (v6, r6) = oneshot::channel();
    source.pending.lock().unwrap().extend([r4, r6]);
    *source.context.lock().unwrap() = Some("error".into());
    worker.request(Request::Refresh).await;
    settle().await;
    v4.send(Err(AddressError::Network)).unwrap();
    v6.send(Err(AddressError::Timeout)).unwrap();
    settle().await;
    assert!(worker.snapshot().context_failed);
    assert_eq!(
        worker.snapshot().probes.ipv4.address.unwrap().ip,
        "203.0.113.1"
    );
    assert!(worker.snapshot().probes.ipv4.failed);
    *source.context.lock().unwrap() = Some("network-a".into());
    worker.request(Request::Observe).await;
    assert!(!worker.snapshot().context_failed);
    worker.request(Request::Open).await;
    settle().await;
    assert_eq!(count(&source, 0), 3);
    assert!(!worker.snapshot().probes.ipv4.failed);
}

#[test]
fn snapshots_have_an_explicit_version_and_preferences_require_both_enable_switches() {
    let (worker, _) = setup();
    let mut preferences = super::super::preferences::ResidentPreferences::default();
    worker.configure(&preferences);
    assert!(worker.cache.lock().unwrap().enabled);
    preferences.enabled = false;
    worker.configure(&preferences);
    assert!(!worker.cache.lock().unwrap().enabled);
    let value = serde_json::to_value(worker.snapshot()).unwrap();
    assert_eq!(value["schemaVersion"], 1);
    assert!(value["probes"]["ipv4"]["address"].is_null());
}

#[tokio::test]
async fn disabling_while_reading_context_prevents_a_late_query() {
    let (worker, source) = setup();
    worker.configure_policy(true);
    let (reply, pending) = oneshot::channel();
    *source.context_pending.lock().unwrap() = Some(pending);
    let cloned = worker.clone();
    let opening = tokio::spawn(async move { cloned.request(Request::Open).await });
    settle().await;
    worker.configure_policy(false);
    reply.send(Ok("network-a".into())).unwrap();
    opening.await.unwrap();
    settle().await;
    assert_eq!(count(&source, 0), 0);
}

#[tokio::test]
async fn reenabling_does_not_restart_old_requests_until_an_explicit_open() {
    let (worker, source) = setup();
    let (v4, r4) = oneshot::channel();
    let (v6, r6) = oneshot::channel();
    source.pending.lock().unwrap().extend([r4, r6]);
    worker.configure_policy(true);
    worker.request(Request::Open).await;
    settle().await;
    worker.configure_policy(false);
    worker.configure_policy(true);
    v4.send(Ok(address(AddressFamily::Ipv4, 99))).unwrap();
    v6.send(Ok(address(AddressFamily::Ipv6, 99))).unwrap();
    settle().await;
    assert_eq!(count(&source, 0), 1);
    assert!(worker.snapshot().probes.ipv4.address.is_none());
    worker.request(Request::Open).await;
    settle().await;
    assert_eq!(count(&source, 0), 2);
}

#[tokio::test(start_paused = true)]
async fn successful_ipv4_survives_unavailable_ipv6_for_one_hour_after_completion() {
    let (worker, source) = setup();
    let (v4, r4) = oneshot::channel();
    let (v6, r6) = oneshot::channel();
    source.pending.lock().unwrap().extend([r4, r6]);
    worker.configure_policy(true);
    worker.request(Request::Open).await;
    settle().await;
    tokio::time::advance(Duration::from_secs(20)).await;
    v4.send(Ok(address(AddressFamily::Ipv4, 1))).unwrap();
    v6.send(Err(AddressError::Network)).unwrap();
    settle().await;
    tokio::time::advance(Duration::from_secs(3599)).await;
    let cached = worker.request(Request::Open).await;
    settle().await;
    assert_eq!(cached.probes.ipv4.address.unwrap().ip, "203.0.113.1");
    assert_eq!(
        count(&source, 0),
        1,
        "fresh usable IPv4 must survive an unavailable IPv6 transport"
    );
    tokio::time::advance(Duration::from_secs(1)).await;
    worker.request(Request::Open).await;
    settle().await;
    assert_eq!(
        count(&source, 0),
        2,
        "the one-hour cache expires only when opened after its lifetime"
    );
}

#[tokio::test]
async fn waiting_for_obsolete_probes_advances_the_snapshot_revision() {
    let (worker, source) = setup();
    let (v4, r4) = oneshot::channel();
    let (v6, r6) = oneshot::channel();
    source.pending.lock().unwrap().extend([r4, r6]);
    worker.configure_policy(true);
    worker.request(Request::Open).await;
    settle().await;
    *source.context.lock().unwrap() = Some("network-b".into());
    worker.request(Request::Observe).await;
    let events = source.events.lock().unwrap().clone();
    v4.send(Ok(address(AddressFamily::Ipv4, 99))).unwrap();
    v6.send(Err(AddressError::Timeout)).unwrap();
    settle().await;
    let before_wait = &events[events.len() - 2];
    let waiting = &events[events.len() - 1];
    assert!(!before_wait.probes.ipv4.loading);
    assert!(waiting.probes.ipv4.loading);
    assert!(waiting.revision > before_wait.revision, "distinct loading states must not share a revision that permits an older response to replace them");
}

#[test]
fn cached_addresses_expire_after_sleep_even_if_the_monotonic_clock_pauses() {
    let cache = Cache {
        cached_at: Some(Instant::now()),
        snapshot: Snapshot {
            updated_at_ms: super::super::sampling_workers::timestamp_ms().saturating_sub(3_600_001),
            ..Default::default()
        },
        ..Default::default()
    };
    assert!(
        cache.expired(),
        "sleep time must count toward the address cache lifetime"
    );
}
