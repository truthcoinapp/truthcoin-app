# Verifying a Truthcoin App release

How to check that a release is what this repository's source says, and what that source does with your wallet. It
is written for you, or for an AI assistant you ask to review it (a prompt is below).

## What you can and cannot check

- **You can** read the source at the release's tag, check the release's signature and GitHub's build attestation,
  and rebuild the Linux program and `.deb` yourself to compare them byte for byte.
- **You trust** L2L's Truthcoin node, which the app downloads and runs (it checks the download against a pinned
  hash, so it runs only the build this source names), GitHub's Mac builders for the macOS app, and the operating
  system and webview you run it on.

## Step 1: get the source at the release tag

```sh
git clone https://github.com/mblowes/truthcoin-app && cd truthcoin-app
git checkout v<version>
```

## Step 2: review the source

A [Tauri 2](https://tauri.app) app. The Rust side in `src-tauri/src/` does everything that touches the node, keys,
files, processes and the network; the screens in `src/` (Svelte) ask it through Tauri commands, all registered in
`src-tauri/src/lib.rs`. `<app data>` below is `~/.local/share/dev.truthcoinapp.desktop` on Linux and
`~/Library/Application Support/dev.truthcoinapp.desktop` on macOS. Every file the app writes there is readable by
your user only (`src-tauri/src/files.rs`).

**The node** (`src-tauri/src/node/`):
- `pins.rs`: the one node release the app runs, by file name, size and SHA-256 for each computer.
- `install.rs`: downloads it from `github.com/LayerTwo-Labs/truthcoin-dc/releases/download/v<node version>/…`,
  hashes it as it arrives, and keeps it (as `<app data>/bin/truthcoin_dc-<node version>`) only if size and hash
  match. It is hashed again before every start.
- `mod.rs`: starts it as the app's child with `--datadir <app data>/node`, the read-only RPC on `127.0.0.1:16013`,
  the wallet and node-control calls on a private port picked at random at each start (`--private-rpc-port`; on Linux
  on a random address in `127.0.0.0/8`, elsewhere 127.0.0.1), P2P on `0.0.0.0:14013`, ZMQ on `127.0.0.1:16015` (all in Settings › Advanced), and the enforcer's gRPC
  (`127.0.0.1:50051` by default). `<app data>/node.json` records its pid, program and ports, so a later launch can
  stop a node a crash left behind (only if that pid still runs the same program). On Linux the node is told to stop
  if the app dies; on every system it is stopped when the app quits. The node gets the app's environment without
  `RUST_LOG`, at `--log-level info` (at trace level it would log RPC requests, recovery words included). One copy of
  the app runs per data folder (`<app data>/lock`).

**Signature:** the release's `SHA256SUMS` is signed with `ssh-keygen -Y sign -n truthcoinapp-sums`; the update check
(`src-tauri/src/update.rs`) refuses any other namespace, any other key, and URLs outside this repository.

**eCash beta only** (`src-tauri/src/settings.rs`, `BETA_ONLY`, true in every release build): the Truthcoin network is
forced to `betanet`, the node program to the pinned release with no extra arguments, whatever `settings.json` says;
and before every start `node/enforcer.rs` (`check_ecash_beta`) asks the enforcer for eCash beta's fork block
(`00000000000000030101ba5cfea54b22becc79f95dc6040beb76e01dd9d04042` at height 967,680) and refuses any other chain.

**Where the secrets are:**
- **The wallet's seed** is the node's: it stores it unencrypted in `<app data>/node/` (L2L's code; the app can't
  change that). The app asks the node for new words (`generate_mnemonic`), holds them in memory only
  (`src-tauri/src/wallet.rs`, `Zeroizing`) until three are typed back, then sets them (`set_seed_from_mnemonic`) and
  forgets them. It never writes them to disk or logs them, and the node has no call to read them back. A restore
  sets the typed words the same way. The node refuses to replace a seed once set.
- **The phone link's keys** are in `<app data>/phone/keys.json`: the desktop's P-256 key and its Nostr key (random,
  unrelated to any Nostr identity of yours). `devices.json` holds each paired phone's public keys and limit,
  `answers.json` each phone trade request's id and answer (24 hours), `held.json` trades waiting for you. Crypto:
  `src-tauri/src/phone/crypto.rs`; Nostr events: `nostr.rs`; relay connections: `relays.rs`; what a phone may do:
  `mod.rs`.
- **Your trades:** `<app data>/trades.json`, the app's own record (each trade is written there before it goes to the
  node). `<app data>/deposits.json` lists deposits from eCash sent in the last two hours (amount, time, eCash txid),
  so Home can show them arriving. `<app data>/activity.log` notes what the app did, without words, keys or full
  addresses. `<app data>/wallet-ready` marks a folder whose wallet was set up. Any of these that can't be read is set
  aside as `*.bad-*`, never written over, and phones can't trade until you say you've looked.

