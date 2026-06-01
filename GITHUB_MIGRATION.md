# Tensorium GitHub Migration

Target organization: `tensorium-labs`

Status: the namespace is available, but GitHub organization creation must be
completed once through GitHub's web flow by the owner account. GitHub does not
currently expose normal organization creation through `gh org` or the public
REST API used by this workspace.

## Recommended Setup

- Organization name: `tensorium-labs`
- Display name: `Tensorium Labs`
- Public email: `dev@tensoriumlabs.com`
- Website: `https://tensoriumlabs.com`
- Billing plan: Free is enough for public repositories.
- Require 2FA for owners and maintainers.

## Repositories To Transfer

- `rygroup-dev/tensorium-core` -> `tensorium-labs/tensorium-core`
- `rygroup-dev/tensorium-pool-website` -> `tensorium-labs/tensorium-pool-website`

## After Organization Exists

Transfer with GitHub CLI/API or GitHub web UI, then update local remotes:

```bash
git -C /root/.openclaw/workspace/tensorium-core remote set-url origin https://github.com/tensorium-labs/tensorium-core.git
git -C /root/.openclaw/workspace/tensorium-pool-website remote set-url origin https://github.com/tensorium-labs/tensorium-pool-website.git
```

Then update public links in:

- `README.md`
- `CONTRIBUTING.md`
- `RISK_DISCLOSURE.md`
- `install.sh`
- `CHANGELOG.md`
- `Cargo.toml`
- `myProject_PoW.md`

Do not announce launch date during this migration. Launch date remains the last
step after readiness gates, soak test, monitoring, and final checks.
