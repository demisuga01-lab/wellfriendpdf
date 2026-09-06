//! In-memory progressive render session store with idle-timeout cancellation.
//!
//! Each session holds an owned [`ProgressiveRenderJob`] (which in turn owns the
//! [`ContentEngine`] and all tile buffers). The store is bounded by a max-session
//! cap and tracks last-access time so an idle-timeout reaper can cancel and
//! remove stale sessions without leaking document memory.
//!
//! The store is thread-safe (`Send + Sync`) via interior `RwLock`/`Mutex`
//! locking. Read-only operations (status) take the outer `RwLock` read lock and
//! an individual session's `Mutex`; mutating operations (step/pause/resume/cancel)
//! take the `RwLock` read lock plus the session `Mutex`. Only insert/remove take
//! the `RwLock` write lock.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, RwLock};
use std::time::{Duration, Instant};

use wellfriendpdf_engine::{
    CancelToken, ProgressiveAdjacentPagePrefetchExecutionReport,
    ProgressiveRenderContextRevisionReport, ProgressiveRenderInvalidationReport,
    ProgressiveRenderJob, ProgressiveRenderStepReport, ProgressiveTilePublication,
    ProgressiveTilePublicationAcceptance, ProgressiveViewerCallbackDispatchReport,
    ProgressiveViewerQueueExecutionReport, RenderContract, RenderTile,
};

use crate::error::ServerError;

/// A single progressive render session's state.
pub struct SessionEntry {
    pub job: ProgressiveRenderJob,
    pub last_access: Instant,
    pub created_at: Instant,
}

#[derive(Clone)]
struct SessionHandle {
    owner: String,
    cancel_token: CancelToken,
    entry: Arc<Mutex<SessionEntry>>,
}

/// Thread-safe progressive session store with bounded capacity and idle timeout.
#[derive(Clone)]
pub struct ProgressiveSessionStore {
    inner: Arc<StoreInner>,
}

#[derive(Clone, Debug, serde::Serialize)]
pub struct AdjacentPagePrefetchSessionReport {
    pub source_session_id: String,
    pub prefetch_session_id: Option<String>,
    pub report: ProgressiveAdjacentPagePrefetchExecutionReport,
}

struct StoreInner {
    sessions: RwLock<HashMap<String, SessionHandle>>,
    max_sessions: usize,
    idle_timeout: Duration,
}

impl ProgressiveSessionStore {
    /// Create a new store with the given capacity and idle timeout.
    pub fn new(max_sessions: usize, idle_timeout: Duration) -> Self {
        Self {
            inner: Arc::new(StoreInner {
                sessions: RwLock::new(HashMap::new()),
                max_sessions: max_sessions.max(1),
                idle_timeout,
            }),
        }
    }

    /// Insert a new session. Returns the session id or rejects if the bounded
    /// store is still at capacity after idle reaping.
    pub fn insert(
        &self,
        owner: String,
        mut job: ProgressiveRenderJob,
    ) -> Result<String, ServerError> {
        let id = generate_session_id();
        let mut map = self
            .inner
            .sessions
            .write()
            .unwrap_or_else(|e| e.into_inner());
        // At capacity: reap idle sessions first, then reject if still full.
        if map.len() >= self.inner.max_sessions {
            self.reap_idle_locked(&mut map);
        }
        if map.len() >= self.inner.max_sessions {
            job.close();
            return Err(ServerError::ResourceLimit(format!(
                "too many active progressive render sessions (max {})",
                self.inner.max_sessions
            )));
        }
        let cancel_token = job.cancellation_token();
        let entry = SessionEntry {
            job,
            last_access: Instant::now(),
            created_at: Instant::now(),
        };
        map.insert(
            id.clone(),
            SessionHandle {
                owner,
                cancel_token,
                entry: Arc::new(Mutex::new(entry)),
            },
        );
        Ok(id)
    }

