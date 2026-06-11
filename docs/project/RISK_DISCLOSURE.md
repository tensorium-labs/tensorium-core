# Risk Disclosure

**Tensorium (TXM) — Mainnet v1 Risk Disclosure**
**Updated: 2026-06-11**

Please read this document carefully before running a node, mining, or acquiring TXM tokens.

---

## 1. Project Status

Tensorium mainnet v1 (`tensorium-mainnet`) is **live** as of 2026-06-11. Mining is active. TXM tokens on the mainnet chain may carry monetary value — participants assume full risk.

- **Mainnet v1 genesis:** nonce `9_223_372_445_780_809_059`, timestamp `2026-06-11 04:14:52 UTC`
- **Bridge live:** TXM ↔ wTXM (Optimism) at https://bridge.tensoriumlabs.com
- No external security audit has been completed. Use at your own risk.

---

## 2. Supply and Launch Model

- **Mainnet v1 uses a zero-premine launch.**
- **Founder allocation in genesis:** `0 TXM`
- The full 33,000,000 TXM max supply is emitted through mining according to the halving schedule.
- Community members should treat this as a fresh-chain relaunch under new consensus parameters (`TensorHash v1`).

---

## 3. Pool Fee

- The **official/reference mining pool** charges a **5% fee** on block rewards.
- The pool fee destination: `txm1px2nmtp087mz8dv3lplqadwzxawk0c5kg0mt24` (pool treasury wallet).
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
- Mainnet v1 requires TensorHash v1 GPU mining (initial difficulty: 42 leading zero bits, ~2^42 work target at launch).
- CPU mining at mainnet v1 difficulty is not practical.
- GPU mining requires an NVIDIA GPU with 24 GB+ VRAM for the included `tensorium-miner` binary.
- Other GPU architectures may require compiling from source.

### Network
- Peer discovery uses the built-in mainnet seed `seed.tensoriumlabs.com:33333`. If the seed node goes offline, new nodes may need a manually specified peer.
- Public RPC is exposed at `https://rpc.tensoriumlabs.com`. Legacy alias `https://mc-rpc.tensoriumlabs.com` currently points to the same backend.

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
