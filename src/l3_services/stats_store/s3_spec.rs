//! Frozen S3 acceptance modules. The behavior module alone is the baseline runtime RED lane.
use super::{disk, maint};
#[path = "s3_api.rs"]
mod s3_api;
#[path = "s3_behavior.rs"]
mod s3_behavior;
