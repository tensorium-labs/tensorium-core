# Tensorium GitHub Migration

Target namespace: `tensorium-labs`

Status: `tensorium-labs` is a GitHub user account controlled by the project.
The old `rygroup-dev` repositories were not transferred through GitHub's
transfer API because GitHub reported the target repository names as taken. The
migration path used instead is to create fresh repositories under
`tensorium-labs`, push the full local Git history, then update remotes and
public links.

Legacy repositories under `rygroup-dev` were set back to private after the
Tensorium namespace push completed.

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

## Working Order

Use this order for all future Tensorium work:

1. Edit locally in `/root/.openclaw/workspace`.
2. Run the relevant local checks:
   - Core: `cargo test --workspace`
   - Pool website: `npm run typecheck && npm run lint && npm run build && npm audit --omit=dev`
3. Commit and push to the new GitHub namespace under `tensorium-labs`.
4. Deploy/sync the VPS from the `tensorium-labs` remote.
5. Run VPS/service smoke checks and update local progress docs.

Current VPS decision:

- Use the existing DigitalOcean VPS as the temporary mainnet-candidate host.
- Treat local Git + `tensorium-labs` GitHub as the source of truth.
- When a new dedicated VPS is ready, migrate by cloning from
  `tensorium-labs`, copying only the required env/secret files, rebuilding,
  syncing state/backups as needed, then switching DNS.
- Until migration, every production-style update follows:
  local edit -> local checks -> push `tensorium-labs` -> deploy to current VPS
  -> smoke check services.

Do not put raw VPS passwords, mailbox passwords, API keys, or GitHub tokens in
Git-tracked files or project notes. Use root-only files or rotate credentials
after temporary use.

## Public Links

Public links in README, CONTRIBUTING, RISK_DISCLOSURE, install script,
CHANGELOG, Cargo metadata, pool website README, and blueprint should point to
`github.com/tensorium-labs`.

Do not announce launch date during this migration. Launch date remains the last
step after readiness gates, soak test, monitoring, and final checks.
