#!/usr/bin/env python3
"""
Convert snarkjs proof.json + verification_key.json into stellar CLI argument JSON
for the BN254 Groth16 verifier contract.

Soroban BN254 serialization (Ethereum-compatible):
  G1 (64 bytes): be_bytes(X) || be_bytes(Y)
  G2 (128 bytes): be_bytes(X_c1) || be_bytes(X_c0) || be_bytes(Y_c1) || be_bytes(Y_c0)
  Fr (U256): 32-byte big-endian
"""
import json
import sys

def fq_to_hex(s):
    """Convert a decimal string field element to 32-byte big-endian hex."""
    val = int(s)
    return val.to_bytes(32, 'big').hex()

def g1_to_hex(x, y):
    """G1 = be_bytes(X) || be_bytes(Y) = 64 bytes."""
    return fq_to_hex(x) + fq_to_hex(y)

def g2_to_hex(x_c0, x_c1, y_c0, y_c1):
    """G2 = be_bytes(X_c1) || be_bytes(X_c0) || be_bytes(Y_c1) || be_bytes(Y_c0) = 128 bytes."""
    return fq_to_hex(x_c1) + fq_to_hex(x_c0) + fq_to_hex(y_c1) + fq_to_hex(y_c0)

def fr_to_hex(s):
    """Fr = U256 big-endian = 32 bytes."""
    val = int(s)
    return val.to_bytes(32, 'big').hex()

def main():
    build_dir = sys.argv[1] if len(sys.argv) > 1 else "../circuits/build"

    with open(f"{build_dir}/multiplier2_vkey.json") as f:
        vk = json.load(f)
    with open(f"{build_dir}/multiplier2_proof.json") as f:
        proof = json.load(f)
    with open(f"{build_dir}/multiplier2_public.json") as f:
        public = json.load(f)

    # Build VerificationKey
    vk_arg = {
        "alpha": g1_to_hex(vk["vk_alpha_1"][0], vk["vk_alpha_1"][1]),
        "beta": g2_to_hex(vk["vk_beta_2"][0][0], vk["vk_beta_2"][0][1],
                          vk["vk_beta_2"][1][0], vk["vk_beta_2"][1][1]),
        "gamma": g2_to_hex(vk["vk_gamma_2"][0][0], vk["vk_gamma_2"][0][1],
                           vk["vk_gamma_2"][1][0], vk["vk_gamma_2"][1][1]),
        "delta": g2_to_hex(vk["vk_delta_2"][0][0], vk["vk_delta_2"][0][1],
                           vk["vk_delta_2"][1][0], vk["vk_delta_2"][1][1]),
        "ic": [g1_to_hex(ic[0], ic[1]) for ic in vk["IC"]],
    }

    # Build Proof
    proof_arg = {
        "a": g1_to_hex(proof["pi_a"][0], proof["pi_a"][1]),
        "b": g2_to_hex(proof["pi_b"][0][0], proof["pi_b"][0][1],
                       proof["pi_b"][1][0], proof["pi_b"][1][1]),
        "c": g1_to_hex(proof["pi_c"][0], proof["pi_c"][1]),
    }

    # Build public signals (Vec<Fr>)
    pub_arg = [fr_to_hex(s) for s in public]

    # Output as stellar CLI argument JSON
    args = {
        "vk": vk_arg,
        "proof": proof_arg,
        "pub_signals": pub_arg,
    }

    print(json.dumps(args))

if __name__ == "__main__":
    main()
