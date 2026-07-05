use std::sync::{Arc, Condvar, Mutex};

use rayon::prelude::*;
use serde::{Deserialize, Serialize};

use crate::error::{OxideError, Result};
use crate::filters::DecodeLimits;

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct DecodeSchedulerMetrics {
    pub jobs: usize,
    pub workers: usize,
    pub memory_budget_bytes: u64,
    pub peak_reserved_bytes: u64,
    pub wait_count: usize,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct RendererDecodeSchedulerAdoptionReport {
    pub status: String,
    pub execution_model: String,
    pub bounded_parallelism: String,
    pub memory_tokens: bool,
    pub deterministic_output_order: bool,
    pub cancellation_observed_before_decode: bool,
    pub adopted_paths: Vec<String>,
    pub audited_deferred_paths: Vec<String>,
    pub timeout_posture: String,
}

pub fn renderer_decode_scheduler_adoption_report() -> RendererDecodeSchedulerAdoptionReport {
    RendererDecodeSchedulerAdoptionReport {
        status: "adopted_for_immediate_renderer_decode_paths".to_string(),
        execution_model: "synchronous_deterministic_decode_with_scheduler_memory_tokens".to_string(),
        bounded_parallelism: "renderer uses one decode job at a time today; batch decode scheduler remains bounded by DecodeLimits::max_concurrent_decode_jobs".to_string(),
        memory_tokens: true,
        deterministic_output_order: true,
        cancellation_observed_before_decode: true,
        adopted_paths: vec![
            "image_xobject_decode".to_string(),
            "inline_image_decode".to_string(),
            "soft_mask_image_decode".to_string(),
            "stencil_mask_image_decode".to_string(),
            "form_xobject_stream_decode".to_string(),
            "transparency_form_stream_decode".to_string(),
            "annotation_appearance_stream_decode".to_string(),
            "tiling_pattern_stream_decode".to_string(),
            "mesh_shading_stream_decode".to_string(),
            "tile_render_full_page_decode_via_shared_path".to_string(),
            "band_render_full_page_decode_via_shared_path".to_string(),
        ],
        audited_deferred_paths: vec![
            "render_image_cache stores final tile buffers, not decoded image streams".to_string(),
            "parallel renderer predecode is intentionally not enabled in Prompt 04".to_string(),
        ],
        timeout_posture: "renderer exposes cooperative cancellation through CancelToken; no binding-level fake progress/cancellation was added".to_string(),
    }
}

#[derive(Debug)]
struct BudgetState {
    reserved: u64,
    peak: u64,
    waits: usize,
}

#[derive(Debug)]
pub struct DecodeMemoryBudget {
    limit: u64,
    state: Mutex<BudgetState>,
    available: Condvar,
}

impl DecodeMemoryBudget {
    pub fn new(limit: u64) -> Self {
        Self {
            limit: limit.max(1),
            state: Mutex::new(BudgetState {
                reserved: 0,
                peak: 0,
                waits: 0,
            }),
            available: Condvar::new(),
        }
    }

    pub fn acquire(self: &Arc<Self>, requested: u64) -> Result<DecodeMemoryToken> {
        let requested = requested.max(1);
        if requested > self.limit {
            return Err(OxideError::MalformedPdf(format!(
                "decode job requested {requested} bytes, exceeding scheduler budget {}",
                self.limit
            )));
        }
        let mut state = self.state.lock().map_err(|_| {
            OxideError::ParseError("decode scheduler memory budget lock poisoned".to_string())
        })?;
        while state.reserved + requested > self.limit {
            state.waits += 1;
            state = self.available.wait(state).map_err(|_| {
                OxideError::ParseError("decode scheduler memory budget lock poisoned".to_string())
            })?;
        }
        state.reserved += requested;
        state.peak = state.peak.max(state.reserved);
        Ok(DecodeMemoryToken {
            budget: Arc::clone(self),
            bytes: requested,
        })
    }

    pub fn metrics(&self) -> DecodeSchedulerMetrics {
        let state = self.state.lock().expect("decode scheduler metrics lock");
        DecodeSchedulerMetrics {
            jobs: 0,
            workers: 0,
            memory_budget_bytes: self.limit,
            peak_reserved_bytes: state.peak,
            wait_count: state.waits,
        }
    }
}

pub struct DecodeMemoryToken {
    budget: Arc<DecodeMemoryBudget>,
    bytes: u64,
}

impl Drop for DecodeMemoryToken {
    fn drop(&mut self) {
        if let Ok(mut state) = self.budget.state.lock() {
            state.reserved = state.reserved.saturating_sub(self.bytes);
            self.budget.available.notify_one();
        }
    }
}

pub struct ScheduledDecodeJob<T> {
    index: usize,
    estimated_bytes: u64,
    work: Box<dyn FnOnce() -> Result<T> + Send>,
}

impl<T> ScheduledDecodeJob<T> {
    pub fn new(
        index: usize,
        estimated_bytes: u64,
        work: impl FnOnce() -> Result<T> + Send + 'static,
    ) -> Self {
        Self {
            index,
            estimated_bytes,
            work: Box::new(work),
        }
    }
}

/// Run independent decode jobs with deterministic output ordering and a shared memory budget.
pub fn run_scheduled_decode_jobs<T: Send + 'static>(
    jobs: Vec<ScheduledDecodeJob<T>>,
    limits: &DecodeLimits,
) -> (Vec<Result<T>>, DecodeSchedulerMetrics) {
    let workers = limits.max_concurrent_decode_jobs.max(1);
    let memory_budget = Arc::new(DecodeMemoryBudget::new(
        limits.scheduler_memory_budget_bytes.max(1),
    ));
    let job_count = jobs.len();
    let pool = rayon::ThreadPoolBuilder::new()
        .num_threads(workers)
        .build()
        .expect("decode scheduler thread pool");

    let mut indexed = pool.install(|| {
        jobs.into_par_iter()
            .map(|job| {
                let token = memory_budget.acquire(job.estimated_bytes);
                let result = match token {
                    Ok(_token) => (job.work)(),
                    Err(err) => Err(err),
                };
                (job.index, result)
            })
            .collect::<Vec<_>>()
    });
    indexed.sort_by_key(|(index, _)| *index);
    let results = indexed.into_iter().map(|(_, result)| result).collect();
    let mut metrics = memory_budget.metrics();
    metrics.jobs = job_count;
    metrics.workers = workers;
    (results, metrics)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scheduler_preserves_output_order() {
        let limits = DecodeLimits {
            max_concurrent_decode_jobs: 4,
            scheduler_memory_budget_bytes: 16,
            ..DecodeLimits::default()
        };
        let jobs = vec![
            ScheduledDecodeJob::new(2, 4, || Ok("c")),
            ScheduledDecodeJob::new(0, 4, || Ok("a")),
            ScheduledDecodeJob::new(1, 4, || Ok("b")),
        ];
        let (results, metrics) = run_scheduled_decode_jobs(jobs, &limits);
        let values = results.into_iter().map(Result::unwrap).collect::<Vec<_>>();
        assert_eq!(values, vec!["a", "b", "c"]);
        assert_eq!(metrics.jobs, 3);
        assert!(metrics.peak_reserved_bytes <= 16);
    }

    #[test]
    fn scheduler_rejects_job_over_budget() {
        let limits = DecodeLimits {
            max_concurrent_decode_jobs: 2,
            scheduler_memory_budget_bytes: 4,
            ..DecodeLimits::default()
        };
        let jobs = vec![ScheduledDecodeJob::new(0, 8, || {
            Ok::<_, OxideError>(b"nope")
        })];
        let (results, _) = run_scheduled_decode_jobs(jobs, &limits);
        assert!(results[0].is_err());
    }

    #[test]
    fn scheduled_decode_matches_serial_ordered_output() {
        let limits = DecodeLimits {
            max_concurrent_decode_jobs: 2,
            scheduler_memory_budget_bytes: 8,
            ..DecodeLimits::default()
        };
        let inputs = [b"61>".to_vec(), b"62>".to_vec(), b"63>".to_vec()];
        let serial = inputs
            .iter()
            .map(|input| {
                crate::filters::apply_filter_bytes_with_limits(
                    "ASCIIHexDecode",
                    input,
                    None,
                    &limits,
                )
                .unwrap()
            })
            .collect::<Vec<_>>();
        let jobs = inputs
            .into_iter()
            .enumerate()
            .map(|(index, input)| {
                let limits = limits.clone();
                ScheduledDecodeJob::new(index, 2, move || {
                    crate::filters::apply_filter_bytes_with_limits(
                        "ASCIIHexDecode",
                        &input,
                        None,
                        &limits,
                    )
                })
            })
            .collect();
        let (parallel, metrics) = run_scheduled_decode_jobs(jobs, &limits);
        let parallel = parallel.into_iter().map(Result::unwrap).collect::<Vec<_>>();
        assert_eq!(serial, parallel);
        assert!(metrics.peak_reserved_bytes <= 8);
    }
}
