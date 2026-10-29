fn ingest() {
    let a = parse_message::<u8>(first_packet_payload);
    let b = parse_message::<i32>(second_packet_payload);
    let c = parse_message::<u64>(third_packet_payload);
    let d = parse_message::<bool>(fourth_packet_payload);
    let e = parse_message::<f64>(fifth_packet_payload);
    let f = parse_message::<String>(sixth_packet_payload);
}
