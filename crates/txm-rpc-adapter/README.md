# txm-rpc-adapter — Bitcoin-core-style JSON-RPC for Tensorium (TXM)

A coin-daemon adapter that lets exchanges (SafeTrade, etc.) automate TXM
deposits & withdrawals using the standard Bitcoin-core RPC surface, wrapping a
running `tensorium-node` + the core wallet crypto.

## Methods
`getblockcount` · `getbestblockhash` · `getblockhash` · `getblock` ·
`getblockchaininfo` · `getnetworkinfo` · `getwalletinfo` · `getnewaddress` ·
`validateaddress` · `getbalance` · `sendtoaddress` · `listsinceblock` ·
`listtransactions` · `gettransaction` · `estimatesmartfee` · `settxfee`

Deposits: a background scanner walks blocks and records outputs paid to managed
addresses into a ledger (`listsinceblock`/`listtransactions`/`gettransaction`).
Withdrawals: `sendtoaddress` selects mature UTXOs across all managed addresses,
signs each input with its key (core crypto), and broadcasts via the node.

## Run
```
ADAPTER_BIND=127.0.0.1:8332 \
NODE_RPC=http://127.0.0.1:33332 \
RPC_USER=exchange RPC_PASS=<strong> \
WALLET_PATH=/var/lib/txm-adapter/wallet.json \
LEDGER_PATH=/var/lib/txm-adapter/ledger.json \
FEE_ATOMS=10000 \
txm-rpc-adapter
```
Auth: HTTP Basic (`RPC_USER`/`RPC_PASS`) — like bitcoind's rpcuser/rpcpassword.
Amounts are decimal TXM (8 dp). The keystore is HOT — secure the host, keep only
operational balances, sweep to cold storage.
