#!/usr/bin/env bash
# A private Truthcoin test chain on this computer, for building the app: eCash betanet bitcoind (regtest), L2L's
# bip300301_enforcer with its wallet, slot 13 (Truthcoin) proposed and activated, and truthcoin_dc on regtest with
# short decision periods. Everything listens on 127.0.0.1 only. It stays up until you stop it (Ctrl-C or kill <pid>),
# then stops every daemon and deletes its folder (TC_KEEP=1 keeps it).
#
#   dev/stack.sh [--deposit COINS]        default 10 coins to the Truthcoin wallet after start
#
# It writes $TC_WORK/stack.env: source it from another shell for the endpoints and the helpers
#   tc METHOD [JSON params]   a JSON-RPC call to truthcoin_dc (prints the result, or the error)
#   bmm [N]                   N Truthcoin blocks (default 1): truthcoin's `mine`, then one L1 block a second later,
#                             as truthcoin-dc's own integration tests do (integration_tests/setup.rs, bmm_single)
#   l1 N                      N L1 blocks only (through the enforcer, which ACKs proposals and bundles)
#
# Binaries (overrides): TC_BIN (truthcoin_dc; default dev/bin/truthcoin-0.19.0-x86_64-unknown-linux-gnu, from
# github.com/LayerTwo-Labs/truthcoin-dc/releases), TC_L1_BIN (dir with bitcoind and bitcoin-cli, eCash betanet
# v31.1.0), TC_ENFORCER, TC_GRPCURL. Ports: TC_PORT_BASE (default: the first free block of 10 from 23400).
#
# TC_NO_NODE=1 starts only eCash and the enforcer (the stack BitWindow gives a user), for the app to run its own
# Truthcoin node on: stack.env then has `tcapp METHOD [params]` (a call to the app's node, found through
# $TRUTHCOIN_APP_DIR/node.json) and `appbmm [N]` (N Truthcoin blocks mined through it).
set -euo pipefail
HERE=$(cd "$(dirname "$0")" && pwd)
TC_BIN=${TC_BIN:-$HERE/bin/truthcoin-0.19.0-x86_64-unknown-linux-gnu}
TC_L1_BIN=${TC_L1_BIN:-$HERE/bin/ecash}
TC_ENFORCER=${TC_ENFORCER:-$HERE/bin/bip300301_enforcer}
TC_GRPCURL=${TC_GRPCURL:-$HOME/.local/bin/grpcurl}
DEPOSIT=10
while [ $# -gt 0 ]; do
    case "$1" in
        --deposit) DEPOSIT=$2; shift 2 ;;
        *) echo "usage: $0 [--deposit COINS]"; exit 2 ;;
    esac
done
for b in "$TC_BIN" "$TC_L1_BIN/bitcoind" "$TC_L1_BIN/bitcoin-cli" "$TC_ENFORCER" "$TC_GRPCURL"; do
    [ -x "$b" ] || { echo "missing: $b"; exit 2; }
done

free() { ! (exec 3<>/dev/tcp/127.0.0.1/$1) 2>/dev/null; }
if [ -z "${TC_PORT_BASE:-}" ]; then
    TC_PORT_BASE=23400
    while :; do
        ok=1; for i in $(seq 0 9); do free $((TC_PORT_BASE + i)) || { ok=0; break; }; done
        [ $ok = 1 ] && break; TC_PORT_BASE=$((TC_PORT_BASE + 10))
    done
fi
L1_RPC=$TC_PORT_BASE; L1_P2P=$((TC_PORT_BASE + 1)); L1_ZMQ=$((TC_PORT_BASE + 2))
ENF_GRPC=$((TC_PORT_BASE + 3)); ENF_RPC=$((TC_PORT_BASE + 4))
TC_RPC=$((TC_PORT_BASE + 5)); TC_P2P=$((TC_PORT_BASE + 6)); TC_ZMQ=$((TC_PORT_BASE + 7))

TC_WORK=${TC_WORK:-$(mktemp -d "${TMPDIR:-/tmp}/tc-stack.XXXXXX")}
L1_DIR=$TC_WORK/l1; ENF_DIR=$TC_WORK/enforcer; TC_DIR=$TC_WORK/truthcoin
mkdir -p "$L1_DIR" "$ENF_DIR" "$TC_DIR"
L1_CLI="$TC_L1_BIN/bitcoin-cli -regtest -datadir=$L1_DIR -rpcuser=t -rpcpassword=t -rpcport=$L1_RPC"
EA=127.0.0.1:$ENF_GRPC

say() { echo "STACK: $*"; }
fail() { echo "STACK FAILED: $*" >&2; exit 1; }

