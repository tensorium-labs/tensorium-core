# Phase 9A Lost Key Procedure

Status: emergency procedure for lost or inaccessible keys.
Last updated: 2026-06-01

## Keys in Phase 9A

| Key | Location | Purpose | Impact if Lost |
|---|---|---|---|
| Owner EOA | Deployer wallet (local) | Pause/unpause, set operator | Cannot pause bridge, cannot update operator |
| Operator hot wallet | DO VPS `.env` | Mint wTXM on Optimism | Deposits not processed until new operator set |
| TXM custody key | VPS `.tensorium-bridge/` | Release TXM on withdrawals | Withdrawals cannot be processed |

## Case 1 — Lost Operator Key

**Impact:** Deposits cannot be minted as wTXM. Withdrawals still work (burn side).

**Recovery:**
1. Generate new operator address
2. Call `controller.setOperator(newOperator, true)` from owner EOA
3. Call `controller.setOperator(oldOperator, false)` to revoke old
4. Update `/root/tensorium-bridge-relayer/.env` on VPS with new `OPERATOR_PRIVATE_KEY`
5. `pm2 restart tensorium-bridge-relayer`
6. Test: verify deposit is processed

**Time to recover:** < 30 minutes if owner key is accessible.

## Case 2 — Lost TXM Custody Key

**Impact:** Withdrawal releases (TXM side) cannot be processed. wTXM holders cannot redeem TXM.

**Immediate actions:**
1. Pause bridge immediately: `controller.pause()`
2. Post on Telegram: "Bridge paused — withdrawal processing temporarily suspended."
3. Inventory: how much wTXM is outstanding? (check `token.totalSupply()`)
4. Inventory: what is at the custody address? (check `/getutxos/<custody>` on MC RPC)

**If custody funds are recoverable (key partially accessible):**
- Use any remaining access to sweep custody funds to a new address
- Deploy new custody address
- Update `CUSTODY_ADDRESS` in relayer `.env`
- Resume bridge operations with new custody

**If custody funds are unrecoverable:**
- This is a critical incident. wTXM is unbacked.
- Post full public disclosure on Telegram immediately
- Do NOT unpause the bridge
- Attempt to compensate affected users manually if possible

**Prevention:** custody key backup MUST exist offline. See PHASE9A_SIGNER_CUSTODY_LAYOUT.md.

## Case 3 — Lost Owner EOA Key

**Impact:** Cannot pause bridge, cannot update operator or pauser.

**Immediate actions:**
1. Assess risk: is bridge currently exploitable without pause access?
2. If operator key still works: revoke it to prevent new mints
   - This requires the owner key — if truly lost, this is not possible
3. Contact Optimism team if contract upgrade/rescue is possible (unlikely for Ownable contracts)

**Honest assessment for Phase 9A:**
If the owner EOA key is lost and bridge is unpaused, the bridge continues operating
but cannot be stopped. Deploy new contracts and redirect all users immediately.

**Recovery:**
1. Deploy new WrappedTensorium + TensoriumBridgeController
2. Update bridge.tensoriumlabs.com with new addresses
3. Post public announcement
4. Handle migration of existing wTXM holders manually

## Case 4 — Lost VPS Access (DO or Vultr)

**Impact:** Relayer bot stops. Deposits queue up but are not processed.

**Recovery:**
1. Regain VPS access via console (DigitalOcean/Vultr web console)
2. `pm2 restart tensorium-bridge-relayer`
3. Relayer will process any backed-up deposits automatically on restart
4. No funds are at risk — relayer just needs to be restarted

## Key Backup Requirements

Every key involved in Phase 9A bridge operations MUST have:
- At least one offline backup (encrypted, separate location from primary)
- Tested recovery: confirm backup can actually sign transactions before going live
- Not stored on any public-facing VPS

Backup verification checklist:
- [ ] Owner EOA: backup stored offline, recovery tested
- [ ] TXM custody key: backup stored offline, recovery tested
- [ ] Operator key: backup stored offline (lower risk, but still required)
