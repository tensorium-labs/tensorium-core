# Contributing to Tensorium

Thank you for your interest in contributing to Tensorium.

## Ways to Contribute

- **Report bugs** — open a [Bug Report](https://github.com/rygroup-dev/tensorium-core/issues/new?template=bug_report.yml)
- **Report mining problems** — open a [Mining Problem](https://github.com/rygroup-dev/tensorium-core/issues/new?template=mining_problem.yml)
- **Suggest features** — open a [Feature Request](https://github.com/rygroup-dev/tensorium-core/issues/new?template=feature_request.yml)
- **Submit a PR** — for small fixes, open a PR directly; for larger changes, open an issue first to discuss

## Development Setup

```bash
git clone https://github.com/rygroup-dev/tensorium-core.git
cd tensorium-core
cargo build
cargo test
```

Requires Rust ≥ 1.76. All 24 unit tests must pass before a PR can merge.

## Code Guidelines

- Run `cargo clippy` and fix warnings before submitting
- Run `cargo test` — all tests must pass
- Keep commits focused: one logical change per commit
- Write commit messages in imperative form: `Fix genesis timestamp` not `Fixed genesis timestamp`

## Pull Request Process

1. Fork the repo and create a branch from `main`
2. Make your changes
3. Ensure `cargo test` passes
4. Open a PR with a clear description of what changed and why
5. PRs require review before merging

## Scope

Tensorium is in **public testnet** (Phase 4). We are actively looking for:

- Consensus bugs
- P2P edge cases
- Wallet signing issues
- Miner reliability problems
- Documentation improvements

We are **not** looking for:
- Mainnet parameter changes (too early)
- Tokenomics modifications
- Breaking consensus changes without prior discussion

## Resources

- [Documentation](https://docs.tensoriumlabs.com)
- [Mining Guide](https://docs.tensoriumlabs.com/mining.html)
- [Whitepaper](https://whitepaper.tensoriumlabs.com)
- [Block Explorer](https://explorer.tensoriumlabs.com)
