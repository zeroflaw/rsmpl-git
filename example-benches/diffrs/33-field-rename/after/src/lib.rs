struct ConnectionConfig {
    response_deadline_milliseconds: u32,
    max_retry_attempts: u8,
}
fn latency(cfg: ConnectionConfig) -> u32 {
    cfg.response_deadline_milliseconds + cfg.max_retry_attempts as u32
}
fn spawn() -> ConnectionConfig {
    ConnectionConfig { response_deadline_milliseconds: 5000, max_retry_attempts: 3 }
}
fn describe(cfg: ConnectionConfig) -> u32 {
    let ConnectionConfig { response_deadline_milliseconds: current, max_retry_attempts: _left } = cfg;
    current
}
fn duplicate(cfg: ConnectionConfig) -> ConnectionConfig {
    ConnectionConfig { response_deadline_milliseconds: cfg.response_deadline_milliseconds, max_retry_attempts: cfg.max_retry_attempts }
}
fn quick() -> ConnectionConfig {
    let request_timeout_milliseconds = 42;
    ConnectionConfig { response_deadline_milliseconds: request_timeout_milliseconds, max_retry_attempts: 0 }
}
