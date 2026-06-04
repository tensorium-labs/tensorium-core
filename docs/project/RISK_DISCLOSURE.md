# Risk Disclosure

**Tensorium (TXM) — Mainnet-Candidate Release v0.3.1**
**Published: 2026-06-01**

Please read this document carefully before running a node, mining, or acquiring TXM tokens.

---

## 1. Project Status

Tensorium mainnet (`tensorium-mainnet-candidate-0`) is **live** as of 2026-06-02. Mining is active. TXM tokens on the mainnet chain may carry monetary value — participants assume full risk.

- **Mainnet genesis:** nonce `114_103_168_481`, hash `000000000063ab6f057a16376b1712e709719126ad977a3d4be23f83b89f0392`, timestamp `2026-06-01 00:00:00 UTC`
- **Bridge live:** TXM ↔ wTXM (Optimism) at https://bridge.tensoriumlabs.com
- No external security audit has been completed. Use at your own risk.

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
- The consensus code has been tested via unit tests and live chain operation, but may contain undiscovered vulnerabilities.
- The founder lock policy is social/manual — no smart contract or timelock enforces it.
- The RPC server is single-threaded and intended for localhost use. Public RPC exposure requires nginx rate-limiting.

### Storage
- Chain state now uses RocksDB persistence, but higher long-term transaction volume may still expose storage and operational scaling constraints.
- Users should maintain chain state backups.

### Mining
- Mainnet-candidate requires GPU mining (initial difficulty: 40 leading zero bits, ~2^40 hashes per block).
- CPU mining at mainnet-candidate difficulty is not practical.
- GPU mining requires an NVIDIA RTX 3060 or equivalent (sm86 CUDA architecture) for the included `tensorium-miner` binary.
- Other GPU architectures may require compiling from source.

### Network
- Peer discovery uses built-in DNS/static mainnet seeds (`seed.tensoriumlabs.com:33333`, `seed2.tensoriumlabs.com:33333`). If all seed nodes go offline, new nodes cannot auto-connect without manually specifying a peer.
- A backup seed node (`139.180.137.144`, Vultr) is operational for the mainnet-candidate. Network decentralization still requires broader community participation.

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

*This disclosure is subject to further operational updates. The most recent version is always available at the GitHub repository.*
