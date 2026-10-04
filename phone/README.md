# The phone page

A remote for the Truthcoin App on your computer: a web page (installable to the Home Screen) that pairs with the
desktop app by QR code, then shows markets, positions and balance, and trades, through sealed messages carried by
public Nostr relays. The protocol is `../docs/PROTOCOL.md`; the shared test vectors are
`../src-tauri/testdata/protocol-v1.json`.

Svelte 4, Vite 5, TypeScript. Crypto is WebCrypto (P-256 ECDH, HKDF-SHA256, AES-256-GCM, SHA-256), plus
`@noble/curves`/`@noble/hashes` for the Nostr side (secp256k1 BIP340) only. `jsqr` reads QR codes from the camera
(Safari has no BarcodeDetector); it loads only when Scan is tapped. Nothing else at run time.

## Commands

```sh
npm ci --ignore-scripts
npm test                 # unit tests, the vectors, and runs through ../dev/test-relay.mjs
npm run check            # svelte-check
npm run build            # dist/: the page for GitHub Pages (relays must be wss://)
npm run build:local      # dist-local/: also takes ws://127.0.0.1 and ws://localhost relays (for the test relay)
npm run preview          # serves dist/ on http://127.0.0.1:4174/
npm run e2e              # both builds, then the browser checks in Chromium and WebKit (headless)
```

`npm run e2e` needs Playwright 1.52's browsers: set `PLAYWRIGHT_BROWSERS_PATH` to where they are (WebKit on Ubuntu
24.04 may also need `LD_LIBRARY_PATH` for a few libraries). An engine whose browser is missing is skipped. Screenshots
go to `e2e/shots/<engine>/`.

## Where things are

| File | What |
|---|---|
| `src/lib/crypto.ts` | Sealing, pairing envelope, comparison code, padding (WebCrypto) |
| `src/lib/nostr.ts` | NIP-01 events: id, BIP340 signing and checking |
| `src/lib/relaypool.ts` | The Nostr client: a socket per relay, checks and deduplicates events, backoff |
| `src/lib/link.ts` | Requests and replies: sealing, resending under the same id, held answers, repeats dropped |
| `src/lib/pairing.ts` | Reading the QR code's value and pairing |
| `src/lib/validate.ts` | Every answer from the desktop checked before it is shown |
| `src/lib/flows.ts` | Trades from the tap to the final answer, kept in IndexedDB until then |
| `src/lib/session.ts` | The page's state once paired, and the "last request" line |
| `src/lib/store.ts` | IndexedDB: the pairing (device key non-extractable) and trades still waiting |
| `src/lib/testing/` | A test desktop and a fake WebSocket, for tests only |
| `src/components/` | The screens |
| `e2e/browser.e2e.ts` | The browser checks |

## Choices PROTOCOL.md now records

- **Asking again keeps the first `ts`**: a repeat is the same request (id, `ts`, args) sealed afresh.
- **Pairing also probes with `status`** every 5 seconds, so a lost pairing answer doesn't leave the phone waiting.
- **`unsure` keeps a trade open** ("Not confirmed: check Positions before trying again", Ask again with the same id);
  `err` always means not done.
- **Nullable fields** (`status.height`, `market.state`, `market.volume`, a trade's `txid`) and 0-based pages.

Beyond it: the phone also asks again about unanswered trades (and asks for `status`) when the page comes back to the
front, polls held trades every 60 seconds, and follows at most the first 5 usable relays in `status`.
