"""Polymarket CTF Exchange V2 EIP-712 Order Signer and CLOB v2 L2 HMAC Authenticator."""

from __future__ import annotations

import base64
from dataclasses import dataclass
from datetime import datetime, timezone
from decimal import Decimal
import hashlib
import hmac
import secrets
from typing import Any, Optional

import ecdsa
from ecdsa.util import sigdecode_string, sigencode_string_canonize

from backend.app.models import SignatureType

# ---------------------------------------------------------------------------
# Pure-Python Keccak-256 (Ethereum / EIP-712 standard with 0x01 padding)
# ---------------------------------------------------------------------------
_RC = [
    0x0000000000000001, 0x0000000000008082, 0x800000000000808A, 0x8000000080008000,
    0x000000000000808B, 0x0000000080000001, 0x8000000080008081, 0x8000000000008009,
    0x000000000000008A, 0x0000000000000088, 0x0000000080008009, 0x000000008000000A,
    0x000000008000808B, 0x800000000000008B, 0x8000000000008089, 0x8000000000008003,
    0x8000000000008002, 0x8000000000000080, 0x000000000000800A, 0x800000008000000A,
    0x8000000080008081, 0x8000000000008080, 0x0000000080000001, 0x8000000080008008,
]

_ROT = [
    [0, 36, 3, 41, 18],
    [1, 44, 10, 45, 2],
    [62, 6, 43, 15, 61],
    [28, 55, 25, 21, 56],
    [27, 20, 39, 8, 14],
]


def _rol64(val: int, shift: int) -> int:
    return ((val << shift) | (val >> (64 - shift))) & 0xFFFFFFFFFFFFFFFF


def _keccak_f1600(state: list[int]) -> None:
    for r in range(24):
        c = [state[x] ^ state[x + 5] ^ state[x + 10] ^ state[x + 15] ^ state[x + 20] for x in range(5)]
        d = [c[(x - 1) % 5] ^ _rol64(c[(x + 1) % 5], 1) for x in range(5)]
        for x in range(5):
            for y in range(5):
                state[x + 5 * y] ^= d[x]
        b = [0] * 25
        for x in range(5):
            for y in range(5):
                b[y + 5 * ((2 * x + 3 * y) % 5)] = _rol64(state[x + 5 * y], _ROT[x][y])
        for x in range(5):
            for y in range(5):
                state[x + 5 * y] = b[x + 5 * y] ^ ((~b[(x + 1) % 5 + 5 * y]) & b[(x + 2) % 5 + 5 * y])
        state[0] ^= _RC[r]