    /// Perform a read-only operation on a session.
    pub fn with_session<F, R>(&self, id: &str, owner: &str, f: F) -> Result<R, ServerError>
    where
        F: FnOnce(&ProgressiveRenderJob) -> R,
    {
        let map = self
            .inner
            .sessions
            .read()
            .unwrap_or_else(|e| e.into_inner());
        let handle = map.get(id).cloned().ok_or_else(|| {
            ServerError::InvalidParameter(format!("progressive session '{}' not found", id))
        })?;
        if handle.owner != owner {
            return Err(ServerError::InvalidParameter(
                "progressive session not found".to_string(),
            ));
        }
        drop(map);
        let mut entry = handle.entry.lock().unwrap_or_else(|e| e.into_inner());
        entry.last_access = Instant::now();
        Ok(f(&entry.job))
    }

    /// Perform a mutable operation on a session.
    pub fn with_session_mut<F, R>(&self, id: &str, owner: &str, f: F) -> Result<R, ServerError>
    where
        F: FnOnce(&mut ProgressiveRenderJob) -> R,
    {
        let map = self
            .inner
            .sessions
            .read()
            .unwrap_or_else(|e| e.into_inner());
        let handle = map.get(id).cloned().ok_or_else(|| {
            ServerError::InvalidParameter(format!("progressive session '{}' not found", id))
        })?;
        if handle.owner != owner {
            return Err(ServerError::InvalidParameter(
                "progressive session not found".to_string(),
            ));
        }
        drop(map);
        let mut entry = handle.entry.lock().unwrap_or_else(|e| e.into_inner());
        entry.last_access = Instant::now();
        Ok(f(&mut entry.job))
    }

    /// Request and finalize cancellation for a session.
    ///
    /// The atomic token is signalled before taking the per-session mutex, so an
    /// in-flight step can observe cancellation while this caller waits to clear
    /// retained tile buffers and transition the job to a terminal state.
    pub fn cancel(&self, id: &str, owner: &str) -> Result<(), ServerError> {
        let map = self
            .inner
            .sessions
            .read()
            .unwrap_or_else(|e| e.into_inner());
        let handle = map.get(id).cloned().ok_or_else(|| {
            ServerError::InvalidParameter(format!("progressive session '{}' not found", id))
        })?;
        if handle.owner != owner {
            return Err(ServerError::InvalidParameter(
                "progressive session not found".to_string(),
            ));
        }
        handle.cancel_token.cancel();
        drop(map);

        let mut entry = handle.entry.lock().unwrap_or_else(|e| e.into_inner());
        entry.last_access = Instant::now();
        entry.job.cancel();
        Ok(())
    }

    /// Revise visible-work priority and mark prior tile publications obsolete.
    ///
    /// The current cancellation source is signalled before the per-session
    /// mutex is acquired, so an in-flight step can pause instead of continuing
    /// to publish tiles for a view the caller has already superseded.
    pub fn revise_viewport_hint(
        &self,
        id: &str,
        owner: &str,
        viewport_hint: Option<RenderTile>,
    ) -> Result<
        Result<ProgressiveRenderStepReport, wellfriendpdf_engine::WellfriendError>,
        ServerError,
    > {
        let map = self
            .inner
            .sessions
            .read()
            .unwrap_or_else(|e| e.into_inner());
        let handle = map.get(id).cloned().ok_or_else(|| {
            ServerError::InvalidParameter(format!("progressive session '{}' not found", id))
        })?;
        if handle.owner != owner {
            return Err(ServerError::InvalidParameter(
                "progressive session not found".to_string(),
            ));
        }
        handle.cancel_token.cancel();
        drop(map);

        let mut entry = handle.entry.lock().unwrap_or_else(|e| e.into_inner());
        entry.last_access = Instant::now();
        Ok(entry.job.revise_viewport_hint(viewport_hint))
    }

