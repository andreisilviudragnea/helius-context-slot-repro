# Helius RPC answers that Agave cannot give

Two kinds of finalized answers from Helius mainnet break rules that Agave's RPC code guarantees. Triton and Alchemy, on the same Agave versions, never break them. A third answer refuses a valid `getProgramAccounts` with the JSON-RPC code for an invalid request.

| Issue                                                                                    | Methods                                                       | Helius answers affected                              | Triton  | `apiVersion` of the Helius answers |
| ---------------------------------------------------------------------------------------- | ------------------------------------------------------------- | ---------------------------------------------------- | ------- | ---------------------------------- |
| [1. Account data newer than `context.slot`](#1-account-data-newer-than-contextslot)      | `getMultipleAccounts`; `getAccountInfo`; `getProgramAccounts` | 2.0% to 4.7%                                         | 0%      | 4.3.0-alpha.2                      |
| [2. A closed account in `getProgramAccounts`](#2-a-closed-account-in-getprogramaccounts) | `getProgramAccounts`                                          | 9 of 55 address-only calls; 3 of 22 whole-data calls | 0 of 43 | 3.1.9                              |
| [3. A rate limit sent as an invalid request](#3-a-rate-limit-sent-as-an-invalid-request) | `getProgramAccounts`                                          | 4 calls on 2026-10-29, 2 on 10-30; 0 of 130 re-run   | 0       | not recorded                       |

This repo holds a program that reproduces them, the results, and the requests to trace.

## Run

```sh
RPC_URL='https://mainnet.helius-rpc.com/?api-key=…' cargo run --release -- clock 300 300
RPC_URL='https://mainnet.helius-rpc.com/?api-key=…' cargo run --release -- gpa 20 15000
RPC_URL='https://mainnet.helius-rpc.com/?api-key=…' cargo run --release -- versions 20 1000
RPC_URL='https://mainnet.helius-rpc.com/?api-key=…' cargo run --release -- deprioritized 10 1000
```

- `clock`: reads the Clock sysvar 300 times, 300 ms apart, alternating `getMultipleAccounts` and `getAccountInfo`. It prints every answer whose Clock slot differs from `context.slot`, then a count per method and `apiVersion`.
- `gpa`: reads the address of every obligation of the lending program `KLend2g3cP87fffoy8q1mQqGKjrxjC8boSyAYavgmjD` with `getProgramAccounts` 20 times, 15 s apart. It prints every answer, saying whether it holds the closed obligation, then reads that account with `getAccountInfo` and `getMultipleAccounts`.

- `clock`, `gpa` and `deprioritized` stop at the first wrong answer, so checking whether an issue still happens sends only the calls it needs.
- `versions`: calls `getVersion` and each method whose answer has a context 20 times, 1 s apart, and counts the `apiVersion` each reports.
- `deprioritized`: sends the `getProgramAccounts` of every lending market, reserve and obligation together, whole and `base64+zstd`, as a client loads them at start, for up to `reads` rounds. It stops at the first answer with error -32600 and prints its HTTP status and every header, then asks for the lending markets alone every `interval-ms` until they are answered. `deprioritized-markets` sends `reads` loads of the lending markets alone at once, as one burst, 1.1 MB each.

Every line names the answer's `apiVersion` and its `zid` and `cf-ray` headers. The program uses the RPC types of the Solana client crates (`solana-rpc-client-api` 4.3.0).

## Three builds behind one endpoint

Agave fills `getVersion`'s `solana-core` and every answer's `context.apiVersion` from the same `solana_version::Version::default()` of the running binary ([rpc.rs#L145](https://github.com/anza-xyz/agave/blob/v4.3.2/rpc/src/rpc.rs#L145) through `RpcResponseContext::new`; `get_version`). On one Agave node the two always match.

The `versions` mode called each method 20 times, 1 s apart, on 2026-10-29 at 00:20Z; the `getProgramAccounts` of all obligations comes from the `gpa` runs. "none" means the answer has a context with no `apiVersion`.

| Method                                                | Helius                | Triton                  | Alchemy |
| ----------------------------------------------------- | --------------------- | ----------------------- | ------- |
| `getVersion`                                          | 4.3.2 (19); 4.3.0 (1) | 4.3.2                   | 4.3.2   |
| `getAccountInfo`; `getMultipleAccounts`; `getBalance` | 4.3.0-alpha.2         | 4.3.2                   | 4.3.2   |
| `getLatestBlockhash`                                  | none                  | 4.3.2                   | 4.3.2   |
| `getSignatureStatuses`                                | none                  | none                    | 4.3.2   |
| `getProgramAccounts` of the 239 lending markets       | 4.3.0-alpha.2         | 4.3.0-beta.2            | 4.3.0   |
| `getProgramAccounts` of the ~135,000 obligations      | 3.1.9 (34 of 34)      | 4.3.0-beta.2 (20 of 20) | not run |

Each provider answers `getProgramAccounts` from another build than its account reads. On Helius the account reads come from a build other than the one `getVersion` reports, and the large `getProgramAccounts` from a third one, 3.1.9, the only build the closed obligation came from. Each issue shows only in the answers of one build. Neither Agave version can give its issue's answers: the account-read code cited below is the same in v4.3.0-alpha.2 and v4.3.2, and the `getProgramAccounts` row filter is the same in v3.1.9 and v4.3.2.

## 1. Account data newer than `context.slot`

Some finalized `getMultipleAccounts` and `getAccountInfo` answers return account data from a later slot than the `context.slot` they report, on mainnet and devnet.

### What we see

The `clock` mode reads the Clock sysvar (`SysvarC1ock11111111111111111111111111111111`) and compares:

- `context.slot` in the answer;
- the Clock's own `slot` field, the first 8 bytes of its data, which is the slot of the bank the data was read from.

On Helius, the Clock's slot is sometimes greater than `context.slot`:

```text
read 9 getAccountInfo: context.slot 462666617, Clock slot 462666618 (+1), apiVersion 4.3.0-alpha.2, zid ZTK7L_cqlUWA, cf-ray a51dcfa5cd9ad22a-FRA
read 26 getMultipleAccounts: context.slot 462666643, Clock slot 462666644 (+1), apiVersion 4.3.0-alpha.2, zid LpWDCnulUAyR, cf-ray a51dcfc758e9d22a-FRA
```

| Run               | Endpoint                | `apiVersion` of the answers | Answers with Clock slot ≠ `context.slot`                                            |
| ----------------- | ----------------------- | --------------------------- | ----------------------------------------------------------------------------------- |
| 2026-10-28 23:46Z | Helius mainnet          | 4.3.0-alpha.2               | 14 of 300 (4.7%), all +1: `getMultipleAccounts` 6 of 150; `getAccountInfo` 8 of 150 |
| 2026-10-28 23:46Z | Triton mainnet          | 4.3.2                       | 0 of 300                                                                            |
| 2026-10-27        | Helius mainnet          | not recorded                | 6 of 300 (2.0%), +1; earlier run 11 of 300 (3.7%), +1                               |
| 2026-10-27        | Helius devnet           | not recorded                | 8 of 300 (2.7%), +1 and +2; earlier run 11 of 300, +1 to +5                         |
| 2026-10-27        | Triton mainnet          | not recorded                | 0 of 300                                                                            |
| 2026-10-27        | Alchemy mainnet         | not recorded                | 0 of 300                                                                            |
| 2026-10-27        | `api.devnet.solana.com` | not recorded                | 0 of 300                                                                            |

Without the program, one read of each method with `curl`. Most answers are right: about 1 in 20 has the Clock ahead, so repeat them a few hundred times and compare the two slots:

```sh
curl -s "$RPC_URL" -H 'Content-Type: application/json' -d '{
  "jsonrpc": "2.0", "id": 1, "method": "getAccountInfo",
  "params": ["SysvarC1ock11111111111111111111111111111111", {"encoding": "base64", "commitment": "finalized"}]
}'
curl -s "$RPC_URL" -H 'Content-Type: application/json' -d '{
  "jsonrpc": "2.0", "id": 1, "method": "getMultipleAccounts",
  "params": [["SysvarC1ock11111111111111111111111111111111"], {"encoding": "base64", "commitment": "finalized"}]
}'
```

The Clock's slot is the little-endian `u64` in the first 8 bytes of the account's data (`result.value.data[0]`, or `result.value[0].data[0]` for `getMultipleAccounts`) after base64 decoding; `context.slot` is `result.context.slot`. They should be equal.

### Answers with accounts newer than their own Clock

A batch of the lending program's accounts, read with one finalized `getMultipleAccounts` that also reads the Clock, was compared with the same accounts kept from a Geyser stream. In these answers, `context.slot` and the Clock's slot were the same, while a reserve in the same answer held the value written one slot later: its owner, lamports and data hash the same as the value the stream delivered for that slot.

| Time (UTC)          | Reserve                                        | Lamports in the answer | Answer's `context.slot` and Clock slot | Slot of the value the answer held | `zid`          | `cf-ray`               |
| ------------------- | ---------------------------------------------- | ---------------------- | -------------------------------------- | --------------------------------- | -------------- | ---------------------- |
| 2026-10-29 20:33:54 | `d4A2prbA2whesmvHaL88BH6Ewn5N4bTSU2Ze8P6Bc4Q`  | 60,913,949             | 463017128                              | 463017129 (+1)                    | not recorded   | not recorded           |
| 2026-10-29 20:33:54 | `2gc9Dm1eB6UgVYFBUN9bWks6Kes9PbWSaPaa9DqyvEiN` | 60,913,945             | 463017128                              | 463017129 (+1)                    | not recorded   | not recorded           |
| 2026-10-29 22:19:34 | `3t4JZcueEzTbVP6kLxXrL3VpWx45jDer4eqysweBchNH` | 200,726,443            | 463046706                              | 463046707 (+1)                    | `4fYkhc4AN0Qg` | `a52590040ce3dd73-PRG` |

The first two came in one answer. So each answer held accounts from two banks: its Clock from one slot and these reserves from the next. Comparing at the Clock's slot, which corrects the answers above, does not help here.

### `getProgramAccounts` rows newer than their context slot

Finalized `getProgramAccounts` loads of every reserve and obligation were compared row by row with the value the stream delivered for the answer's `context.slot`. Each row that differed was read again, with one `getMultipleAccounts` that also reads the Clock. In each case below:

- the row differed from the account's value at the answer's `context.slot`;
- the same endpoint's `getMultipleAccounts`, at a Clock slot 2 to 13 slots later, returned exactly the row, its lamports and every byte of its data, and the stream's value at that later slot;

so the row held a write made after the slot the answer names.

| `context.slot` | Clock slot of the read again | Account                                                                                                                                        | Lamports        | `zid` of the `getProgramAccounts` answer |
| -------------- | ---------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------- | --------------- | ---------------------------------------- |
| 463081037      | 463081039                    | `febGYTnFX4GbSGoFHFeJXUHgNaK53fB23uDins9Jp1E`                                                                                                  | 60,913,930      | `uBs-G4M75nw-`                           |
| 463108627      | 463108638                    | `Be1mLVXWUk5RXxJFNPLhLdLfePRvSwMFYVVqjHHd9RhC`                                                                                                 | 24,165,120      | `1WgjzO3DNq6Z`                           |
| 463124065      | 463124073                    | `HyWrRUL6TxJiP8W3pfwYAEbTohRAuaPGY1gq7ocSvkUG`                                                                                                 | 17,637,760      | `XHQBeTiwYBpM`                           |
| 463133917      | 463133930                    | `819TRcMwNGsV5TDqnbgbUiD3of5iqpCbEY4C7p33ohrt`                                                                                                 | 24,165,120      | `IsmGXs9rzlu_`                           |
| 463142345      | 463142353                    | `Gaq4icykJEggaBii8gGKUTWjbiH7u2px1m1jEVm9HSEm`, `7TFVfMDYLRug3RxZVxhHfNQF8vvp8mysgqQvevrcwC9N`                                                 | 21,988,176 each | `_pv9ZvC0wswb`                           |
| 463142345      | 463142353                    | `BiAXvYXhNgdk3YK1XGcJk3r6c1SCT6zdL6sgaiLeAXFC`, `9THm4BoRjhABDKnsnEt1X4UA8X78DFoZqgTjmVTJbdKo`, `3K75spH2k79pyJ5pWcZHacq58aZh2GioPCC6gR5hKXhv` | 17,637,760 each | `_pv9ZvC0wswb`                           |
| 463156381      | 463156390                    | `PC1XgAxn3ResZ4czb1SNNFtDURHM2U3Y3Gbr18FBYK9`                                                                                                  | 17,637,760      | `jdveAJbl898z`                           |

These 10 rows came in 6 answers of the 53 loads of the reserves and of the obligations each, between 2026-10-30 00:22Z and 04:50Z. Each row and each read again were saved, and can be shared.

### Requests to trace

The 14 answers of 2026-10-28 with the Clock one slot ahead:

| Method              | `context.slot` | `zid`          | `cf-ray`               |
| ------------------- | -------------- | -------------- | ---------------------- |
| getAccountInfo      | 462666617      | `ZTK7L_cqlUWA` | `a51dcfa5cd9ad22a-FRA` |
| getMultipleAccounts | 462666643      | `LpWDCnulUAyR` | `a51dcfc758e9d22a-FRA` |
| getMultipleAccounts | 462666660      | `gh0z2ZGZg3l0` | `a51dcfdf1dcfd22a-FRA` |
| getAccountInfo      | 462666664      | `TbXj6LQ5z43e` | `a51dcfe50a1bd22a-FRA` |
| getMultipleAccounts | 462666762      | `KBsgSxRbWvqL` | `a51dd0654c62d22a-FRA` |
| getAccountInfo      | 462666773      | `k67KZ0d6T5NO` | `a51dd0730b22d22a-FRA` |
| getMultipleAccounts | 462666787      | `W3xjjuHLR6ST` | `a51dd084bdecd22a-FRA` |
| getAccountInfo      | 462666812      | `bUzmemDWf_S2` | `a51dd0a63bb1d22a-FRA` |
| getAccountInfo      | 462666870      | `rRT7VE1BSdaS` | `a51dd0f55dd7d22a-FRA` |
| getMultipleAccounts | 462666921      | `KFsSCMJkz3SC` | `a51dd1367d16d22a-FRA` |
| getAccountInfo      | 462666935      | `iB5qeaRhGmtb` | `a51dd1482ebdd22a-FRA` |
| getAccountInfo      | 462666938      | `th27E1Y4e2ij` | `a51dd14c1f12d22a-FRA` |
| getAccountInfo      | 462666955      | `BgbxVBGmDqDs` | `a51dd163afe5d22a-FRA` |
| getMultipleAccounts | 462667049      | `RukI5wkppguo` | `a51dd1dfcc1ed22a-FRA` |

From 400 reads on 2026-10-27, these 12 had the Clock one slot ahead (times UTC):

| Time         | Method              | `context.slot` | Clock slot | `zid`          | `cf-ray`               |
| ------------ | ------------------- | -------------- | ---------- | -------------- | ---------------------- |
| 22:00:40.156 | getAccountInfo      | 462231776      | 462231777  | `jK1pzS6Y0Slt` | `a514f99318b4fcd5-FRA` |
| 22:00:43.000 | getAccountInfo      | 462231790      | 462231791  | `Wk7VzqqQUCA0` | `a514f9a4ef38bb97-FRA` |
| 22:00:57.250 | getAccountInfo      | 462231857      | 462231858  | `cJqtaTzi1ts0` | `a514f9fdee58db0f-FRA` |
| 22:01:01.957 | getAccountInfo      | 462231880      | 462231881  | `6Bvv0X9W8_G2` | `a514fa1b5e2a65b0-FRA` |
| 22:01:15.928 | getMultipleAccounts | 462231946      | 462231947  | `AZqivv6wKs1H` | `a514fa72a82f613b-FRA` |
| 22:01:28.193 | getAccountInfo      | 462232003      | 462232004  | `2tdhHuh1ghaH` | `a514fabf5ec3de45-FRA` |
| 22:01:29.046 | getMultipleAccounts | 462232007      | 462232008  | `D3l9YYcre8Ol` | `a514fac4acbd8f1a-FRA` |
| 22:01:40.987 | getMultipleAccounts | 462232065      | 462232066  | `R0NMy9IjsqlB` | `a514fb0f4d021cbf-FRA` |
| 22:01:56.774 | getAccountInfo      | 462232140      | 462232141  | `EK9QXxo7Q1LB` | `a514fb71fb9e39ce-FRA` |
| 22:02:14.812 | getMultipleAccounts | 462232225      | 462232226  | `IEYOa_T3RZVn` | `a514fbe2bb998dd9-FRA` |
| 22:02:19.934 | getMultipleAccounts | 462232247      | 462232248  | `JWCFYrozeO3i` | `a514fc02bbb03038-FRA` |
| 22:02:21.082 | getMultipleAccounts | 462232252      | 462232253  | `ixLX0c-MtyFG` | `a514fc09ee955e09-FRA` |

The headers show no other difference between these answers and the rest:

- **The same path:** every answer came through the Frankfurt edge with `cf-cache-status: DYNAMIC` and a `server-timing` of `cfEdge` 1–5 ms, `cfOrigin;dur=0` and `cfWorker` 4–14 ms. So a Worker built each answer, and no origin request went through Cloudflare.
- **The same timing:** the median round trip was 34 ms for the answers ahead and 33 ms for the rest.

| Provider | Clock ahead (2026-10-27) | How the headers say it was served                                                                                                                                     |
| -------- | ------------------------ | --------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Helius   | 12 of 400                | a Cloudflare Worker, with no header naming the node behind it                                                                                                         |
| Triton   | 0 of 400                 | a load balancer over 5 nodes, with `x-rpc-node` naming the node that answered each request (`lb-fra9` 106; `lb-fra13` 101; `lb-fra12` 96; `lb-fra7` 51; `lb-fra8` 46) |
| Alchemy  | 0 of 400                 | `istio-envoy`, with the upstream's time in `x-envoy-upstream-service-time` (1–6 ms) and an `x-alchemy-trace-id` on each answer                                        |

Triton spreads requests over several nodes and still never breaks the rule, because each answer comes whole from one node.

### Why the two must be equal

In Agave v4.3.2, one node cannot give an answer where they differ, and v4.3.0-alpha.2 has the same `getMultipleAccounts`, `getAccountInfo` and `new_response` code:

- `getAccountInfo` and `getMultipleAccounts` each take one bank from `get_bank_with_config` ([rpc.rs#L548](https://github.com/anza-xyz/agave/blob/v4.3.2/rpc/src/rpc.rs#L548), [#L576](https://github.com/anza-xyz/agave/blob/v4.3.2/rpc/src/rpc.rs#L576)).
- They read every account from that bank: `get_encoded_account` calls `bank.get_account` ([rpc.rs#L2580](https://github.com/anza-xyz/agave/blob/v4.3.2/rpc/src/rpc.rs#L2580), [account_resolver.rs#L6](https://github.com/anza-xyz/agave/blob/v4.3.2/rpc/src/rpc/account_resolver.rs#L6)).
- `context.slot` is that same bank's `slot()`, set by `new_response` ([rpc.rs#L145](https://github.com/anza-xyz/agave/blob/v4.3.2/rpc/src/rpc.rs#L145), called at [#L562](https://github.com/anza-xyz/agave/blob/v4.3.2/rpc/src/rpc.rs#L562) and [#L594](https://github.com/anza-xyz/agave/blob/v4.3.2/rpc/src/rpc.rs#L594)).
- Every write of a bank's Clock sets `slot: self.slot`:
  - `update_clock` at the start of a PoH bank ([bank.rs#L2492](https://github.com/anza-xyz/agave/blob/v4.3.2/runtime/src/bank.rs#L2492));
  - `update_clock_slot_for_alpenglow` at the start of an Alpenglow bank ([#L2522](https://github.com/anza-xyz/agave/blob/v4.3.2/runtime/src/bank.rs#L2522));
  - `update_clock_from_footer` at its end ([#L3493](https://github.com/anza-xyz/agave/blob/v4.3.2/runtime/src/bank.rs#L3493));
  - `warp_from_parent`, which keeps the slot and changes only the timestamps ([#L1957](https://github.com/anza-xyz/agave/blob/v4.3.2/runtime/src/bank.rs#L1957)).

So the Clock in an answer is always at `context.slot`. An answer whose Clock is at a later slot has its accounts read from a later bank than its context, or from something other than the bank its context names.

### Why it matters

A client that orders account data by `context.slot`, such as one that compares an RPC read with the same accounts kept from a Geyser stream, treats the newer data as the value at the older slot. Every account the later slot changed then looks wrong for that read. The Clock shows it in nearly every such answer only because it changes every slot; other accounts in the same answers are at the later slot's value too.

### What we expect

`context.slot` is the slot of the bank every account in the answer was read from, as on Triton, Alchemy and the public endpoint.

### How an account index keeps the rule

An index can keep the rule as well. [Cloudbreak](https://github.com/solana-rpc/cloudbreak), an open-source index that serves `getAccountInfo`, `getMultipleAccounts` and `getProgramAccounts` from Postgres, does it in three steps:

1. It takes the newest slot at the requested commitment ([get_account_info.rs#L37](https://github.com/solana-rpc/cloudbreak/blob/main/crates/api/src/methods/get_account_info.rs#L37)).
2. It reads each account's newest version at or below that slot, with `slot <= $2`:
   - [getAccountInfo.sql#L31](https://github.com/solana-rpc/cloudbreak/blob/main/crates/api/src/db/getAccountInfo.sql#L31);
   - [getMultipleAccounts.sql#L46](https://github.com/solana-rpc/cloudbreak/blob/main/crates/api/src/db/getMultipleAccounts.sql#L46);
   - [getProgramAccounts.sql#L21](https://github.com/solana-rpc/cloudbreak/blob/main/crates/api/src/db/getProgramAccounts.sql#L21).
3. It returns that slot as `context.slot` ([get_account_info.rs#L126](https://github.com/solana-rpc/cloudbreak/blob/main/crates/api/src/methods/get_account_info.rs#L126)).

The table can already hold rows of later slots; the `slot <=` bound leaves them out. The slot it takes is also complete:

- Its indexer handles the Yellowstone stream one message at a time and writes all of a block's accounts before the next message ([indexer.rs#L293](https://github.com/solana-rpc/cloudbreak/blob/main/crates/index/src/indexer.rs#L293), [save_block.rs#L257](https://github.com/solana-rpc/cloudbreak/blob/main/crates/index/src/modules/save_block.rs#L257)).
- Yellowstone sends a slot's block before its finalized status, filling in any status level Agave skipped, as under Alpenglow ([block_reconstruction_v2.rs#L743](https://github.com/rpcpool/yellowstone-grpc/blob/f49d0fb18c648559d13e17d643724286ef304436/yellowstone-grpc-geyser/src/block_reconstruction_v2.rs#L743), [grpc.rs#L1397-L1416](https://github.com/rpcpool/yellowstone-grpc/blob/f49d0fb18c648559d13e17d643724286ef304436/yellowstone-grpc-geyser/src/grpc.rs#L1397-L1416)).
- So a slot is marked finalized only after its accounts are written. The one exception is a reconnect that cannot replay the missed blocks: a slot can then be marked finalized without them until the next block shows the gap, about one slot, and the index then stops answering until the gap is filled ([self_healing.rs#L81](https://github.com/solana-rpc/cloudbreak/blob/main/crates/index/src/modules/self_healing.rs#L81)).

| Index            | Accounts newer than `context.slot` | Accounts older than `context.slot`             |
| ---------------- | ---------------------------------- | ---------------------------------------------- |
| Helius (mainnet) | 2.0% to 4.7% of answers            | not measured                                   |
| Cloudbreak       | never: the `slot <=` bound         | only for about one slot after such a reconnect |

## 2. A closed account in `getProgramAccounts`

Some `getProgramAccounts` answers hold an obligation that was closed 23 days earlier, with its values from before the close. The same endpoint's `getMultipleAccounts` returns it as null.

### The account

`7bkKsyFNrNs1GrrkeuN3HQ6wwb85ECxp7YRDZZedo8sW`, an obligation of the lending program `KLend2g3cP87fffoy8q1mQqGKjrxjC8boSyAYavgmjD`:

- **Closed** by transaction [`5DLVGcdN…bawSVw6ea`](https://explorer.solana.com/tx/5DLVGcdNXoiY6a1KyzehWFUN71NV8DxLhasYegE1dUU7Fy6uW1AXc1kQjJwRLiAkNZ9W28menX4DQTxbawSVw6ea) at slot 453889261 (2026-10-06 11:33:07 UTC). Its collateral withdrawal instruction logged `Closing account`, and the account's lamports went from 24,165,120 to 0.
- **Untouched since:** `getSignaturesForAddress` on both Helius and Triton lists no later transaction.
- **Gone now:** `getMultipleAccounts` returns `null` for it on Helius (`apiVersion` 4.3.0-alpha.2) and on Triton.

### What we see

The `gpa` mode asks for the address of every account of the program with an obligation's size and discriminator, with a `dataSlice` of 0 bytes. The same call with `curl`. Most answers are right: about 1 in 6 held the closed obligation, in runs of a few minutes, so repeat it every few seconds and look for it (each answer is about 31 MB):

```sh
curl -s "$RPC_URL" -H 'Content-Type: application/json' -d '{
  "jsonrpc": "2.0", "id": 1, "method": "getProgramAccounts",
  "params": ["KLend2g3cP87fffoy8q1mQqGKjrxjC8boSyAYavgmjD", {"encoding": "base64", "commitment": "finalized", "withContext": true,
    "dataSlice": {"offset": 0, "length": 0},
    "filters": [{"dataSize": 3344}, {"memcmp": {"offset": 0, "bytes": "VEdzkJnDweW"}}]}]
}' | jq -c '{slot: .result.context.slot, apiVersion: .result.context.apiVersion, obligations: (.result.value | length),
  closed: [.result.value[] | select(.pubkey == "7bkKsyFNrNs1GrrkeuN3HQ6wwb85ECxp7YRDZZedo8sW")]}'
```

`closed` should always be empty.

Some Helius answers hold the closed obligation, as it was before the close:

```json
{
  "pubkey": "7bkKsyFNrNs1GrrkeuN3HQ6wwb85ECxp7YRDZZedo8sW",
  "account": {
    "lamports": 24165120,
    "data": ["", "base64"],
    "owner": "KLend2g3cP87fffoy8q1mQqGKjrxjC8boSyAYavgmjD",
    "executable": false,
    "rentEpoch": 18446744073709551615,
    "space": 3344
  }
}
```

Apart from it, the answer is current: in each of the 3 answers of the 30-second run that held it, the closed obligation was the only address that Triton's answer of a few hundred slots earlier lacked, and the answer's count was one more than that of the Helius answers without it. So the answer comes from an up-to-date index that kept this one closed account, not from a node behind the chain.

All runs were on 2026-10-28 between 23:07Z and 23:51Z:

| Run                                                       | Encoding                                  | Helius answers holding it | Triton answers holding it |
| --------------------------------------------------------- | ----------------------------------------- | ------------------------- | ------------------------- |
| `gpa` mode, 20 calls 15 s apart                           | base64, `dataSlice` 0                     | 3 of 20                   | 0 of 20                   |
| 6 calls 10 s apart                                        | base64, `dataSlice` 0                     | 3 of 6                    | not run                   |
| one call every 30 s                                       | base64, `dataSlice` 0                     | 3 of 29                   | 0 of 10                   |
| one call every 2 min                                      | base64+zstd, whole data, `minContextSlot` | 0 of 9                    | not run                   |
| a client's start, full load                               | base64+zstd, whole data, `minContextSlot` | 3 of 3                    | 0 of 3                    |
| the same client's reloads, every 15 min then every minute | base64+zstd, whole data, `minContextSlot` | 0 of 10                   | 0 of 10                   |

Every Helius `getProgramAccounts` answer of all obligations reported `apiVersion` 3.1.9, with or without the closed obligation (34 of 34 recorded), while one of the 239 lending markets reports 4.3.0-alpha.2; Triton's reported 4.3.0-beta.2.

### Requests to trace

| `context.slot` | `apiVersion` | `zid`          | `cf-ray`               |
| -------------- | ------------ | -------------- | ---------------------- |
| 462663879      | 3.1.9        | not recorded   | not recorded           |
| 462665872      | 3.1.9        | `BpJUlA27uaWD` | `a51dcbb90bd9cef6-FRA` |
| 462665927      | 3.1.9        | `X_-9PYimH0Lu` | `a51dcc016e12d26d-FRA` |
| 462665982      | 3.1.9        | `1j3MtzunoYl0` | `a51dcc49cdfe18e3-FRA` |
| 462667058      | 3.1.9        | `AWpnE-qVwVef` | `a51dd1f0783dd2ca-FRA` |
| 462667397      | 3.1.9        | `p09ocwQzhn-K` | `a51dd3aeca62d2ca-FRA` |
| 462667701      | 3.1.9        | `N2Ks8TwoZkPb` | `a51dd5413d45d2ca-FRA` |

### Why Agave cannot return it

Agave v4.3.2 (and v3.1.9, with the same code) keeps a `getProgramAccounts` row only for an account whose newest version in the bank has lamports and the program as owner:

- Without an account index, `get_filtered_program_accounts` scans the bank's accounts with `load_by_program_with_filter` ([rpc.rs#L2265](https://github.com/anza-xyz/agave/blob/v4.3.2/rpc/src/rpc.rs#L2265), [bank.rs#L5164](https://github.com/anza-xyz/agave/blob/v4.3.2/runtime/src/bank.rs#L5164), [accounts.rs#L338](https://github.com/anza-xyz/agave/blob/v4.3.2/accounts-db/src/accounts.rs#L338)).
- With the program-id index, `get_filtered_indexed_accounts` checks the owner again, for accounts the index still lists after they were wiped ([rpc.rs#L309](https://github.com/anza-xyz/agave/blob/v4.3.2/rpc/src/rpc.rs#L309), [accounts.rs#L396](https://github.com/anza-xyz/agave/blob/v4.3.2/accounts-db/src/accounts.rs#L396)).
- Both keep a row only through `load_while_filtering`, which drops an account with zero lamports ([accounts.rs#L304-L310](https://github.com/anza-xyz/agave/blob/v4.3.2/accounts-db/src/accounts.rs#L304-L310), [is_loadable.rs#L10-L13](https://github.com/anza-xyz/agave/blob/v4.3.2/accounts-db/src/is_loadable.rs#L10-L13)).

The close left the account with zero lamports (the transaction's `postBalances`), and Anchor's `close`, which the lending program uses, also assigns it to the System program, so no Agave bank since slot 453889261 can return it.

### Why it matters

A client that loads a program's accounts with `getProgramAccounts` keeps the closed account as live, with its old balances. For a lending obligation that means a position that no longer exists, which a liquidator or a risk check may act on.

### What we expect

`getProgramAccounts` returns only accounts that exist at `context.slot`, as `getMultipleAccounts` on the same endpoint, and Triton's `getProgramAccounts`, do.

## 3. A rate limit sent as an invalid request

Helius refused valid `getProgramAccounts` calls with JSON-RPC error -32600, the code for a request that is not a valid request object, and a message that reads as a rate limit.

### What we see

On 2026-10-29 a client loading the lending markets, reserves and obligations with `getProgramAccounts` at start, all three together, whole and `base64+zstd`, got this answer to four of those calls:

```json
{
  "jsonrpc": "2.0",
  "id": 1,
  "error": {
    "code": -32600,
    "message": "Request deprioritized due to number of accounts requested. Please use getProgramAccountsV2 with pagination to handle large datasets. See: https://www.helius.dev/docs/api-reference/rpc/http/getprogramaccountsv2"
  }
}
```

| Time (UTC)   | Accounts asked for  | Accounts the program has |
| ------------ | ------------------- | ------------------------ |
| 00:06:11.719 | the lending markets | 239                      |
| 00:06:17.171 | the obligations     | about 135,000            |
| 00:06:22.478 | the reserves        | not counted              |
| 00:06:28.371 | the reserves        | not counted              |

On 2026-10-30 a load of the obligations got the same answer at 01:45:14Z and again on its retry 11 s later, with the HTTP status and headers recorded:

```text
HTTP 200 OK
content-type: application/json
content-length: 270
cf-ray: a526bd45da6ab342-PRG
cf-cache-status: DYNAMIC
server: cloudflare
zid: K1xxEXBC_UJm
```

No `Retry-After` header, and no other header that says how long to wait.

- **Refused at once:** each answer came within 40 ms of the client sending its loads, so the call was refused, not timed out.
- **Not 429:** the answer comes with HTTP 200 and no `Retry-After`, so a client sees an invalid request, not a rate limit.
- **Not the size:** the message names the number of accounts, but the 239 lending markets, 1.1 MB of account data, were refused too, while the same calls are answered at other times.
- **Not reproduced on demand:** on 2026-10-29 every call of two runs was answered with HTTP 200:
  - at about 08:50Z, the `deprioritized` mode sent the same three calls together for 10 rounds, back to back: 30 calls;
  - at 09:20Z, the `deprioritized-markets` mode sent 100 calls for the lending markets at once, as one burst, the way a client sends its loads together.

### Why it is wrong

- JSON-RPC 2.0 gives -32600 to "The JSON sent is not a valid Request object" ([section 5.1](https://www.jsonrpc.org/specification#error_object)). These were valid requests, answered before and after.
- Helius documents its rate limits as HTTP 429 ([error codes](https://www.helius.dev/docs/faqs/error-codes#why-am-i-getting-a-429-error)), with a `Retry-After` header in its own example of handling one ([authentication](https://www.helius.dev/docs/api-reference/authentication#handling-rate-limits)).

### Why it matters

A client stops retrying a request the node calls invalid, since sending it again cannot help. A client that loads its state at start then fails the start on a refusal that would have passed a few seconds later, and restarts until its loads go through.

### What we expect

A rate limit answered as HTTP 429, with JSON-RPC code 429 if any, and a `Retry-After` header that says how long to wait.