def keccak256(data: bytes) -> bytes:
    """Compute Ethereum Keccak-256 digest (32 bytes)."""
    rate = 136  # 1088 bits
    state = [0] * 25
    padded = bytearray(data)
    padded.append(0x01)
    while len(padded) % rate != 0:
        padded.append(0x00)
    padded[-1] |= 0x80

    for offset in range(0, len(padded), rate):
        block = padded[offset : offset + rate]
        for i in range(rate // 8):
            word = int.from_bytes(block[i * 8 : (i + 1) * 8], "little")
            state[i] ^= word
        _keccak_f1600(state)

    out = bytearray()
    for i in range(4):
        out.extend(state[i].to_bytes(8, "little"))
    return bytes(out)


def ethereum_address_from_verifying_key(vk: ecdsa.VerifyingKey) -> str:
    """Derive the 0x-prefixed 40-char lowercase Ethereum address from an uncompressed secp256k1 VerifyingKey."""
    pub_bytes_64 = vk.to_string()  # 64-byte uncompressed X || Y
    addr_bytes = keccak256(pub_bytes_64)[-20:]
    return "0x" + addr_bytes.hex()


def recover_eip712_signer_address(digest_32: bytes, signature_hex: str) -> str:
    """Recover the Ethereum signer address (`ecrecover`) from a 32-byte EIP-712 digest and 65-byte `r || s || v` signature."""
    if len(digest_32) != 32:
        raise ValueError("EIP-712 digest must be exactly 32 bytes")
    sig_clean = signature_hex.strip().removeprefix("0x")
    sig_bytes = bytes.fromhex(sig_clean)
    if len(sig_bytes) != 65:
        raise ValueError(f"Signature must be 65 bytes (got {len(sig_bytes)})")

    r_s = sig_bytes[:64]
    v_byte = sig_bytes[64]
    if v_byte in (27, 28):
        recovery_id = v_byte - 27
    elif v_byte in (0, 1):
        recovery_id = v_byte
    else:
        raise ValueError(f"Invalid Ethereum signature v byte: {v_byte}")

    candidates = ecdsa.VerifyingKey.from_public_key_recovery_with_digest(
        r_s,
        digest_32,
        curve=ecdsa.SECP256k1,
        sigdecode=sigdecode_string,
    )
    if recovery_id >= len(candidates):
        raise ValueError("Unable to recover public key for recovery_id")
    return ethereum_address_from_verifying_key(candidates[recovery_id])


# ---------------------------------------------------------------------------
# EIP-712 CTF Exchange V2 Constants & Encoders
# ---------------------------------------------------------------------------
ORDER_V2_TYPE_STRING = (
    "Order(uint256 salt,address maker,address signer,uint256 tokenId,"
    "uint256 makerAmount,uint256 takerAmount,uint8 side,uint8 signatureType,"
    "uint256 timestamp,bytes32 metadata,bytes32 builder)"
)
EIP712_DOMAIN_TYPE_STRING = (
    "EIP712Domain(string name,string version,uint256 chainId,address verifyingContract)"
)

ORDER_V2_TYPE_HASH = keccak256(ORDER_V2_TYPE_STRING.encode("ascii"))
EIP712_DOMAIN_TYPE_HASH = keccak256(EIP712_DOMAIN_TYPE_STRING.encode("ascii"))

SIGNATURE_TYPE_MAP = {
    SignatureType.EOA.value: 0,
    SignatureType.POLY_PROXY.value: 1,
    SignatureType.POLY_GNOSIS_SAFE.value: 2,
    SignatureType.POLY_1271.value: 3,
}

# Polymarket V2 6-decimal base units:
# - Maker USD notional (`BUY makerAmount` or `SELL takerAmount`) must be an integer multiple of 10^4 base units ($0.01 cents)
# - Share quantity (`BUY takerAmount` or `SELL makerAmount`) must have at most 5 decimals (multiple of 10^1 base units)
POLY_BASE_SCALE = Decimal("1000000")
CENT_BASE_UNIT_MODULUS = 10_000  # 10^(6 - 2) -> max 2 decimal places for USD notional
SHARE_BASE_UNIT_MODULUS = 10     # 10^(6 - 5) -> max 5 decimal places for share size


def validate_polymarket_order_amounts(
    *,
    side: str,
    maker_amount: int,
    taker_amount: int,
) -> None:
    """Validate that `makerAmount` and `takerAmount` satisfy Polymarket CLOB V2 precision & minimum rules."""
    if maker_amount <= 0 or taker_amount <= 0:
        raise ValueError("makerAmount and takerAmount must be positive integers")

    side_upper = side.upper()
    if side_upper == "BUY":
        usd_units = maker_amount
        share_units = taker_amount
        # Minimum marketable BUY order on Polymarket CLOB is $1.00 (1,000,000 base units)
        if usd_units < 1_000_000:
            raise ValueError(f"BUY makerAmount ({usd_units}) is below the $1.00 (1,000,000 units) minimum")
    elif side_upper == "SELL":
        share_units = maker_amount
        usd_units = taker_amount
    else:
        raise ValueError(f"Invalid order side: {side}")

    if usd_units % CENT_BASE_UNIT_MODULUS != 0:
        raise ValueError(
            f"USD amount ({usd_units} base units) violates Polymarket 2-decimal ($0.01) precision rule"
        )
    if share_units % SHARE_BASE_UNIT_MODULUS != 0:
        raise ValueError(
            f"Share amount ({share_units} base units) violates Polymarket 5-decimal precision rule"
        )


def _abi_uint256(val: int) -> bytes:
    return int(val).to_bytes(32, "big")


def _abi_address(addr: str) -> bytes:
    cleaned = addr.strip().lower().removeprefix("0x")
    raw = bytes.fromhex(cleaned)
    if len(raw) != 20:
        raise ValueError(f"Invalid Ethereum address `{addr}`")
    return b"\x00" * 12 + raw


def _abi_bytes32(val: Optional[str]) -> bytes:
    if not val:
        return b"\x00" * 32
    cleaned = val.strip().lower().removeprefix("0x")
    if len(cleaned) != 64:
        return b"\x00" * 32
    return bytes.fromhex(cleaned)


@dataclass(frozen=True)
class SignedPolymarketOrder:
    order_payload: dict[str, Any]
    eip712_hash_hex: str
    signature_hex: str
    recovery_v: int


def build_and_sign_v2_order(
    *,
    private_key_hex: str,
    maker_address: str,
    signer_address: str,
    signature_type: str,
    token_id: str,
    side: str,  # "BUY" | "SELL"
    price: Decimal,
    size_shares: Decimal,
    api_key_owner: str,
    order_type: str = "FAK",
    neg_risk: bool = False,
    chain_id: int = 137,
    verifying_contract: str = "0xE111180000d2663C0091e4f400237545B87B996B",
    salt: Optional[int] = None,
    timestamp_ms: Optional[int] = None,
) -> SignedPolymarketOrder:
    """Build, validate precision, and EIP-712 sign a Polymarket CTF Exchange V2 order."""
    if price <= Decimal("0") or size_shares <= Decimal("0"):
        raise ValueError("Order price and size_shares must be positive")

    size_scaled = size_shares * POLY_BASE_SCALE
    cost_scaled = price * size_shares * POLY_BASE_SCALE
    if size_scaled != size_scaled.to_integral_value() or cost_scaled != cost_scaled.to_integral_value():
        raise ValueError(
            f"Order amounts must be exact integers in 6-decimal base units (got size={size_scaled}, cost={cost_scaled})"
        )

    size_units = int(size_scaled)
    cost_units = int(cost_scaled)

    side_upper = side.upper()
    if side_upper == "BUY":
        maker_amount = cost_units
        taker_amount = size_units
        side_int = 0
    elif side_upper == "SELL":
        maker_amount = size_units
        taker_amount = cost_units
        side_int = 1
    else:
        raise ValueError(f"Invalid order side: {side}")

    # Fix P-03: Enforce Polymarket makerAmount/takerAmount precision before signing
    validate_polymarket_order_amounts(
        side=side_upper,
        maker_amount=maker_amount,
        taker_amount=taker_amount,
    )

    sig_type_int = SIGNATURE_TYPE_MAP.get(signature_type, 0)
    order_salt = salt if salt is not None else secrets.randbits(63)
    ts_ms = timestamp_ms if timestamp_ms is not None else int(datetime.now(timezone.utc).timestamp() * 1000)
    token_int = int(token_id, 16) if token_id.startswith("0x") else int(token_id)

    domain_name = "Polymarket Neg Risk CTF Exchange" if neg_risk else "Polymarket CTF Exchange"
    domain_separator = keccak256(
        EIP712_DOMAIN_TYPE_HASH
        + keccak256(domain_name.encode("ascii"))
        + keccak256(b"2")
        + _abi_uint256(chain_id)
        + _abi_address(verifying_contract)
    )

    metadata_b32 = b"\x00" * 32
    builder_b32 = b"\x00" * 32

    struct_hash = keccak256(
        ORDER_V2_TYPE_HASH
        + _abi_uint256(order_salt)
        + _abi_address(maker_address)
        + _abi_address(signer_address)
        + _abi_uint256(token_int)
        + _abi_uint256(maker_amount)
        + _abi_uint256(taker_amount)
        + _abi_uint256(side_int)
        + _abi_uint256(sig_type_int)
        + _abi_uint256(ts_ms)
        + metadata_b32
        + builder_b32
    )

    signing_digest = keccak256(b"\x19\x01" + domain_separator + struct_hash)

    pk_clean = private_key_hex.strip().removeprefix("0x")
    pk_bytes = bytes.fromhex(pk_clean)
    signing_key = ecdsa.SigningKey.from_string(pk_bytes, curve=ecdsa.SECP256k1)
    expected_key_addr = ethereum_address_from_verifying_key(signing_key.get_verifying_key())

    # Fix P-02: Verify that the private key actually corresponds to `signer_address`
    if expected_key_addr.lower() != signer_address.strip().lower():
        raise ValueError(
            f"Private key address ({expected_key_addr}) does not match signer_address ({signer_address})"
        )

    r_s_bytes = signing_key.sign_digest_deterministic(
        signing_digest,
        hashfunc=hashlib.sha256,
        sigencode=sigencode_string_canonize,
    )

    # Fix P-02: Compute the true secp256k1 recovery ID (0 -> v=27, 1 -> v=28)
    candidates = ecdsa.VerifyingKey.from_public_key_recovery_with_digest(
        r_s_bytes,
        signing_digest,
        curve=ecdsa.SECP256k1,
        sigdecode=sigdecode_string,
    )
    recovery_v: Optional[int] = None
    for rec_id, candidate_vk in enumerate(candidates):
        if ethereum_address_from_verifying_key(candidate_vk) == expected_key_addr:
            recovery_v = 27 + rec_id
            break

    if recovery_v is None:
        raise ValueError("Failed to determine valid secp256k1 recovery ID for EIP-712 signature")

    sig_65 = r_s_bytes + bytes([recovery_v])
    sig_hex = "0x" + sig_65.hex()

    # Self-verify recovered address matches expected signer before returning
    recovered_addr = recover_eip712_signer_address(signing_digest, sig_hex)
    if recovered_addr != expected_key_addr:
        raise ValueError("EIP-712 signature self-verification failed")

    wire_body = {
        "order": {
            "salt": order_salt,
            "maker": maker_address.lower(),
            "signer": signer_address.lower(),
            "tokenId": str(token_int),
            "makerAmount": str(maker_amount),
            "takerAmount": str(taker_amount),
            "side": side_upper,
            "expiration": "0",
            "signatureType": sig_type_int,
            "timestamp": str(ts_ms),
            "metadata": "0x" + ("00" * 32),
            "builder": "0x" + ("00" * 32),
            "signature": sig_hex,
        },
        "orderType": order_type,
        "owner": api_key_owner,
    }
    return SignedPolymarketOrder(
        order_payload=wire_body,
        eip712_hash_hex="0x" + signing_digest.hex(),
        signature_hex=sig_hex,
        recovery_v=recovery_v,
    )


# ---------------------------------------------------------------------------
# CLOB v2 L2 HMAC-SHA256 Authentication Headers
# ---------------------------------------------------------------------------
def compute_l2_hmac_signature(
    *,
    api_secret_b64: str,
    timestamp_str: str,
    method: str,
    path: str,
    body_str: Optional[str] = None,
) -> str:
    """Compute URL-safe Base64 HMAC-SHA256 signature for Polymarket CLOB v2 REST API."""
    cleaned_secret = api_secret_b64.strip()
    missing_padding = len(cleaned_secret) % 4
    if missing_padding:
        cleaned_secret += "=" * (4 - missing_padding)
    secret_bytes = base64.urlsafe_b64decode(cleaned_secret.encode("ascii"))

    message = f"{timestamp_str}{method.upper()}{path}"
    if body_str:
        message += body_str
    digest = hmac.new(secret_bytes, message.encode("utf-8"), hashlib.sha256).digest()
    return base64.urlsafe_b64encode(digest).decode("ascii")


def build_l2_auth_headers(
    *,
    poly_address: str,
    api_key: str,
    api_secret: str,
    api_passphrase: str,
    method: str,
    path: str,
    body_str: Optional[str] = None,
    timestamp_str: Optional[str] = None,
) -> dict[str, str]:
    ts = timestamp_str or str(int(datetime.now(timezone.utc).timestamp()))
    sig = compute_l2_hmac_signature(
        api_secret_b64=api_secret,
        timestamp_str=ts,
        method=method,
        path=path,
        body_str=body_str,
    )
    return {
        "POLY_ADDRESS": poly_address,
        "POLY_API_KEY": api_key,
        "POLY_SIGNATURE": sig,
        "POLY_TIMESTAMP": ts,
        "POLY_PASSPHRASE": api_passphrase,
    }
