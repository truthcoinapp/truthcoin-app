# A private Truthcoin chain for development

Checked 2026-10-04 on Linux with truthcoin_dc v0.19.0, eCash betanet bitcoind v31.1.0 and L2L's
enforcer (`73d239a` build); since 2026-10-08 with truthcoin_dc v0.20.0 (L2L's release), which the app runs from
then on.

```
dev/stack.sh                       # ~70 s: L1 + enforcer + slot 13 + truthcoin_dc, 10 coins deposited
dev/lifecycle.sh <TC_WORK>/stack.env   # ~3 min: create a market, buy, sell, vote, settle, paid out
```

- `stack.sh` prints where its `stack.env` is. `source` it in another shell for `tc METHOD '[params]'` (JSON-RPC to
  truthcoin_dc), `bmm [N]` (N Truthcoin blocks) and `l1 N` (eCash blocks only).
- Stop it with `kill <pid>` (printed) or Ctrl-C. It stops every daemon and deletes its folder (`TC_KEEP=1` keeps it).
- Ports: a free block of 10 from 23400. Everything on 127.0.0.1.
- Decision periods are 10 blocks (`--decision-config-testing 10`): a decision for period 3 is voted on at heights
  30-40 and settles at 41.
- The wallet's voter address holds all the votecoin (1.0) on regtest, so one vote decides.
- `dev/bin/` holds L2L's release binaries (`gh release download v<version> -R LayerTwo-Labs/truthcoin-dc`); keep it
  out of git. The stack and the app's tests run `truthcoin-0.20.0-x86_64-unknown-linux-gnu` (or `TC_BIN` /
  `TC_NODE_BIN`). `truthcoin-0.19.0-x86_64-unknown-linux-gnu` (sha256
  `31604f5e306bca15b38df27c8ca454f87acc4fb435350f0e16bf22347b47a838`) stays for one test: `realnode_from_019_to_020`
  runs it to leave a 0.19 folder, then the app's 0.20 node on it.
- Needs, in `dev/bin/` (kept out of git): `ecash/` holding eCash's `bitcoind` and `bitcoin-cli` (the betanet build,
  v31.1.0, as BitWindow downloads it), `bip300301_enforcer` (L2L's enforcer), and `~/.local/bin/grpcurl` for the
  enforcer's gRPC. Override with `TC_L1_BIN` (the folder with bitcoind), `TC_ENFORCER`, `TC_GRPCURL`, `TC_BIN`.
- `TC_NO_NODE=1 dev/stack.sh` starts eCash and the enforcer only (what BitWindow gives a user), for the app to run its
  own node on; `stack.env` then has `tcapp` and `appbmm` (calls and blocks through the app's node, found through
  `$TRUTHCOIN_APP_DIR/node.json`). The app's `realnode` test runs on it (`../src-tauri/src/realnode.rs`).

Truthcoin's own `create_deposit` call doesn't deposit from eCash (in v0.19.0 it makes a transfer inside the Truthcoin
wallet); the stack deposits with the enforcer's `CreateDepositTransaction`, as the app does.

Never point tests at the user's real Truthcoin data folder (`~/.local/share/com.layertwolabs.truthcoin`) or its RPC
on 6013.
