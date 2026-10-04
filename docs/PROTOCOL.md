# The phone link, v1

How the phone page talks to the desktop app: sealed messages carried by public Nostr relays. Both ends dial out to the
relays, so neither needs an open port, and carrier-grade NAT (mobile networks, Starlink) doesn't matter. Nobody hosts
anything for the link. Relays see two random keys exchanging same-sized blobs, and when; never what's in them.

Every transport is treated as untrusted. Relays may drop, repeat, reorder or delay any event, or make up their own.
Each message is therefore sealed on its own (no session), and every request carries an id that makes repeating it
safe. The test vectors are in `src-tauri/testdata/protocol-v1.json`; the Rust and TypeScript tests both check them.

## Keys

| Key | Where | What for |
|---|---|---|
| Desktop static `D` (P-256) | the app's data folder, owner-only | identity; sealing |
| Desktop Nostr key `nD` (secp256k1) | the same folder; random, never the user's own Nostr identity | signing its events; the address phones send to |
| Phone device key `P` (P-256) | IndexedDB, **non-extractable** WebCrypto key | identity; sealing |
| Phone Nostr key `nP` (secp256k1) | IndexedDB (WebCrypto has no secp256k1) | signing its events only. Not a trust anchor: the sealing proves who sent a message |
| Pairing code `C` | the pairing QR; 16 random bytes, 5 minutes, used once | pairing |

P-256 public keys travel as base64url (no padding) of the 65-byte uncompressed SEC1 point; anything else is refused.
Nostr keys are x-only, 64 lowercase hex characters (NIP-01).

## Pairing

The desktop shows a QR code for this URL:
```
https://<phone page>/#pair=<b64u of JSON {"v":1, "r":["wss://…", …], "n":"<nD, hex>", "d":"<D_pub, b64u>",
                                          "c":"<C, b64u>", "x":<expiry, unix seconds>}>
```
The fragment never reaches a web server. The page reads it, then removes it from the address bar before showing
anything. `r` lists at most 5 relays, `wss://` only.

1. The phone makes `P`, `nP` and an ephemeral P-256 key `E`, and sends a **pairing request**: an event (below) from
   `nP` to `nD` whose content is a pairing envelope:
   ```
   k   = HKDF-SHA256(ikm = ECDH(E, D), salt = C, info = "tcr-pair-v1", 32 bytes)
   aad = "tcr-pair-v1" || D_pub || E_pub                      (raw 65-byte keys)
   ct  = AES-256-GCM(k, 12-byte random nonce, plaintext, aad)
   content = JSON {"v":1, "k":"pair", "e":b64u(E_pub), "n":b64u(nonce), "ct":b64u(ct)}
   plaintext = {"t":"pair", "p":b64u(P_pub), "np":"<nP hex>", "name":"<device name>", "id":"<request id>"}, padded
   ```
2. The desktop opens it with the live `C` and checks that `np` is the event's own pubkey. The first request that
   opens claims `C`. The desktop picks a random 16-byte **commitment nonce** `N` and sends it to the claimant, sealed
   (D to P): `{"re":"<id>", "nonce":b64u(N)}`. Only then does it ask "Allow this phone? <name>", showing the
   **comparison code**: `SHA-256("tcr-pair-sas-v1" || D_pub || P_pub || E_pub || C || N)`, the first 4 bytes
   big-endian, mod 1,000,000, as six digits in two groups ("042 917"). The phone shows the same code once it has `N`.
   Because `N` is chosen after the phone's `P` and `E` are fixed, nobody can search for keys that give a code they
   want: someone who saw the QR code can send a request too, but can't make the codes match.
3. The same phone (same `P` and `nP`) asking again gets the same `N` again (its answer may have been lost). A request
   from another key while a claim is shown means the code is out: the pairing is **contested** and can only be
   refused. Allowing or refusing uses `C` up.
4. After the desktop's yes, the phone asks its user to confirm that the computer showed this code and that they
   allowed it there; a rogue "desktop" from a crafted link can answer yes at once, but can't make the user's own
   computer ask. Pairing links whose expiry `x` is more than 10 minutes ahead are refused, and an attempt never
   restarts by itself after a reload.
5. The desktop answers with a **sealed message** (never clear text) under the request's id:
   `{"re":"<id>", "ok":{"paired":true, "name":"<name as shown>", "limit_sats":<daily limit>}}` or
   `{"re":"<id>", "err":"not allowed"}`.
