fn latency(cfg: ConnectionConfig) -> u32 {
    cfg.deadline_milliseconds + cfg.max_retry_attempts as u32
}
fn spawn() -> ConnectionConfig {
    ConnectionConfig { deadline_milliseconds: 5000, max_retry_attempts: 3 }
}
fn describe(cfg: ConnectionConfig) -> u32 {
    let ConnectionConfig { deadline_milliseconds: current, max_retry_attempts: _left } = cfg;
    current
}
fn duplicate(cfg: ConnectionConfig) -> ConnectionConfig {
    ConnectionConfig { deadline_milliseconds: cfg.deadline_milliseconds, max_retry_attempts: cfg.max_retry_attempts }
}
