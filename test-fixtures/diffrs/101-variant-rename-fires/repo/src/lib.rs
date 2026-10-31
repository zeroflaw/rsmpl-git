enum ConnectionState {
    EstablishedConnection(u32),
    ClosedConnection,
}
use crate::ConnectionState::EstablishedConnection;
fn handle(state: ConnectionState) -> u32 {
    let first = ConnectionState::EstablishedConnection(1);
    let second = ConnectionState::EstablishedConnection(2);
    match state {
        ConnectionState::EstablishedConnection(value) => value + first + second,
        ConnectionState::ClosedConnection => 0,
    }
}
