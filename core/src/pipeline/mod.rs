//! Pipeline scheduling and control.
//!
//! Contains the main pipeline scheduler, processing stages, backpressure
//! control, canary probing, and circuit breaker logic.

pub mod backpressure;
pub mod canary;
pub mod circuit_breaker;
pub mod policy;
pub mod scheduler;
pub mod stages;

pub use backpressure::{BackpressureConfig, BackpressureController, DropStrategy};
pub use canary::{
    CanaryConfig, CanaryManager, CanaryProber, CanaryResult, CanaryStats, SinkHealth,
};
pub use circuit_breaker::{CircuitBreaker, CircuitBreakerConfig, CircuitState};
pub use policy::{DropLevelPolicy, RateLimiter};
pub use scheduler::Pipeline;
pub use stages::{
    report_stats, run_pipeline, PipelineContext, StageAction, StageIndex, StageStats,
};