6. Device names are cut to 40 characters, with control and format characters (bidi overrides, zero-width) removed.
7. **A lost answer:** if every relay drops the desktop's pairing answer, resending the request doesn't help (a repeat
   of the same event is a duplicate, and `C` is used up). So while it waits, the phone also sends a sealed `status`
   request from `P` every 5 seconds. The desktop drops those until it has allowed the phone, then answers one: that
   answer proves the pairing.
8. **Fingerprint:** both screens can show the desktop key's fingerprint, the first 6 bytes of SHA-256(D_pub) in hex,
   in three groups of four (`keys.D.fingerprint` in the test vectors).

## Events (NIP-01)

- Kind **21913**: ephemeral (relays pass it on and don't keep it), claimed by no NIP as of October 2026.
- Tags: `["p", <recipient's Nostr pubkey>]` and `["expiration", <created_at + 300>]` (NIP-40, for relays that keep
  ephemeral events anyway).
- `content`: the envelope as compact JSON.
- Every event goes to every relay in the list. Receivers drop events whose id or signature is wrong, then duplicates
  by event id, then repeats by request id.
- The desktop subscribes with `{"kinds":[21913], "#p":[nD], "since":<now - 120>}` on every relay while it runs; the
  phone subscribes to its `nP` while the page is open.
- The desktop drops, before any other work, an event whose pubkey isn't a paired phone's `nP`, unless a pairing code
  is live and the content is a pairing envelope.

## Sealed messages

From a sender with static key `S` to a receiver `R` (phone to desktop: `S = P`, `R = D`; desktop to phone the
other way round). Sender-authenticated, like HPKE's auth mode:
```
E      = a fresh ephemeral P-256 key
ikm    = ECDH(E, R) || ECDH(S, R)                        (each 32 bytes: the shared point's X)
aad    = "tcr-msg-v1" || S_pub || R_pub || E_pub          (raw 65-byte keys)
k      = HKDF-SHA256(ikm, salt = SHA-256(aad), info = "tcr-msg-v1", 32 bytes)
ct     = AES-256-GCM(k, 12-byte random nonce, plaintext, aad)
content = JSON {"v":1, "k":"msg", "e":b64u(E_pub), "n":b64u(nonce), "ct":b64u(ct)}
```
The receiver knows `S` from the event's Nostr key (pairing tied `nP` to `P`; the phone knows `nD` and `D` from the
QR). A message that doesn't open is dropped without an answer.

**Padding:** a plaintext is UTF-8 JSON padded with spaces (which JSON ignores) to 1,024, 4,096 or 16,384 bytes,
whichever fits first. Nothing bigger is sent: long lists come in pages.

## Requests and replies

- Request: `{"id":"<16 random bytes, hex>", "ts":<unix seconds>, "m":"<method>", "a":{…}}`
- Reply: `{"re":"<id>", "ok":<result>}` or `{"re":"<id>", "err":"<message for people>"}`
- Held for the desktop: `{"re":"<id>", "held":{"text":"<what the desktop is asked>"}}`, later followed by a final
  reply under the same `re`.
- Unsure: `{"re":"<id>", "unsure":"<message>"}`: a trade that may have reached the node without an answer coming back
  (or the desktop stopped between the two). It may have gone through: the phone keeps it open, says "Not confirmed:
  check Positions before trying again", and may ask again with the same id. `err` always means it was not done.

**Repeats are safe.** The desktop keeps every request id with its answer for 24 hours, on disk. A request it has seen
gets the stored answer again and **never runs twice**. That makes it safe to send a trade again when no answer came,
and makes a replayed event harmless. The id is stored before a trade goes to the node, so after a crash between the
two the trade counts as "may have gone out".

**Freshness:** the desktop refuses a new id whose `ts` is more than 5 minutes from its clock, without running it. A
repeat is the same request (id, `ts` and args) sealed afresh, so a trade can only start within 5 minutes of being
made; later repeats can only fetch its answer. (A repeat of a known id is answered whatever its `ts`.)

**No answer:** the phone says "Not confirmed: check Positions before trying again", never "Not sent", and offers to
ask again **with the same id**, which is always safe.

