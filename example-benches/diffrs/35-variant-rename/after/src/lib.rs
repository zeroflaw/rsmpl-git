enum ConnectionState {
    ActiveSession(u32),
    ClosedConnection,
}
use crate::ConnectionState::ActiveSession;
fn handle(state: ConnectionState) -> u32 {
    let first = ConnectionState::ActiveSession(1);
    let second = ConnectionState::ActiveSession(2);
    match state {
        ConnectionState::ActiveSession(value) => value + first + second,
        ConnectionState::ClosedConnection => 0,
    }
}
