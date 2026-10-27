# Helius context.slot reproduction

Some finalized `getMultipleAccounts` and `getAccountInfo` answers from Helius return account data from a later slot than the `context.slot` they report.

The program reads the Clock sysvar (`SysvarC1ock11111111111111111111111111111111`) at `finalized` commitment, alternating the two methods, and compares:

- `context.slot`: the slot the answer says it was read at;
- the Clock's own `slot` field, the first 8 bytes of its data: the slot of the bank the data came from.

The two are equal in every answer from the other endpoints tried. From Helius, about 2% to 4% of answers carry a Clock slot 1 to 5 slots after `context.slot`.

## Run

```sh
RPC_URL='https://mainnet.helius-rpc.com/?api-key=…' cargo run --release -- 300 300
```

The arguments are the number of reads (300) and the time between them in milliseconds (300). It prints every answer whose Clock slot differs from `context.slot`, then a summary per method.

## Results, 2026-10-27, 300 reads each, run at the same time

| Endpoint | `getVersion` `solana-core` | Answers with Clock slot ≠ `context.slot` |
| --- | --- | --- |
| Helius mainnet | 4.3.0 (4.3.2 an hour earlier) | 6 of 300 (2.0%), all +1 |
| Triton mainnet | 4.3.2 | 0 of 300 |
| Helius devnet | 4.4.0-beta.0 | 8 of 300 (2.7%): +1 ×5; +2 ×3 |
| `api.devnet.solana.com` | 4.4.0-beta.1 | 0 of 300 |

An earlier run of the same check, over 300 reads per endpoint, gave Helius mainnet 11 of 300 (3.7%, all +1), Alchemy mainnet (`solana-core` 4.3.2) 0 of 300, and Helius devnet 11 of 300 (+1 to +5).

Helius mainnet, lines the program printed:

```text
read 37 getAccountInfo: context.slot 462215068, Clock slot 462215069 (+1)
read 63 getAccountInfo: context.slot 462215108, Clock slot 462215109 (+1)
read 111 getAccountInfo: context.slot 462215182, Clock slot 462215183 (+1)
```

Helius devnet:

```text
read 70 getMultipleAccounts: context.slot 515821700, Clock slot 515821701 (+1)
read 130 getMultipleAccounts: context.slot 515821798, Clock slot 515821799 (+1)
read 134 getMultipleAccounts: context.slot 515821804, Clock slot 515821805 (+1)
```

## Why it matters

A client that orders account data by `context.slot`, for example to compare an RPC read with the same accounts kept from a Geyser stream, treats the newer data as the value at the older slot. Every account the later slot wrote then looks wrong for one read. It is the Clock sysvar here because the Clock changes every slot; the same answers carry any other account at the later slot's value.
