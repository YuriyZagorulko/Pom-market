-- Explicit paper vs wallet trading accounts (wallet fields required only for wallet).

ALTER TABLE trading_accounts
    ADD COLUMN account_type text NOT NULL DEFAULT 'wallet'
        CHECK (account_type IN ('paper', 'wallet'));

UPDATE trading_accounts SET account_type = 'wallet';

ALTER TABLE trading_accounts
    ALTER COLUMN signature_type DROP NOT NULL,
    ALTER COLUMN signer_address DROP NOT NULL,
    ALTER COLUMN funder_address DROP NOT NULL;

ALTER TABLE trading_accounts
    DROP CONSTRAINT IF EXISTS trading_accounts_signature_type_check;

ALTER TABLE trading_accounts
    ADD CONSTRAINT trading_accounts_signature_type_check CHECK (
        signature_type IS NULL
        OR signature_type IN ('eoa', 'poly_proxy', 'poly_gnosis_safe', 'poly_1271')
    );

ALTER TABLE trading_accounts
    ADD CONSTRAINT trading_accounts_wallet_fields_required CHECK (
        account_type = 'paper'
        OR (
            signature_type IS NOT NULL
            AND signer_address IS NOT NULL
            AND TRIM(signer_address) <> ''
            AND funder_address IS NOT NULL
            AND TRIM(funder_address) <> ''
        )
    ) NOT VALID;

ALTER TABLE trading_accounts
    ADD CONSTRAINT trading_accounts_paper_has_no_wallet_fields CHECK (
        account_type <> 'paper'
        OR (
            signature_type IS NULL
            AND signer_address IS NULL
            AND funder_address IS NULL
        )
    ) NOT VALID;