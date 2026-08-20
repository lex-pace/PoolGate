//! PoolGate Token Monitor —— 模块声明与对外 re-export。
//!
//! **共享文件**：子 Agent 只在此文件顶部按字典序追加 `pub mod xxx;`，
//! 不得改动他人条目；由集成负责人统一合并。

pub mod alerts;
pub mod collector;
pub mod daily_archive;
pub mod dedup;
pub mod detect;
pub mod model;
pub mod normalization;
pub mod pricing;
pub mod quota;
pub mod service;
pub mod service_collect;
pub mod status;

pub use model::*;
pub use service::{init, TokenMonitorRuntime};
