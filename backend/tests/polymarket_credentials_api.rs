use pommarket::polymarket::credentials::{derived_signer_address, verify_key_signer_configuration};

const TEST_KEY: &str = "0x59c6995e998f97a5a0044966f0945389dc9e86dae88c7a8412f0a55b2a4f2732";
const PROFILE: &str = "0x2501833efcccfeebcb43051493546b36092d671cb";

#[test]
fn api_error_messages_do_not_contain_private_key() {
    let derived = derived_signer_address(TEST_KEY).unwrap();
    let err = verify_key_signer_configuration("eoa", &derived, PROFILE, PROFILE).unwrap_err();
    assert!(!err.contains(TEST_KEY.trim_start_matches("0x")));
    assert!(err.contains("poly_proxy"));
}