    /// Revise dirty-work scheduling and mark intersecting tile publications
    /// obsolete without discarding clean retained tiles.
    pub fn revise_dirty_region(
        &self,
        id: &str,
        owner: &str,
        dirty_region: Option<RenderTile>,
    ) -> Result<
        Result<ProgressiveRenderStepReport, wellfriendpdf_engine::WellfriendError>,
        ServerError,
    > {
        let map = self
            .inner
            .sessions
            .read()
            .unwrap_or_else(|e| e.into_inner());
        let handle = map.get(id).cloned().ok_or_else(|| {
            ServerError::InvalidParameter(format!("progressive session '{}' not found", id))
        })?;
        if handle.owner != owner {
            return Err(ServerError::InvalidParameter(
                "progressive session not found".to_string(),
            ));
        }
        handle.cancel_token.cancel();
        drop(map);

        let mut entry = handle.entry.lock().unwrap_or_else(|e| e.into_inner());
        entry.last_access = Instant::now();
        Ok(entry.job.revise_dirty_region(dirty_region))
    }

    /// Revise caller-visible render identity and mark every prior publication
    /// obsolete.
    pub fn revise_render_context(
        &self,
        id: &str,
        owner: &str,
        render_contract_fingerprint: Option<String>,
        visibility_fingerprint: Option<String>,
    ) -> Result<
        Result<ProgressiveRenderContextRevisionReport, wellfriendpdf_engine::WellfriendError>,
        ServerError,
    > {
        let map = self
            .inner
            .sessions
            .read()
            .unwrap_or_else(|e| e.into_inner());
        let handle = map.get(id).cloned().ok_or_else(|| {
            ServerError::InvalidParameter(format!("progressive session '{}' not found", id))
        })?;
        if handle.owner != owner {
            return Err(ServerError::InvalidParameter(
                "progressive session not found".to_string(),
            ));
        }
        handle.cancel_token.cancel();
        drop(map);

        let mut entry = handle.entry.lock().unwrap_or_else(|e| e.into_inner());
        entry.last_access = Instant::now();
        Ok(entry
            .job
            .revise_render_context(render_contract_fingerprint, visibility_fingerprint))
    }

    /// Revise the active render contract and mark every prior publication
    /// obsolete.
    pub fn revise_render_contract(
        &self,
        id: &str,
        owner: &str,
        contract: RenderContract,
    ) -> Result<
        Result<ProgressiveRenderContextRevisionReport, wellfriendpdf_engine::WellfriendError>,
        ServerError,
    > {
        let map = self
            .inner
            .sessions
            .read()
            .unwrap_or_else(|e| e.into_inner());
        let handle = map.get(id).cloned().ok_or_else(|| {
            ServerError::InvalidParameter(format!("progressive session '{}' not found", id))
        })?;
        if handle.owner != owner {
            return Err(ServerError::InvalidParameter(
                "progressive session not found".to_string(),
            ));
        }
        handle.cancel_token.cancel();
        drop(map);

        let mut entry = handle.entry.lock().unwrap_or_else(|e| e.into_inner());
        entry.last_access = Instant::now();
        Ok(entry.job.revise_render_contract(contract))
    }

    /// Apply a source-edit render-invalidation plan to the session-owned cache
    /// and obsolete affected retained tile publications.
    pub fn apply_render_invalidation_plan_json(
        &self,
        id: &str,
        owner: &str,
        plan_json: &str,
    ) -> Result<
        Result<ProgressiveRenderInvalidationReport, wellfriendpdf_engine::WellfriendError>,
        ServerError,
    > {
        let map = self
            .inner
            .sessions
            .read()
            .unwrap_or_else(|e| e.into_inner());
        let handle = map.get(id).cloned().ok_or_else(|| {
            ServerError::InvalidParameter(format!("progressive session '{}' not found", id))
        })?;
        if handle.owner != owner {
            return Err(ServerError::InvalidParameter(
                "progressive session not found".to_string(),
            ));
        }
        handle.cancel_token.cancel();
        drop(map);

        let mut entry = handle.entry.lock().unwrap_or_else(|e| e.into_inner());
        entry.last_access = Instant::now();
        Ok(entry.job.apply_render_invalidation_plan_json(plan_json))
    }

