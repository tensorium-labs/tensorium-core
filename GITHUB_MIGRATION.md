# Tensorium GitHub Migration

Target namespace: `tensorium-labs`

Status: `tensorium-labs` is a GitHub user account controlled by the project.
The old `rygroup-dev` repositories were not transferred through GitHub's
transfer API because GitHub reported the target repository names as taken. The
migration path used instead is to create fresh repositories under
`tensorium-labs`, push the full local Git history, then update remotes and
public links.

## Account Setup

- Username: `tensorium-labs`
- Display name: `Tensorium Labs`
- Public email: `dev@tensoriumlabs.com`
- Website: `https://tensoriumlabs.com`
- Enable 2FA before final public launch.

## Repositories

- `https://github.com/tensorium-labs/tensorium-core`
- `https://github.com/tensorium-labs/tensorium-pool-website`

## Local Remotes

Local remotes should point to the Tensorium namespace:

```bash
git -C /root/.openclaw/workspace/tensorium-core remote set-url origin https://github.com/tensorium-labs/tensorium-core.git
git -C /root/.openclaw/workspace/tensorium-pool-website remote set-url origin https://github.com/tensorium-labs/tensorium-pool-website.git
```

## Public Links

Public links in README, CONTRIBUTING, RISK_DISCLOSURE, install script,
CHANGELOG, Cargo metadata, pool website README, and blueprint should point to
`github.com/tensorium-labs`.

Do not announce launch date during this migration. Launch date remains the last
step after readiness gates, soak test, monitoring, and final checks.
