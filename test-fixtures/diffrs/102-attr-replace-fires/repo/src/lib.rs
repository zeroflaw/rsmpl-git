#[derive(Debug)]
pub struct Request {
    pub id: u64,
    pub path: String,
}

#[derive(Debug)]
pub struct Response {
    pub code: u16,
    pub body: Vec<u8>,
}

#[derive(Debug)]
struct RetryPolicy {
    attempts: u32,
    backoff_ms: u64,
}

#[derive(Debug)]
struct CacheEntry {
    key: String,
    hits: usize,
}
