use venturi_domain::Channel;

#[test]
fn workspace_exposes_domain_channel() {
    assert!(matches!(Channel::Main, Channel::Main));
}