    /// Evaluate whether a step-report tile publication is still current for
    /// this session. This is the server-side stale-publication suppression hook
    /// for external viewers.
    pub fn evaluate_tile_publication(
        &self,
        id: &str,
        owner: &str,
        publication: &ProgressiveTilePublication,
    ) -> Result<ProgressiveTilePublicationAcceptance, ServerError> {
        self.with_session(id, owner, |job| job.evaluate_tile_publication(publication))
    }

    /// Return the current deterministic viewer queue preview without rendering
    /// another tile.
    pub fn viewer_queue_report(
        &self,
        id: &str,
        owner: &str,
    ) -> Result<ProgressiveRenderStepReport, ServerError> {
        self.with_session(id, owner, |job| job.viewer_queue_report())
    }

    /// Execute owned current-page work from the deterministic viewer queue and
    /// return adjacent-page entries as explicit deferred prefetch work.
    pub fn execute_viewer_queue(
        &self,
        id: &str,
        owner: &str,
        max_items: usize,
    ) -> Result<
        Result<ProgressiveViewerQueueExecutionReport, wellfriendpdf_engine::WellfriendError>,
        ServerError,
    > {
        self.with_session_mut(id, owner, |job| {
            job.execute_viewer_queue(max_items, &CancelToken::none())
        })
    }

    /// Execute an adjacent-page prefetch and retain the page-owned child job as
    /// a normal progressive session when the source session is live.
    pub fn execute_adjacent_page_prefetch(
        &self,
        id: &str,
        owner: &str,
        prefetch_identity: &str,
        max_tiles: usize,
    ) -> Result<
        Result<AdjacentPagePrefetchSessionReport, wellfriendpdf_engine::WellfriendError>,
        ServerError,
    > {
        let map = self
            .inner
            .sessions
            .read()
            .unwrap_or_else(|e| e.into_inner());
        let handle = map.get(id).cloned().ok_or_else(|| {
            ServerError::InvalidParameter(format!("progressive session '{}' not found", id))
        })?;
        if handle.owner != owner {
            return Err(ServerError::InvalidParameter(
                "progressive session not found".to_string(),
            ));
        }
        drop(map);

        let execution = {
            let mut entry = handle.entry.lock().unwrap_or_else(|e| e.into_inner());
            entry.last_access = Instant::now();
            entry.job.execute_adjacent_page_prefetch(
                prefetch_identity,
                max_tiles,
                &CancelToken::none(),
            )
        };
        let execution = match execution {
            Ok(execution) => execution,
            Err(error) => return Ok(Err(error)),
        };
        let prefetch_session_id = match execution.job {
            Some(job) => Some(self.insert(owner.to_string(), job)?),
            None => None,
        };

        Ok(Ok(AdjacentPagePrefetchSessionReport {
            source_session_id: id.to_string(),
            prefetch_session_id,
            report: execution.report,
        }))
    }

    /// Return the deterministic callback dispatch plan for external viewers
    /// without invoking language/runtime callbacks on the server thread.
    pub fn viewer_callback_dispatch_report(
        &self,
        id: &str,
        owner: &str,
    ) -> Result<ProgressiveViewerCallbackDispatchReport, ServerError> {
        self.with_session(id, owner, |job| job.viewer_callback_dispatch_report())
    }

