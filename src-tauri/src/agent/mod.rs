// Агентный рантайм.
//
// S6: цикл вызовов инструментов в [`executor`]. S7: подтверждения
// двумя уровнями и журнал того, что агент делал.

pub mod approvals;
pub mod compaction;
pub mod executor;
pub mod live;