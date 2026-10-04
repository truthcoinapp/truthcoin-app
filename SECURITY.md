# Security

Please report security problems privately, never in a public issue:
[Report a vulnerability](https://github.com/mblowes/truthcoin-app/security/advisories/new) (the repository's Security
tab). Only you and the maintainers see it.

A security problem is anything that could let someone else take coins from a wallet the app runs, act for a paired
phone, get past a phone's daily limit or the desktop's confirmation, run code through the app, or make the app run a
node program other than the one it pins.

The Truthcoin node itself (L2L's `truthcoin_dc`) has no RPC login and stores its seed unencrypted; the app can't fix
that, and says so (README, "What the app can't protect"). Problems in the node belong with
[L2L](https://github.com/LayerTwo-Labs/truthcoin-dc).

Please give us time to fix a problem before you tell anyone else about it.