**When the phone is closed:** ephemeral events only reach open subscriptions. So the phone pulls: on opening it asks
for `status`, and asks again (same ids) about trades still held or unanswered.

The desktop runs at most 60 new requests a minute per phone, and answers at most 30 repeats; over that, a new
request gets (a few times a minute at most) `{"re":"<id>", "err":"Your computer is busy: ask again in a few seconds",
"busy":true}` and isn't run: asking again under the same id is safe.

## Methods

Amounts are sats. Prices are the market's probabilities, 0 to 1. A share pays 1 sat if its outcome wins. Pages count
from 0. Fields the node may not give can be `null` or missing: `status.height` (node not running), `market.state`,
`market.volume`, a trade's `txid`. `status.relays` holds at most 5, all `wss://`.

| Method | Args | Result |
|---|---|---|
| `status` | — | `{"app":"0.1.0", "node":"running"\|"starting"\|"stopped"\|"failed", "height":n, "synced":bool, "network":"betanet", "name":"<this phone>", "limit_sats":n, "left_sats":n, "relays":[…]}` |
| `markets` | `{"page":n}` | `{"markets":[{"id","title","state","outcomes":n,"volume":n,"created":height,"leading":{"label","price"}\|null}], "page":n, "pages":n}`: trading markets first, newest first; `leading` is the outcome with the highest chance (null when not trading) |
| `market` | `{"id"}` | `{"id","title","description","state","fee_rate","volume","outcomes":[{"i","label","price","volume"}], "resolution":{"summary","winners":[i]}\|null, "holdings":[{"outcome","shares","value"}], "decisions":[{"question","rules","period"}], "current_period", "blocks_per_period"\|null}`: `decisions` say how each question is decided and in which voting period; `blocks_per_period` is set only on test networks |
| `positions` | — | `{"positions":[{"market_id","title","state","outcome","label","shares","price","value","paid"}], "total_value":n, "settled":[{"market_id","title","winners":[label],"paid","outcomes":[{"label","shares","per_share"}]}]}` (`paid` may be null; `settled`: markets this wallet traded in through the app that have settled, and what they paid) |
| `balance` | — | `{"total","available","in_pending_trades","pending_trades"}` |
| `quote` | `{"id","outcome","shares","side":"buy"\|"sell"}` | `{"side","sats","fee","miner_fee","price_now","price_after","limit"}`: `sats` is the cost (buy, trading fee included) or the net proceeds (sell); `limit` is the cap the desktop suggests |
| `trade` | `{"id","outcome","shares","side","limit"}` | `{"status":"pending","txid"}`, or `held` first when over the phone's limit. `limit` is the phone's cap: at most this (buy, miner fee included), at least this (sell) |
| `trades` | — | `{"trades":[{"id","time","title","label","side","shares","sats","limit","status","txid"}]}`: this phone's last 20 |
| `receive` | — | `{"address","deposit_address"}` |
| `unpair` | — | `{"unpaired":true}`, then the desktop forgets the phone (its "Forget this computer") |

Never reachable from a phone: sending coins, withdrawing, creating markets, voting, the wallet's words, raw node calls.

**Limits:** each phone has a daily limit (a rolling 24 hours). Every trade counts from the moment it may have gone to
the node: a buy at its cap (fees included), a sell at its number of shares (a sat each, the most they can pay, so no price
move can lower it). A trade
that would pass the limit is held, and the desktop shows it ("Phone <name> wants to buy 100,000 Yes in <market> for at
most 54,743 sats"); unanswered for an hour, it ends with an `err`. A phone may have at most 3 trades held and 3
waiting for their block. A buy's `limit` must be at least the quote's `sats` plus the miner fee and half the margin
(`limit - sats - miner_fee` in the quote is the whole margin), and at most twice the quote's `limit`. A sell is
refused if its proceeds before the trading fee are under 80% of the shares' value, or if its `limit` is more than one
margin below the quote's `limit`. Every phone trade is in the desktop's trade list.

## Relays

- Default: `wss://relay.damus.io`, `wss://nos.lol`, `wss://relay.primal.net`, editable on the desktop. The `status`
  reply carries the current list, so a paired phone follows changes.
- Back off on `["OK", <id>, false, "rate-limited: …"]` and on dropped connections. One working relay is enough.
- What a relay sees: two random keys, when they talk, and padded sizes.
