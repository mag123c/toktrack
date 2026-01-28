//! Services for data aggregation and processing
#![allow(dead_code)]

pub mod aggregator;
pub mod cache;
pub mod pricing;
pub mod update_checker;

pub use aggregator::Aggregator;
pub use pricing::PricingService;
pub use update_checker::check_and_prompt_update;
