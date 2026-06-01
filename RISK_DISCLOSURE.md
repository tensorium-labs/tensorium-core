# Risk Disclosure

**Tensorium (TXM) — Mainnet-Candidate Release v0.3.0**
**Published: 2026-05-31**

Please read this document carefully before running a node, mining, or acquiring TXM tokens.

---

## 1. Project Status

Tensorium is a **mainnet-candidate** blockchain. It is **not yet in production mainnet**. The current public chain is a GPU-first testnet (`tensorium-testnet-0`) for community testing and GPU mining evaluation.

- Testnet tokens have **no monetary value**.
- Mainnet-candidate launch requires GPU mining of the genesis block (40-bit difficulty).
- The mainnet-candidate genesis timestamp is tentatively set to **2026-06-01 00:00:00 UTC**, but may change.
- **No launch date is announced.** A launch date will only be set after this document and all items in `MAINNET_READINESS.md` are finalized.

---

## 2. Founder Allocation

- **Founder address:** `txm18c3t652j0x0sanux3dhse8fqgrqpsdzx97358d`
- **Founder allocation:** 1,000,000 TXM out of 33,000,000 TXM total supply (~3.03%)
- This allocation is included in the **genesis block** (block 0) and is **not earned through mining**.
- This is **not a fair launch**. Community members must evaluate this allocation and decide whether it is acceptable.

### Founder Lock Policy

The founder commits to a **voluntary 24-month lock** starting from mainnet genesis:

- No more than **10% of the allocation (100,000 TXM)** may be moved per calendar month for the first 24 months.
- After month 24, the remaining balance is fully movable at founder discretion.
- This policy is **social/reputational only** — it is **not enforced by L1 consensus**. The network does not technically prevent the founder from moving funds before the lock period ends.
- All movements from the founder address are visible on-chain via the public explorer (`explorer.tensoriumlabs.com`).
- Community members must decide whether they trust this voluntary lock.

---

## 3. Pool Fee

- The **official/reference mining pool** charges a **5% fee** on block rewards.
- The pool fee destination: `txm10wa2dazhn2yqwwxkm4aegvzjq55hj9m2jlznt9` (pool treasury wallet).
- This fee is **pool-level only**, not a protocol-level tax.
- **Solo mining is fee-free at the protocol level.** Miners who connect directly to a node receive 100% of the block reward.
- Miners using the official pool must accept the 5% fee. Third-party pools may charge different fees.

---

## 4. Technical Risks

### Consensus and Security
- Tensorium has not undergone a formal third-party security audit.
- The consensus code has been tested via unit tests (54+ tests passing) and public testnet operation, but may contain undiscovered vulnerabilities.
- The founder lock policy is social/manual — no smart contract or timelock enforces it.
- The RPC server is single-threaded and intended for localhost use. Public RPC exposure requires nginx rate-limiting.

### Storage
- Chain state is stored in JSON format. This is acceptable for testnet and early mainnet but may not scale to high transaction volumes. A migration to a binary/database format is planned for future versions.
- Users should maintain chain state backups.

### Mining
- Mainnet-candidate requires GPU mining (initial difficulty: 40 leading zero bits, ~2^40 hashes per block).
- CPU mining at mainnet-candidate difficulty is not practical.
- GPU mining requires an NVIDIA RTX 3060 or equivalent (sm86 CUDA architecture) for the included `txmminer-cuda` binary.
- Other GPU architectures may require compiling from source.

### Network
- Peer discovery uses a built-in static seed list (`157.230.44.162:23333`). If the seed node goes offline, new nodes cannot auto-connect without manually specifying a peer.
- DNS seed support is planned but not yet implemented.
- The testnet operates on a single seed node. Network decentralization requires community participation.

---

## 5. No Guarantees

- The software is provided **as-is**, without warranty of any kind.
- The founder does not guarantee that:
  - the network will reach mainnet launch,
  - TXM will have any monetary value,
  - the founder lock will be honored,
  - the project will continue indefinitely.
- Community members participate at their own risk.

---

## 6. Open Source

- Source code: [https://github.com/tensorium-labs/tensorium-core](https://github.com/tensorium-labs/tensorium-core)
- License: Apache-2.0.
- The code is publicly readable for review and audit purposes.

---

## 7. Contact and Community

- Website: [https://tensoriumlabs.com](https://tensoriumlabs.com)
- Docs: [https://docs.tensoriumlabs.com](https://docs.tensoriumlabs.com)
- Explorer: [https://explorer.tensoriumlabs.com](https://explorer.tensoriumlabs.com)
- Telegram: [https://t.me/+QOsnpSdhDGZkZGQ1](https://t.me/+QOsnpSdhDGZkZGQ1)
- GitHub Issues: [https://github.com/tensorium-labs/tensorium-core/issues](https://github.com/tensorium-labs/tensorium-core/issues)

---

*This disclosure is subject to update before the mainnet-candidate chain launches. The most recent version is always available at the GitHub repository.*
