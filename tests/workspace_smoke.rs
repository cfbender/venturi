use venturi_domain::{Channel, channel_node_name};

#[test]
fn workspace_exposes_domain_channel() {
    assert_eq!(channel_node_name(Channel::Main), "Venturi-Output");
}
