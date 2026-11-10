"use client";

import { FieldLabel } from "@/components/field-help";
import {
  FunderAddressHelp,
  PrivateKeyHelp,
  SignatureTypeHelp,
  SignerAddressHelp,
} from "@/components/polymarket-credentials-hints";
import { funderRequiredForSignature, type SignatureTypeValue } from "@/lib/polymarket-account-hints";

const SIGNATURE_OPTIONS: { value: SignatureTypeValue; label: string }[] = [
  { value: "eoa", label: "EOA" },
  { value: "poly_proxy", label: "Proxy / email wallet" },
  { value: "poly_gnosis_safe", label: "Gnosis Safe" },
  { value: "poly_1271", label: "EIP-1271 deposit wallet" },
];

type Props = {
  signatureType: SignatureTypeValue;
  onSignatureTypeChange: (value: SignatureTypeValue) => void;
  showName?: boolean;
  nameDefault?: string;
  privateKeyRequired?: boolean;
  privateKeyPlaceholder?: string;
  signerDefault?: string;
  funderDefault?: string;
};

export function PolymarketAccountCredentialsFields({
  signatureType,
  onSignatureTypeChange,
  showName,
  nameDefault,
  privateKeyRequired,
  privateKeyPlaceholder = "Private key 0x…",
  signerDefault,
  funderDefault,
}: Props) {
  const funderRequired = funderRequiredForSignature(signatureType);
  return (
  <>
      {showName && (
        <>
          <FieldLabel label="Name" helpTitle="Account name" help={<p>A label for this account inside POM Market.</p>} htmlFor="account-name" />
          <input id="account-name" name="name" placeholder="Name" defaultValue={nameDefault} required />
        </>
      )}
      <FieldLabel label="Signature type" helpTitle="Signature Type" help={<SignatureTypeHelp />} htmlFor="signature_type" />
      <select
        id="signature_type"
        name="signature_type"
        value={signatureType}
        onChange={(e) => onSignatureTypeChange(e.target.value as SignatureTypeValue)}
      >
        {SIGNATURE_OPTIONS.map((opt) => (
          <option key={opt.value} value={opt.value}>
            {opt.label}
          </option>
        ))}
      </select>
      <FieldLabel
        label="Signer address"
        helpTitle="Signer Address"
        help={<SignerAddressHelp signatureType={signatureType} />}
        htmlFor="signer_address"
      />
      <input
        id="signer_address"
        name="signer_address"
        placeholder="Signer EOA from exported private key"
        defaultValue={signerDefault}
        required
        autoComplete="off"
      />
      <FieldLabel
        label="Funder address"
        helpTitle="Funder Address"
        help={<FunderAddressHelp signatureType={signatureType} />}
        htmlFor="funder_address"
        optional={!funderRequired}
      />
      <input
        id="funder_address"
        name="funder_address"
        placeholder={
          funderRequired
            ? "Polymarket profile / proxy address"
            : "Same as signer if left empty"
        }
        defaultValue={funderDefault}
        autoComplete="off"
      />
      <FieldLabel
        label="Private key"
        helpTitle="Private Key"
        help={<PrivateKeyHelp signatureType={signatureType} />}
        htmlFor="private_key"
        optional={!privateKeyRequired}
      />
      <input
        id="private_key"
        name="private_key"
        placeholder={privateKeyPlaceholder}
        required={privateKeyRequired}
        autoComplete="off"
      />
    </>
  );
}