    /// Remove a session by id.
    pub fn remove(&self, id: &str, owner: &str) -> bool {
        let mut map = self
            .inner
            .sessions
            .write()
            .unwrap_or_else(|e| e.into_inner());
        let owner_matches = map
            .get(id)
            .map(|handle| handle.owner == owner)
            .unwrap_or(false);
        if !owner_matches {
            return false;
        }
        if let Some(handle) = map.remove(id) {
            handle.cancel_token.cancel();
            let mut e = handle.entry.lock().unwrap_or_else(|p| p.into_inner());
            e.job.close();
            true
        } else {
            false
        }
    }

    /// Reap all sessions that have been idle longer than the configured timeout.
    /// Returns the number of sessions removed.
    pub fn reap_idle(&self) -> usize {
        let mut map = self
            .inner
            .sessions
            .write()
            .unwrap_or_else(|e| e.into_inner());
        self.reap_idle_locked(&mut map)
    }

    /// Number of active sessions.
    pub fn len(&self) -> usize {
        self.inner
            .sessions
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// The configured idle timeout.
    pub fn idle_timeout(&self) -> Duration {
        self.inner.idle_timeout
    }

    // -- private --

    fn reap_idle_locked(&self, map: &mut HashMap<String, SessionHandle>) -> usize {
        let now = Instant::now();
        let timeout = self.inner.idle_timeout;
        let mut removed = 0;
        map.retain(|_id, handle| {
            let mut e = handle.entry.lock().unwrap_or_else(|p| p.into_inner());
            if now.duration_since(e.last_access) >= timeout {
                handle.cancel_token.cancel();
                e.job.cancel();
                removed += 1;
                false
            } else {
                true
            }
        });
        removed
    }
}

pub fn spawn_cleanup_task(store: ProgressiveSessionStore) -> tokio::task::JoinHandle<()> {
    let interval = cleanup_interval(store.idle_timeout());
    tokio::spawn(async move {
        let mut ticker = tokio::time::interval(interval);
        ticker.tick().await;
        loop {
            ticker.tick().await;
            let reaped = store.reap_idle();
            if reaped > 0 {
                tracing::debug!(reaped, "reaped idle progressive render sessions");
            }
        }
    })
}

fn cleanup_interval(idle_timeout: Duration) -> Duration {
    let secs = (idle_timeout.as_secs() / 10).clamp(1, 60);
    Duration::from_secs(secs)
}

/// Generate a random session id (32 hex chars, 128 bits of entropy).
fn generate_session_id() -> String {
    let mut bytes = [0u8; 16];
    if getrandom::fill(&mut bytes).is_err() {
        // Fallback: use a combination of pointer and thread id.
        let stack_marker = &bytes as *const _ as usize;
        let tid = format!("{:?}", std::thread::current().id());
        let mut acc = stack_marker as u64 ^ 0xA3B1_C2D3_E4F5_6789;
        for b in tid.as_bytes() {
            acc = acc.rotate_left(7) ^ (*b as u64);
        }
        for (i, slot) in bytes.iter_mut().enumerate() {
            *slot = (acc >> ((i % 8) * 8)) as u8;
        }
    }
    let mut s = String::with_capacity(32);
    for b in bytes {
        s.push(char::from_digit((b >> 4) as u32, 16).unwrap());
        s.push(char::from_digit((b & 0x0f) as u32, 16).unwrap());
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn session_ids_are_32_hex_chars() {
        let id = generate_session_id();
        assert_eq!(id.len(), 32);
        assert!(id.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn session_ids_are_unique() {
        let mut ids = std::collections::HashSet::new();
        for _ in 0..1000 {
            assert!(ids.insert(generate_session_id()));
        }
    }

    #[test]
    fn cleanup_interval_clamps_to_reasonable_sweep_rate() {
        assert_eq!(
            cleanup_interval(Duration::from_secs(0)),
            Duration::from_secs(1)
        );
        assert_eq!(
            cleanup_interval(Duration::from_secs(50)),
            Duration::from_secs(5)
        );
        assert_eq!(
            cleanup_interval(Duration::from_secs(3600)),
            Duration::from_secs(60)
        );
    }
}
