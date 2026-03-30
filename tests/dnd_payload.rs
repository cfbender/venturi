use venturi::core::messages::Channel;
use venturi::gui::app_chip::DndPayload;

#[test]
fn payload_roundtrip_encode_decode() {
    let payload = DndPayload {
        stream_id: 42,
        app_key: "discord".to_string(),
        origin: Channel::Chat,
    };

    let encoded = payload.encode();
    let decoded = DndPayload::decode(&encoded).expect("decode payload");

    assert_eq!(decoded, payload);
}

#[test]
fn payload_decode_rejects_invalid_data() {
    assert!(DndPayload::decode("bad").is_none());
    assert!(DndPayload::decode("1|key|Unknown").is_none());
}