stop_pid() {   # stop_pid <pidfile> <name>: TERM, then wait up to 30 s
    local pid; pid=$(cat "$1" 2>/dev/null) || return 0
    kill -TERM "$pid" 2>/dev/null || return 0
    local n=0; while kill -0 "$pid" 2>/dev/null; do n=$((n + 1)); [ $n -ge 30 ] && { say "$2 would not stop (pid $pid)"; return 0; }; sleep 1; done
}
down() {
    local rc=$?
    trap - EXIT
    stop_pid "$TC_WORK/tc.pid" truthcoin_dc
    stop_pid "$TC_WORK/enf.pid" enforcer
    stop_pid "$L1_DIR/regtest/bitcoind.pid" bitcoind
    if [ "${TC_KEEP:-0}" = 1 ] || [ $rc -ne 0 ] && [ $rc -lt 128 ]; then say "kept $TC_WORK"; else rm -rf "$TC_WORK"; fi
    exit $rc
}
trap down EXIT
trap 'exit 143' TERM; trap 'exit 130' INT; trap 'exit 129' HUP

enf() { timeout 30 "$TC_GRPCURL" -plaintext -d "${2:-{\}}" "$EA" "cusf.mainchain.v1.$1"; }

# --- L1: eCash betanet bitcoind, regtest ---
"$TC_L1_BIN/bitcoind" -regtest -daemon -datadir="$L1_DIR" -server -rpcuser=t -rpcpassword=t \
    -rpcbind=127.0.0.1 -rpcallowip=127.0.0.1 -rpcport=$L1_RPC -port=$L1_P2P -listen=0 \
    -zmqpubsequence=tcp://127.0.0.1:$L1_ZMQ -fallbackfee=0.0001 -rest -txindex >/dev/null || fail "bitcoind"
n=0; until $L1_CLI getblockcount >/dev/null 2>&1; do n=$((n + 1)); [ $n -ge 60 ] && fail "L1 RPC never came up"; sleep 1; done

# --- the enforcer, with its wallet (it funds deposits and BMM bids) ---
setsid nohup "$TC_ENFORCER" --data-dir="$ENF_DIR" \
    --node-rpc-addr=127.0.0.1:$L1_RPC --node-rpc-user=t --node-rpc-pass=t \
    --node-zmq-addr-sequence=tcp://127.0.0.1:$L1_ZMQ --serve-grpc-addr=$EA --serve-rpc-addr=127.0.0.1:$ENF_RPC \
    --op-drivechain=nop8 --enable-mempool --enable-wallet --wallet-auto-create --wallet-sync-source=disabled \
    </dev/null >> "$ENF_DIR/enf.log" 2>&1 &
echo $! > "$TC_WORK/enf.pid"
n=0; until enf ValidatorService/GetChainTip >/dev/null 2>&1; do
    n=$((n + 1)); [ $n -ge 90 ] && { tail -20 "$ENF_DIR/enf.log"; fail "enforcer gRPC never came up"; }; sleep 1
done
MINEADDR=$(enf WalletService/CreateNewAddress | sed -n 's/.*"address": *"\([^"]*\)".*/\1/p')
[ -n "$MINEADDR" ] || fail "the enforcer wallet gave no address"
l1() {
    enf MiningService/GenerateToAddress "{\"blocks\":$1,\"address\":\"$MINEADDR\"}" >/dev/null || fail "GenerateToAddress $1"
}
enf BlockProducerService/SetAckAllProposals '{"policy":"ACK_ALL_PROPOSALS_POLICY_NEW_SLOTS"}' >/dev/null || fail "SetAckAllProposals"
l1 120   # mature coinbases for deposits and bids

# --- slot 13: propose Truthcoin and mine until it is active ---
h1=$(printf 'a%.0s' {1..64}); h2=$(printf 'b%.0s' {1..40})
enf BlockProducerService/SubmitSidechainProposal \
    "{\"sidechain_id\":13,\"declaration\":{\"v0\":{\"title\":\"Truthcoin\",\"description\":\"dev stack\",\"hash_id_1\":{\"hex\":\"$h1\"},\"hash_id_2\":{\"hex\":\"$h2\"}}}}" \
    >/dev/null || fail "SubmitSidechainProposal"
n=0; until enf ValidatorService/GetSidechains | grep -q '"sidechainNumber": 13'; do
    n=$((n + 1)); [ $n -ge 30 ] && fail "slot 13 not active"; l1 1
done
say "slot 13 active at L1 height $($L1_CLI getblockcount)"

