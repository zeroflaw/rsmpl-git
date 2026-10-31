struct DatabaseConnection {
    timeout: u32,
    retries: u8,
}
struct CacheEntry {
    timeout: bool,
    key: String,
}
fn total(a: DatabaseConnection, b: CacheEntry) -> u32 {
    let lit = DatabaseConnection { timeout: 30, retries: 3 };
    a.timeout + lit.timeout + if b.timeout { 1 } else { 0 }
}