**Every network contact:**
- The node's RPC and the enforcer's gRPC, on this computer (or where Settings › Advanced points).
- `github.com` (and its download host): the node's release, when you press Install.
- `api.github.com/repos/mblowes/truthcoin-app/releases/latest` and that release's `SHA256SUMS` and `SHA256SUMS.sig`:
  only when you press "Check for a newer version" (`src-tauri/src/update.rs`).
- The Nostr relays in Settings › Phone (three public ones by default), over `wss://`, only while a phone is paired
  or pairing is under way (`src-tauri/src/phone/relays.rs`).
- Links the app opens in your browser go through the shell plugin, limited by the `open` pattern in
  `src-tauri/tauri.conf.json` to this repository's pages, L2L's release page and the phone page.
- The screens themselves can reach nothing else: the content security policy in `tauri.conf.json` is
  `default-src 'self'`.
- The node itself talks to Truthcoin peers (P2P) and to the enforcer; that is L2L's program.

**What the app asks the node** (the node's calls, by port):
- Read-only port: `getblockcount`, `mainchain_sync_progress`, `list_peers`, `market_list`, `market_get`,
  `market_positions`, `list_mempool`, `get_transaction_info`, `decision_status`, `list_open_periods_with_pricing`,
  `calculate_initial_liquidity`, `get_utxos`, `get_stxos`.
- Private port: `bitcoin_balance`, `get_wallet_addresses`, `get_wallet_utxos`, `get_new_address`,
  `get_voter_address`, `generate_mnemonic`, `set_seed_from_mnemonic`, `refresh_wallet`, `market_buy` and
  `market_sell` (quotes with `dry_run`, then trades with a cap), `market_create`, `transfer_many` (only to the
  wallet's own new addresses: "split coins"), `remove_from_mempool` (cancelling your own stuck trade), `withdraw`
  (to an eCash address you type), `stop`.
- Nothing else: there is no generic "call the node" command.

**What a phone can do** (`src-tauri/src/phone/mod.rs`, `docs/PROTOCOL.md`): pair only with a live, single-use code
and your yes on the desktop, after you compare a code on both screens; then `status`, `markets`, `market`,
`positions`, `balance`, `quote`, `trade`, `trades`, `receive`. Every trade counts against the phone's daily limit (a
rolling 24 hours; every trade that may have reached the node counts, cancelled ones too): a buy at its cap, a sell at
its number of shares (a sat each: the most they can pay, so nothing in the market can lower it). A trade over what's left waits for you on the desktop, for up
to an hour; a phone can have at most 3 waiting for you and 3 waiting for their block. A buy's cap must be at least the
quote, the miner fee and half the margin, and at most twice the suggested cap. A sell is refused if the price
movement alone would lose over 20% of its shares' value, or if its minimum is more than one margin below the
suggested one. Every request carries an id: a repeat gets the
stored answer and never runs twice. Requests more than 5 minutes off the desktop's clock are refused. A phone may make 60
new requests a minute and get 30 repeated answers; over that, a new request isn't run, and the phone is told (at most
5 times a minute) to ask again. Messages that don't open with a paired phone's keys are dropped; events
from keys that aren't paired aren't even checked unless a pairing code is live. If the app can't read its trade or
phone records at start, it sets them aside and phones can't trade until someone looks.

