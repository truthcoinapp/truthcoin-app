#!/usr/bin/env bash
# One market's whole life on the dev stack (dev/stack.sh, started fresh): create it with a new binary decision, buy,
# sell, vote, and watch it settle and pay out. Checked by hand 2026-10-04 on truthcoin_dc v0.19.0; each step says what
# it showed then.
#
#   dev/lifecycle.sh <TC_WORK>/stack.env
set -euo pipefail
# shellcheck disable=SC1090
source "${1:?usage: dev/lifecycle.sh <the stack.env that dev/stack.sh printed>}"
eval "$(sed -n '/^tc() {/,/^}/p' "$(dirname "$0")/stack.sh")"   # the current tc helper (stdin, any size)
ok() { echo "OK: $*"; }
die() { echo "FAILED: $*" >&2; exit 1; }
field() { python3 -c "import json,sys; print(json.load(sys.stdin)$1)"; }

# 1. Create a market and claim its decision in one transaction: period 3, so it trades in periods 1-3 and is voted on
#    in period 4 (10 blocks per period on this stack). Paid: the liquidity (beta * ln 2 for 2 outcomes), the listing fee
#    (2,500 sats at tier 0) and the transaction fee.
r=$(tc market_create '[{"title":"Will the dev chain reach height 40?","description":"A test market on a private dev chain.",
  "dimensions":[{"type":"new","period_index":3,"decision_type":"binary","header":"Dev chain height 40 by period 3",
  "description":"Resolves yes if block 40 exists.","option_0_label":"No","option_1_label":"Yes","tags":["test"]}],
  "beta":1000000,"trading_fee":0.01,"tx_fee_sats":1000,"max_listing_fee_sats":100000}]')
case "$r" in ERROR*) die "market_create: $r" ;; esac
M=$(echo "$r" | field '["market_id"]'); D=$(echo "$r" | field '["claimed_decisions"][0]["id"]')
bmm 1
ok "market $M, decision $D: $(tc market_list | field '[0]["state"]')"

# 2. Quote, then buy 100,000 share units of "Yes" (a unit pays 1 sat if it wins). max_cost must leave room for the
#    trade's miner fee (1,000 sats): the RPC checks cost <= max_cost, but the block builder checks cost + miner fee and
#    otherwise skips the trade every block, leaving the whole spent coin tied up ("Skipping tx ... due to slippage").
q=$(tc market_buy "[{\"market_id\":\"$M\",\"outcome_index\":1,\"shares_amount\":100000,\"dry_run\":true}]")
cost=$(echo "$q" | field '["cost_sats"]')
tc market_buy "[{\"market_id\":\"$M\",\"outcome_index\":1,\"shares_amount\":100000,\"max_cost\":$((cost + 1000 + 1000))}]" >/dev/null
# While it waits, the wallet shows almost nothing: a trade spends a whole coin and gets its change at block connection.
ok "bought, pending; balance meanwhile $(tc bitcoin_balance | field '["total_sats"]') sats"
bmm 1

# 3. The shares sit at the address the wallet picked (not get_wallet_addresses()[0]): look at every address.
holder=""
for a in $(tc get_wallet_addresses | python3 -c 'import json,sys; print(" ".join(json.load(sys.stdin)))'); do
    n=$(tc market_positions "[\"$a\", \"$M\"]" | python3 -c 'import json,sys; print(sum(p["shares"] for p in json.load(sys.stdin)["positions"]))')
    [ "$n" -gt 0 ] && { holder=$a; ok "$n shares at $a"; }
done
[ -n "$holder" ] || die "no position found"

# 4. Sell half, with min_proceeds below the quote's net.
q=$(tc market_sell "[{\"market_id\":\"$M\",\"outcome_index\":1,\"shares_amount\":50000,\"seller_address\":\"$holder\",\"dry_run\":true}]")
net=$(echo "$q" | field '["net_proceeds_sats"]')
tc market_sell "[{\"market_id\":\"$M\",\"outcome_index\":1,\"shares_amount\":50000,\"seller_address\":\"$holder\",\"min_proceeds\":$((net - 2000))}]" >/dev/null
bmm 1
ok "sold 50,000 for about $net sats"

# 5. Mine to the voting period (height 31 here: the decision turns "Voting"), vote Yes with the wallet's voter address
#    (index 0; on this regtest stack it holds all the votecoin, 1.0), mine past the period's end.
n=0; until tc vote_period '[null]' | grep -q "\"decision_id_hex\": \"$D\""; do
    bmm 1; n=$((n + 1)); [ $n -ge 60 ] && die "decision $D never came up for a vote"
done
tc vote_submit "[[{\"decision_id\":\"$D\",\"vote_value\":1.0}], 1000]" >/dev/null
ok "voted Yes at height $(tc getblockcount)"
n=0; until [ "$(tc market_get "[\"$M\"]" | field '["state"]')" = settled ]; do
    bmm 1; n=$((n + 1)); [ $n -ge 30 ] && die "not settled after 30 blocks"
done
ok "settled at height $(tc getblockcount): $(tc market_get "[\"$M\"]" | field '["resolution"]["summary"]')"
ok "shares paid out automatically: positions $(tc market_positions "[\"$holder\", \"$M\"]" | field '["positions"]'), balance $(tc bitcoin_balance | field '["total_sats"]') sats"
