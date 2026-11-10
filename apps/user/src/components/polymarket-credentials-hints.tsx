import {
  POLYMARKET_ACCOUNT_LINKS,
  funderRequiredForSignature,
  type SignatureTypeValue,
} from "@/lib/polymarket-account-hints";
import { HelpLink } from "@/components/field-help";

export function PrivateKeyHelp({ signatureType }: { signatureType: SignatureTypeValue }) {
  const magic = signatureType === "poly_proxy";
  return (
    <>
      <p>
        <strong>Where can I get my Private Key?</strong>
      </p>
      {magic ? (
        <ol>
          <li>Open the official Polymarket Magic key export page.</li>
          <li>Authenticate with the same Google account or email used on Polymarket.</li>
          <li>Export your private key and paste it here.</li>
        </ol>
      ) : signatureType === "eoa" ? (
        <p>
          For a standard wallet (MetaMask, Rabby, hardware wallet, etc.), export the private key from that wallet&apos;s
          settings. Do <strong>not</strong> use the Magic Link export page for EOA wallets.
        </p>
      ) : (
        <p>
          Use the private key for the wallet that signs on behalf of your Polymarket account (often exported from your
          wallet app or custody setup). For email/Google Polymarket logins, prefer{" "}
          <strong>Proxy / email wallet</strong> and the Magic export flow instead.
        </p>
      )}
      {magic && (
        <p>
          <HelpLink href={POLYMARKET_ACCOUNT_LINKS.magicKeyExport}>reveal.magic.link/polymarket</HelpLink>
        </p>
      )}
      <p className="field-help-warn">
        Never share your private key with anyone. POM Market uses it to sign requests on your behalf.
      </p>
    </>
  );
}

export function SignerAddressHelp({ signatureType }: { signatureType: SignatureTypeValue }) {
  return (
    <>
      <p>
        <strong>What is Signer Address?</strong>
      </p>
      <p>
        The Signer Address is the Ethereum address derived from your private key. It represents the wallet that signs
        Polymarket API requests.
      </p>
      {signatureType !== "eoa" && (
        <p>
          For Magic / Google accounts, this address is usually <strong>different</strong> from your Polymarket profile
          address (Funder). Paste the address that matches your exported private key — not the profile address shown on
          polymarket.com.
        </p>
      )}
      {signatureType !== "eoa" && (
        <p>POM Market checks that your private key matches this signer address when you save credentials.</p>
      )}
      <p>
        <HelpLink href={POLYMARKET_ACCOUNT_LINKS.walletsAuthDocs}>Polymarket wallets &amp; authentication docs</HelpLink>
      </p>
    </>
  );
}

export function FunderAddressHelp({ signatureType }: { signatureType: SignatureTypeValue }) {
  if (!funderRequiredForSignature(signatureType)) {
    return (
      <>
        <p>
          <strong>What is Funder Address?</strong>
        </p>
        <p>
          For <strong>EOA</strong> accounts, funds sit on the same wallet as the signer. You can leave this empty; the
          signer address is used as the funder.
        </p>
      </>
    );
  }
  return (
    <>
      <p>
        <strong>What is Funder Address?</strong>
      </p>
      <p>
        The Funder Address is your Polymarket profile wallet — where your USDC balance and positions are held. For Proxy,
        Safe, and deposit-wallet setups it differs from the signing EOA.
      </p>
      <p>
        For Magic / Google accounts, use the address on your Polymarket profile, not the address derived from your
        exported private key.
      </p>
      <p>
        Open <HelpLink href={POLYMARKET_ACCOUNT_LINKS.profileSettings}>Polymarket settings</HelpLink> and check the{" "}
        <strong>Address</strong> field under your profile (the deposit / proxy address, not an arbitrary deposit QR for
        unrelated chains).
      </p>
    </>
  );
}

export function SignatureTypeHelp() {
  return (
    <>
      <p>
        <strong>Signature Type</strong>
      </p>
      <ul className="field-help-list">
        <li>
          <strong>EOA</strong> — standard external wallet (MetaMask, hardware wallet, etc.).
        </li>
        <li>
          <strong>Proxy / email wallet</strong> — Polymarket accounts created with Google, email, or Magic Link.
        </li>
        <li>
          <strong>Gnosis Safe</strong> — Polymarket accounts backed by a Gnosis Safe proxy (supported by this app).
        </li>
        <li>
          <strong>EIP-1271 deposit wallet</strong> — Polymarket deposit-wallet signature mode when applicable.
        </li>
      </ul>
      <p className="muted">
        If you signed up with Google or email, you will typically need <strong>Proxy / email wallet</strong>.
      </p>
      <p>
        <HelpLink href={POLYMARKET_ACCOUNT_LINKS.walletsAuthDocs}>Official signature type documentation</HelpLink>
      </p>
    </>
  );
}
