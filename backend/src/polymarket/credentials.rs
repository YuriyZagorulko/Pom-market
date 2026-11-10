//! Private key and signer/funder validation for Polymarket CLOB accounts.

use crate::engine::{is_private_key, normalize_address};

use super::live::address_of;

/// Strip whitespace/newlines so pasted Magic export keys validate.
pub fn sanitize_private_key(raw: &str) -> Result<String, String> {
    let compact: String = raw.chars().filter(|c| !c.is_whitespace()).collect();
    if compact.is_empty() {
        return Err("private key is required".into());
    }
    Ok(compact)
}

pub fn validate_private_key_format(raw: &str) -> Result<String, String> {
    let key = sanitize_private_key(raw)?;
    if !is_private_key(&key) {
        return Err("private key must be a 32-byte hex key, not a seed phrase".into());
    }
    Ok(key)
}

pub fn derived_signer_address(private_key: &str) -> Result<String, String> {
    let key = validate_private_key_format(private_key)?;
    address_of(&key)
}

/// Ensures the exported key belongs to `signer_address`, not the Polymarket profile/funder.
pub fn verify_key_signer_configuration(
    signature_type: &str,
    derived_signer: &str,
    signer_address: &str,
    funder_address: &str,
) -> Result<(), String> {
    let derived = normalize_address(derived_signer);
    let signer = normalize_address(signer_address);
    let funder = normalize_address(funder_address);

    if derived == signer {
        if signature_type != "eoa" && signer == funder {
            return Err(
                "proxy and safe accounts need a funder address (Polymarket profile) different from the signing EOA".into(),
            );
        }
        return Ok(());
    }

    if signature_type == "eoa" && signer == funder {
        return Err(
            "This looks like a Polymarket email or Magic wallet. Use signature type poly_proxy, put your Polymarket profile address in funder, and the EOA address from reveal.magic.link in signer.".into(),
        );
    }

    if signature_type != "eoa" && derived == funder && derived != signer {
        return Err(
            "The private key matches the funder address, not the signer. Signer must be the EOA from your Magic export; funder is the address shown on your Polymarket profile.".into(),
        );
    }

    if signature_type != "eoa" && derived != signer {
        return Err(format!(
            "private key does not match signer address (derived {derived}, expected signer {signer})"
        ));
    }

    Err("private key does not match signer address".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    // Deterministic test fixture (64-byte hex); not a user or production key.
    const TEST_KEY: &str = "0x59c6995e998f97a5a0044966f0945389dc9e86dae88c7a8412f0a55b2a4f2732";
    const TEST_EOA: &str = "0x1917fe6a2c26914bd21a4ddfdbcb927b4b456c8f";
    const PROXY_ONLY: &str = "0x2501833efcccfeebcb43051493546b36092d671cb";

    #[test]
    fn derives_anvil_signer_with_and_without_0x() {
        let with = derived_signer_address(TEST_KEY).unwrap();
        let no_prefix = derived_signer_address(TEST_KEY.trim_start_matches("0x")).unwrap();
        assert_eq!(with, TEST_EOA);
        assert_eq!(no_prefix, TEST_EOA);
    }

    #[test]
    fn strips_whitespace_from_pasted_key() {
        let padded = "  0x59c6995e998f97a5a0044966f0945389dc9e86dae88c7a8412f0a55b2a4f2732 \n";
        assert!(validate_private_key_format(padded).is_ok());
    }

    #[test]
    fn eoa_match_ok() {
        verify_key_signer_configuration("eoa", TEST_EOA, TEST_EOA, TEST_EOA).unwrap();
    }

    #[test]
    fn wrong_key_rejected() {
        let err =
            verify_key_signer_configuration("eoa", TEST_EOA, PROXY_ONLY, PROXY_ONLY).unwrap_err();
        assert!(err.contains("Magic") || err.contains("does not match"));
    }

    #[test]
    fn magic_misconfigured_as_eoa_gets_hint() {
        let err =
            verify_key_signer_configuration("eoa", TEST_EOA, PROXY_ONLY, PROXY_ONLY).unwrap_err();
        assert!(err.contains("poly_proxy"));
        assert!(err.contains("reveal.magic.link"));
    }

    #[test]
    fn poly_proxy_valid_signer_and_funder() {
        verify_key_signer_configuration("poly_proxy", TEST_EOA, TEST_EOA, PROXY_ONLY).unwrap();
    }

    #[test]
    fn poly_proxy_key_in_funder_field_rejected() {
        let err = verify_key_signer_configuration("poly_proxy", TEST_EOA, PROXY_ONLY, TEST_EOA)
            .unwrap_err();
        assert!(err.contains("funder address, not the signer"));
    }

    #[test]
    fn poly_proxy_same_signer_and_funder_rejected() {
        let err = verify_key_signer_configuration("poly_proxy", TEST_EOA, TEST_EOA, TEST_EOA)
            .unwrap_err();
        assert!(err.contains("different from the signing EOA"));
    }

    #[test]
    fn addresses_case_insensitive() {
        let upper = "0x1917FE6a2c26914bd21a4ddfdbcb927b4b456c8f";
        verify_key_signer_configuration("eoa", TEST_EOA, upper, upper).unwrap();
    }

    #[test]
    fn invalid_hex_key_rejected() {
        assert!(validate_private_key_format("0xdead").is_err());
        assert!(validate_private_key_format("seed phrase words").is_err());
    }
}