tc() {   # tc METHOD [JSON params array]
    local out
    out=$(curl -s --max-time 120 -H 'content-type: application/json' \
        -d "{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"$1\",\"params\":${2:-[]}}" "http://127.0.0.1:$TC_RPC")
    printf '%s' "$out" | python3 -c 'import json,sys; r=json.load(sys.stdin); print(json.dumps(r["result"]) if "result" in r else "ERROR "+json.dumps(r.get("error")))'
}
bmm() {
    local i
    for i in $(seq "${1:-1}"); do
        tc mine '[null]' > "$TC_WORK/mine.out" &
        local m=$!
        sleep 1; l1 1; wait $m || true
        grep -q '^ERROR' "$TC_WORK/mine.out" && { cat "$TC_WORK/mine.out"; return 1; }
    done
    return 0
}
tcapp() {   # tcapp METHOD [JSON params array]: the app's node, wallet port
    local port
    # host:port of the wallet's calls (a random 127.x.y.z on Linux)
    port=$(python3 -c 'import json,os; j=json.load(open(os.environ["TRUTHCOIN_APP_DIR"]+"/node.json")); print("%s:%d" % (j.get("private_host") or "127.0.0.1", j["private_port"]))') || return 1
    curl -s --max-time 120 -H 'content-type: application/json' \
        -d "{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"$1\",\"params\":${2:-[]}}" "http://$port" |
        python3 -c 'import json,sys; r=json.load(sys.stdin); print(json.dumps(r["result"]) if "result" in r else "ERROR "+json.dumps(r.get("error")))'
}
appbmm() {
    local i
    for i in $(seq "${1:-1}"); do
        tcapp mine '[null]' > "$TC_WORK/appmine.out" &
        local m=$!
        sleep 1; l1 1; wait $m || true
    done
}
if [ "${TC_NO_NODE:-0}" != 1 ]; then
# --- truthcoin_dc, regtest, decision periods of 10 blocks ---
setsid nohup "$TC_BIN" --headless --network regtest --datadir "$TC_DIR" \
    --mainchain-grpc-host 127.0.0.1 --mainchain-grpc-port $ENF_GRPC \
    --rpc-host 127.0.0.1 --rpc-port $TC_RPC --net-addr 127.0.0.1:$TC_P2P --zmq-addr 127.0.0.1:$TC_ZMQ \
    --decision-config-testing 10 --log-level info \
    </dev/null >> "$TC_WORK/truthcoin.out" 2>&1 &
echo $! > "$TC_WORK/tc.pid"

n=0; until tc getblockcount 2>/dev/null | grep -qx '[0-9]*'; do
    kill -0 "$(cat "$TC_WORK/tc.pid")" 2>/dev/null || { tail -20 "$TC_WORK/truthcoin.out"; fail "truthcoin_dc stopped"; }
    n=$((n + 1)); [ $n -ge 60 ] && { tail -20 "$TC_WORK/truthcoin.out"; fail "truthcoin_dc RPC never came up"; }; sleep 1
done

# A wallet seed of its own (a test seed, never used anywhere else).
tc set_seed_from_mnemonic "[$(tc generate_mnemonic)]" >/dev/null
ADDR=$(tc get_new_address | tr -d '"')

# Deposit (M5) from the enforcer wallet, then BMM until it shows. Not truthcoin's `create_deposit`: in v0.19.0 that
# call makes a transfer inside the Truthcoin wallet (rpc_server.rs, create_deposit -> wallet.create_transfer).
if [ "$DEPOSIT" != 0 ]; then
    sats=$(python3 -c "print(int(round($DEPOSIT*1e8)))")
    enf WalletService/CreateDepositTransaction \
        "{\"sidechain_id\":13,\"address\":\"$ADDR\",\"value_sats\":$sats,\"fee_sats\":1000000}" >/dev/null \
        || fail "CreateDepositTransaction"
    l1 1
    bmm 2 || fail "BMM"
    say "deposited $DEPOSIT coins to $ADDR: balance $(tc bitcoin_balance)"
fi

fi

cat > "$TC_WORK/stack.env" <<EOF
# truthcoin dev stack, pid $$ ($(date -u +%FT%TZ)). source this file.
TC_WORK=$TC_WORK
TC_RPC=$TC_RPC
TC_URL=http://127.0.0.1:$TC_RPC
TC_P2P=$TC_P2P
ENF_GRPC=$EA
EA=$EA
L1_CLI="$L1_CLI"
TC_GRPCURL=$TC_GRPCURL
MINEADDR=$MINEADDR
$(declare -f enf l1 tc bmm tcapp appbmm fail)
EOF
if [ "${TC_NO_NODE:-0}" = 1 ]; then say "up without a Truthcoin node: L1 RPC $L1_RPC,"
else say "up: truthcoin_dc RPC http://127.0.0.1:$TC_RPC (no login: anything on this computer can call it), L1 RPC $L1_RPC,"; fi
say "enforcer gRPC $EA. Endpoints and helpers: source $TC_WORK/stack.env. Stop: kill $$"
# Wait in the background so a TERM or Ctrl-C runs the teardown at once (bash holds traps during a foreground sleep).
while :; do sleep 3600 & wait $! || true; done
