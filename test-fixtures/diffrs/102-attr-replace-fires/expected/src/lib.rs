#[derive(Debug, Clone)]
pub struct Request {
    pub id: u64,
    pub path: String,
}

#[derive(Debug, Clone)]
pub struct Response {
    pub code: u16,
    pub body: Vec<u8>,
}

#[derive(Debug, Clone)]
struct RetryPolicy {
    attempts: u32,
    backoff_ms: u64,
}

#[derive(Debug, Clone)]
struct CacheEntry {
    key: String,
    hits: usize,
}
