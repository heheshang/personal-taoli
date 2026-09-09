//! 行情归档与连续影子统计模块。
//!
//! 负责保存可复现的行情、规格版本、费率版本、决策和数据缺口，
//! 形成连续影子运行报告。

mod event_types;
mod reader;
mod report;
mod writer;

pub(crate) const SCHEMA_VERSION: u16 = 1;

pub use event_types::{
    ArchiveGap, ArchiveRecord, ArchivedEvent, ArchivedFeeVersion, DecisionConfig, DecisionEvent,
    FeedVersion, HealthEvent,
};
pub use reader::replay_archive;
pub use report::{
    CapacityDistribution, ProfitDistribution, ShadowReport, TailSample, default_gap_path,
};
pub use writer::ArchiveWriter;
