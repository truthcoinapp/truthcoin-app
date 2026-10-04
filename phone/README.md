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
- **The comparison code includes the desktop's commitment nonce N** (`{"re","nonce"}`, sent after the phone's
  request): the phone shows "Waiting for your computer…" until N comes. Until then it resends the request, each time
  sealed afresh (a new event id; the same P, E, id and name), so the desktop's "same phone asks again" path sends the
  same N again (at most every 2 s, 10 times). After 10 s without a code it says "No code yet: don't allow anything on
  your computer until this phone shows one." A yes that overtakes N waits up to 3 s for it.
- **`unsure` keeps a trade open** ("Not confirmed: check Positions before trying again", Ask again with the same id);
  `err` always means not done.
- **Nullable fields** (`status.height`, `market.state`, `market.volume`, a trade's `txid`) and 0-based pages.

## Pairing safeguards (from the v0.1.0 security review)

- **Links:** an expiry `x` more than 10 minutes ahead is refused. The Name and waiting screens show the computer's key
  fingerprint (as the desktop shows it in its Phone tab), warn when it's a different computer from the one paired
  now, and say: "Only scan the code your own computer shows. Never use a pairing link someone sent you."
- **Attempts:** one pairing code's keys (P, E, nP, the request id, the name, then N) are kept in IndexedDB only while
  that pairing is under way, at most 5 minutes plus 30 s from when they were made, and never written once the run is
  cancelled. Leaving the pairing screen in any way drops them (unloading the page doesn't). After a reload the page
  asks "Carry on pairing with the computer whose key is …? [Carry on] [Stop]"; it never carries on by itself. Cancel
  while waiting says "This code is used now: show a new one on your computer" (scanning it again would be a second
  phone to the desktop), and a run cancelled just as the yes arrives ends cancelled.
- **Confirming:** after the desktop's yes, nothing is kept until the person answers "Did your computer show <code>,
  and did you allow it there?" with "Yes, I allowed it". "No" keeps nothing and sends nothing more.
- **Sessions:** pairing again stops the old session before anything is written; a stopped session writes nothing;
  waiting trades are kept per pairing (`pending:<npub>`), and pairing drops every other pairing's list; the status
  reply's name, limit or relays are saved only over the same pairing. Other tabs reload when one pairs or forgets (BroadcastChannel).
- **Forget** asks the desktop to `unpair` (a few seconds, best effort), then deletes the whole IndexedDB database. No
  answer to `unpair` reads as "Probably done… check the Phone tab on your computer"; a failed delete is reported.
- **Relays:** a repeat of an accepted event is dropped before anything else; a cheap check (kind, from the desktop,
  to us) runs before the signature check. Strangers' events count for nothing (anyone can address the phone); a relay
  passing on more than 50 events in 10 s that pass the cheap check, or more than 2 frames over 256 KiB in a minute,
  is dropped for 5 minutes (a single oversize frame is just dropped). Reconnect backoff starts over only after 30 s
  up.
- **Framing:** inside another page's frame, the page shows "Open this page directly" and stops before reading the
  fragment or storage.
- **CSP:** `default-src 'none'; script-src 'self'; style-src 'self'; connect-src wss:; img-src 'self' data:;
  manifest-src 'self'; base-uri 'none'; form-action 'none'` (no `'unsafe-inline'`, no `worker-src`). Only the dev
  server relaxes it, for Vite's injected styles. The browser checks fail on any console error, CSP included; WebKit's
  screenshots inject a stylesheet of Playwright's own, and only those refusals (one per screenshot) are set aside.

Beyond the protocol:
- **Asking:** a request with no answer is sent again once, after 15 s, and only if nothing at all came from the
  computer meanwhile (a computer answering other requests is alive; this one is just slow). A read with no answer
  that a relay did take is asked once more by itself ("Your computer hasn't answered yet (it may be reconnecting).
  Asking again…"). With no relay open, a request gives up after 25 s, and the last failed read is asked again when a
  relay comes back.
- **Busy:** the computer's busy answer (`"busy": true` beside its `err`; from an older computer, an `err` starting
  "Your computer is busy") isn't kept for the request id, so
  the phone says "Your computer is busy; asking again in a moment" and asks again under the same id after 5 s, twice
  at most.
- **Polling:** Home asks for status, balance and positions when shown and every 30 s while shown (never from another
  screen), and for recent trades too when the block number changes while a trade is on its way. The page also asks
  again about unanswered trades when it comes back to the front, polls held trades every 60 seconds, and follows at
  most the first 5 usable relays in `status`.
- **Markets:** list rows show the leading outcome's chance ("Yes 53%") from the `markets` reply's `leading`.
- **Money words:** the balance shows what the wallet holds once what's moving settles, with "Held by N waiting trades"
  and "On its way to eCash … (pays out in days)" under it only when not zero. Trade amounts include the miner fee
  (a buy costs `sats + miner_fee`, a sell brings `sats − miner_fee`). A buy that costs at least what it can pay back
  is shown in red and takes a second tap.