**Third-party code:** the Rust crates are pinned by `src-tauri/Cargo.lock`, the npm packages by `package-lock.json`
and `phone/package-lock.json`. The QR encoder is the app's own (`src/lib/qr.ts`), as is the Nostr client.

### A prompt for an AI reviewer

Give your assistant the checked-out tree and something like this:

> You are reviewing the Truthcoin App, a Tauri 2 desktop app (Rust in `src-tauri/src/`, Svelte screens in `src/`)
> with a phone web page (`phone/`), at commit `<commit>`, for a person deciding whether to run it. It downloads and
> runs L2L's Truthcoin node, whose wallet holds the person's coins, and lets a paired phone trade within a limit.
> VERIFY.md, step 2, maps the files, every network contact, what the app asks the node and what a phone can do. Check
> that map against the code, then look for:
> (1) the recovery words, keys or wallet files leaving the computer, being logged, or being written anywhere the map
> doesn't say;
> (2) any network contact not in the map, including from the screens (the content security policy and the shell
> plugin's `open` pattern in `src-tauri/tauri.conf.json`, and `src-tauri/capabilities/`);
> (3) node calls that could move coins or change the wallet without the person asking, through any Tauri command in
> `src-tauri/src/lib.rs`;
> (4) anything that lets a paired phone, a relay or a web page do more than the documented limits
> (`src-tauri/src/phone/`, `docs/PROTOCOL.md`), including replays, forged senders, and getting past the daily limit;
> (5) whether the node program is checked against its pin before it is run, and anything else started or downloaded
> from untrusted input;
> (6) in `phone/`: secrets leaving the page, third-party requests, and anything a desktop or relay could make the page
> show or do beyond its screens;
> (7) dependencies in the lockfiles that look out of place.
> For each finding give the file and line, what an attacker needs, and the impact. Say plainly what you did not check.

## Step 3: check the signature and GitHub's attestation

- **Signature:** `SHA256SUMS` is signed with this app's release key: the key and the commands are in the README,
  under [Verify your download](README.md#verify-your-download).
- **Attestation:** the release workflow builds each tag on GitHub, which records a signed attestation naming the
  workflow, the tag and each package's hash:

  ```sh
  gh attestation verify truthcoin-app_<version>_amd64.deb --repo mblowes/truthcoin-app \
    --signer-workflow mblowes/truthcoin-app/.github/workflows/release.yml --source-ref refs/tags/v<version>
  ```

## Step 4: rebuild the Linux package yourself

The Linux program and `.deb` are built in a pinned image (`build/linux/`), so the same tag builds the same bytes:

```sh
build/linux/rebuild.sh v<version>      # prints the hashes; compare them with SHA256SUMS
```

## What this does not cover

- **The AppImage and the Mac app** don't rebuild byte for byte yet; for those you trust GitHub's builders, which the
  attestation names. The Mac app isn't notarised.
- **L2L's Truthcoin node** is L2L's program. The app runs only the build whose hash it pins, but that pin says "the
  file L2L published for v<node version>", not that the program is safe. The node has no RPC login and stores its seed
  unencrypted.
- **The phone page** is served by GitHub Pages from this repository's tagged source (`.github/workflows/pages.yml`);
  whoever controls those files controls what the page shows and asks, within the phone's limit. Its origin,
  `https://mblowes.github.io`, is shared by every Pages site of the `mblowes` account, and the phone's keys live in
  that origin's storage: so no other repository under `mblowes` may enable Pages, and `mblowes/mblowes.github.io`
  must never exist. (Moving the page to an origin of its own later would make every phone pair again.)
- **eCash, the enforcer and BitWindow** are not covered here.
