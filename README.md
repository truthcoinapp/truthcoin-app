# Truthcoin App

A desktop app for your own Truthcoin wallet: it runs a Truthcoin node on your computer, shows the prediction markets,
and lets you trade, create markets and deposit from eCash. A phone page pairs with it by QR code, so you can see
markets and positions and trade from your phone, within a daily limit you set. The phone reaches the desktop through
public Nostr relays, sealed end to end. **Nobody hosts anything for it.**

Truthcoin is L2L's prediction-market sidechain (slot 13) on eCash. This app runs L2L's own node,
[`truthcoin_dc`](https://github.com/LayerTwo-Labs/truthcoin-dc), and talks to it over its JSON-RPC. **It is not made
by L2L**, and it takes no fees.

**Releases run only on eCash beta.** A release build runs the Truthcoin node on its beta network and starts it only
when the enforcer follows eCash beta (its chain has beta's fork block, 967,680); it can't be pointed at another
chain. (Development builds can use the private dev chain in `dev/`.)

**v0.1.0 is an early release.** It has had automated security reviews, not a professional audit. Keep only what you
are trading in it.

## What you need

- **Linux (x86-64) or macOS.** L2L builds its node for those.
- **BitWindow running eCash**, which also starts the enforcer the Truthcoin node follows (on `127.0.0.1:50051`). The
  app doesn't install or run eCash or the enforcer; it runs everything else.

## Install

Download the package for your computer from [Releases](https://github.com/mblowes/truthcoin-app/releases):

- **Debian or Ubuntu:** `truthcoin-app_<version>_amd64.deb` (`sudo apt install ./truthcoin-app_<version>_amd64.deb`).
- **Other Linux:** `Truthcoin-App_<version>_amd64.AppImage` (make it executable, then run it).
- **macOS:** `Truthcoin-App_<version>_universal.dmg`. The app isn't notarised yet: the first time, open it from
  Applications with right-click › Open, and allow it.

Check your download first: [Verify your download](#verify-your-download).

## First run

1. With BitWindow's eCash running, open the app. It finds the enforcer.
2. **Install:** it downloads L2L's Truthcoin node for your computer from L2L's GitHub release (about 56 MB) and keeps
   it only if its SHA-256 is the one pinned in this app (`src-tauri/src/node/pins.rs`). L2L publishes no checksums
   or signatures, so the pin is this app's own check, and this app's signed release vouches for it.
3. **Start:** the node runs as the app's child, in the app's own data folder, on its own ports (it never clashes with
   a Truthcoin that BitWindow runs). It stops when the app quits.
4. **Wallet:** new recovery words, shown once (the node can't show them again, and the app keeps no copy), with three
   asked back; or your old words, after which the app looks through the chain for the wallet's coins.
5. **Deposit** from BitWindow's eCash wallet (the enforcer's) on Home. It arrives after the deposit's eCash block and
   the Truthcoin block that follows.

## Using it

- **Markets:** each outcome's chance (its price), volume, and what you hold. A share pays 1 sat if its outcome
  happens.
- **Trading:** get a price first, then buy or sell: "about X, at most Y". The cap includes the 1,000-sat miner fee
  and a margin, because a trade's price is fixed when its block is built, and a trade whose cap is passed waits.
  Trades go through with the next Truthcoin block, about every 10-17 minutes on eCash beta. A trade that keeps
  waiting can be cancelled from Home.
- **One coin, one waiting trade:** a trade spends a whole coin until its block. If your balance is a single coin,
  Home offers to split it into four, so several trades can wait at once.
- **Creating a market:** under Markets. You pick the question, how voters will decide it, the period when they will,
  and its depth (the liquidity you put in). Every cost is shown before it is made, and you earn its trading fees.
- **Withdrawing to eCash:** on Home. It joins a withdrawal bundle that eCash miners approve over many blocks.

## Your phone

The **Phone** tab › **Pair a phone** shows a QR code. Scan it with the phone's camera: it opens the phone page
(`https://mblowes.github.io/truthcoin-app/`, built from this repository's tagged source), which pairs with the app.
Both screens then show the same six-digit code: allow the phone only if they match.

A paired phone can see markets, positions and balance, get prices, trade within its daily limit (a buy counts at its
cap, a sell at its number of shares, a sat each; trades over the limit wait for you on the desktop, for up to an hour)
and show a receiving address. It can never withdraw, send coins, create markets or see the
recovery words. The app must be open for the phone to reach it. On iPhone, add the page to the Home Screen first,
then pair from there.

The link is described in [`docs/PROTOCOL.md`](docs/PROTOCOL.md): P-256 ECDH, HKDF-SHA256 and AES-256-GCM, each
message sealed on its own, carried by public Nostr relays (`wss://relay.damus.io`, `wss://nos.lol`,
`wss://relay.primal.net` by default; change them in the Phone tab › Relays).

## What the app can't protect

The Truthcoin node has **no login**, and its RPC lets any web page call it (it allows every origin). Its wallet's seed
is stored **unencrypted** in its data folder. The app puts the wallet's calls on a separate port, picked at random at
each start, listening only on this computer; on Linux also on a random address in `127.0.0.0/8`, so a web page can't
find it by scanning. On macOS it is `127.0.0.1`: a web page in a browser that lets pages reach this computer (some do)
could find it by trying ports, and any program running as you can find it anywhere. So: run it on a computer you
trust, close it when you aren't using it, and keep only what you're trading in it. [`VERIFY.md`](VERIFY.md) maps
everything the app does.

## Verify your download

Each release has `SHA256SUMS`, signed with this app's release key (`SHA256SUMS.sig`), and a build attestation from
GitHub for each package.

The release key (also in [`release/truthcoinapp-release.pub`](release/truthcoinapp-release.pub)):

```
<the key is added here with the first signed release>
```

```sh
echo "truthcoinapp-release <the key above>" > allowed_signers
ssh-keygen -Y verify -f allowed_signers -I truthcoinapp-release -n truthcoinapp-sums -s SHA256SUMS.sig < SHA256SUMS
sha256sum -c --ignore-missing SHA256SUMS
gh attestation verify <package> --repo mblowes/truthcoin-app
```

The app's own "Check for a newer version" (Settings) trusts only a release whose `SHA256SUMS` carries that
signature.

## Development

```sh
npm ci --ignore-scripts
cd src-tauri && cargo test -j2          # unit tests, and the phone link end to end through a local relay
```

A private chain for development (eCash regtest, the enforcer, slot 13 active) is in [`dev/`](dev/README.md):
`TC_NO_NODE=1 dev/stack.sh` starts eCash and the enforcer only, as BitWindow gives a user, for the app to run its own
node on. `cargo test realnode -- --ignored` then walks a market's whole life through the app's own code (see
`src-tauri/src/realnode.rs`). Run the app on a scratch folder with `TRUTHCOIN_APP_DIR=<folder>`; its
`settings.json` can point at the dev chain (`network`, `enforcer`, `node_binary`, `node_args`).

The phone page is in [`phone/`](phone/) (`npm ci --ignore-scripts && npm test && npm run build`).

## License

MIT. See [LICENSE](LICENSE). L2L's Truthcoin node is L2L's, under its own terms: this app downloads and runs it, and
never includes or links its code.
