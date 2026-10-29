fn ingest() {
    let a = decode_message::<u8>(first_packet_payload);
    let b = decode_message::<i32>(second_packet_payload);
    let c = decode_message::<u64>(third_packet_payload);
    let d = decode_message::<bool>(fourth_packet_payload);
    let e = decode_message::<f64>(fifth_packet_payload);
    let f = decode_message::<String>(sixth_packet_payload);
}
