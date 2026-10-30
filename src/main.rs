//! Reproduces two kinds of finalized RPC answers that Agave cannot give, and prints each answer's `apiVersion` and the
//! `zid` and `cf-ray` headers that identify the request:
//!
//! - `clock`: reads the Clock sysvar, alternating getMultipleAccounts and getAccountInfo, and reports every answer
//!   whose Clock slot (the first 8 bytes of the account's data, the slot of the bank the data was read from) differs
//!   from the answer's `context.slot`.
//! - `gpa`: reads the address of every obligation of the lending program with getProgramAccounts and reports every answer that
//!   holds obligation 7bkKsyFNrNs1GrrkeuN3HQ6wwb85ECxp7YRDZZedo8sW, which the program closed at slot 453889261; it
//!   then reads that account with getMultipleAccounts.
//!
//! - `versions`: calls getVersion and every method below whose answer has a context, once a round, and counts the
//!   `apiVersion` each reports.
//! - `deprioritized`: sends the getProgramAccounts of every lending market, reserve and obligation together,
//!   round after round, until an answer is the JSON-RPC error -32600 "Request deprioritized", whose HTTP status and
//!   headers it prints; it then asks for the lending markets alone until they are answered. `deprioritized-markets`
//!   sends `reads` loads of the lending markets alone at once, as one burst.
//!
//! `clock`, `gpa` and `deprioritized` stop at the first wrong answer.
//!
//! Usage: `RPC_URL=<url> cargo run --release -- clock|gpa|versions|deprioritized|deprioritized-markets [reads]
//! [interval-ms]`, by default 300 reads 300 ms apart.

use std::{collections::BTreeMap, env, thread, time::Duration};

use serde::de::{DeserializeOwned, IgnoredAny};
use solana_account_decoder_client_types::{UiAccount, UiAccountEncoding, UiDataSliceConfig};
use solana_commitment_config::CommitmentConfig;
use solana_rpc_client_api::{
    config::{RpcAccountInfoConfig, RpcProgramAccountsConfig},
    filter::{Memcmp, RpcFilterType},
    request::RpcRequest,
    response::{OptionalContext, Response, RpcKeyedAccount, RpcResponseContext, RpcVersionInfo},
};

const LENDING_PROGRAM: &str = "KLend2g3cP87fffoy8q1mQqGKjrxjC8boSyAYavgmjD";
/// The size of an obligation account of the lending program, and its Anchor discriminator (the first 8 bytes of
/// sha256("account:Obligation")).
const OBLIGATION_SIZE: u64 = 3344;
const OBLIGATION_DISCRIMINATOR: [u8; 8] = [168, 206, 141, 106, 88, 76, 172, 167];
/// Closed by a collateral withdrawal in transaction `CLOSING_TRANSACTION`, slot
/// 453889261.
const CLOSED_OBLIGATION: &str = "7bkKsyFNrNs1GrrkeuN3HQ6wwb85ECxp7YRDZZedo8sW";
/// The transaction that closed it.
const CLOSING_TRANSACTION: &str =
    "5DLVGcdNXoiY6a1KyzehWFUN71NV8DxLhasYegE1dUU7Fy6uW1AXc1kQjJwRLiAkNZ9W28menX4DQTxbawSVw6ea";
/// The size of a lending market account, and its Anchor discriminator.
const LENDING_MARKET_SIZE: u64 = 4664;
const LENDING_MARKET_DISCRIMINATOR: [u8; 8] = [246, 114, 50, 98, 72, 157, 28, 120];
/// The size of a reserve account, and its Anchor discriminator.
const RESERVE_SIZE: u64 = 8624;
const RESERVE_DISCRIMINATOR: [u8; 8] = [43, 242, 204, 202, 26, 247, 59, 127];
/// The JSON-RPC error code Helius answers a deprioritized getProgramAccounts with.
const INVALID_REQUEST: i64 = -32600;

/// An answer's result, with the headers that identify its request.
struct Answer<T> {
    result: T,
    zid: String,
    cf_ray: String,
}

struct Client {
    http: reqwest::blocking::Client,
    url: String,
}

/// A JSON-RPC answer with the HTTP status and headers it came with.
struct Raw {
    status: reqwest::StatusCode,
    headers: reqwest::header::HeaderMap,
    output: jsonrpc_core::Output,
}

impl Raw {
    fn header(&self, name: &str) -> String {
        self.headers
            .get(name)
            .and_then(|value| value.to_str().ok())
            .unwrap_or("none")
            .to_owned()
    }
}

impl Client {
    /// Sends one request, and returns the answer as it came.
    fn post(&self, request: RpcRequest, params: serde_json::Value) -> Result<Raw, String> {
        let response = self
            .http
            .post(&self.url)
            .json(&request.build_request_json(1, params))
            .send()
            .map_err(|e| format!("{request}: {e}"))?;
        let (status, headers) = (response.status(), response.headers().clone());
        let output = response.json().map_err(|e| format!("{request}: {e}"))?;
        Ok(Raw {
            status,
            headers,
            output,
        })
    }

    fn call<T: DeserializeOwned>(
        &self,
        request: RpcRequest,
        params: serde_json::Value,
    ) -> Result<Answer<T>, String> {
        let raw = self.post(request, params)?;
        let (zid, cf_ray) = (raw.header("zid"), raw.header("cf-ray"));
        match raw.output {
            jsonrpc_core::Output::Success(success) => {
                let result = serde_json::from_value(success.result)
                    .map_err(|e| format!("{request}: {e}"))?;
                Ok(Answer {
                    result,
                    zid,
                    cf_ray,
                })
            }
            jsonrpc_core::Output::Failure(failure) => Err(format!(
                "{request}: {} {} (zid {zid}, cf-ray {cf_ray})",
                failure.error.code.code(),
                failure.error.message
            )),
        }
    }

    fn version(&self) -> String {
        match self.call::<RpcVersionInfo>(RpcRequest::GetVersion, serde_json::Value::Null) {
            Ok(answer) => format!(
                "solana-core {}, feature-set {:?}",
                answer.result.solana_core, answer.result.feature_set
            ),
            Err(error) => format!("getVersion failed: {error}"),
        }
    }
}

fn api_version(context: &RpcResponseContext) -> String {
    context
        .api_version
        .as_ref()
        .map_or_else(|| "none".to_owned(), |version| version.to_string())
}

/// One read of the Clock: the answer's context, the Clock's own slot, and the request's headers.
struct ClockRead {
    context: RpcResponseContext,
    clock_slot: u64,
    zid: String,
    cf_ray: String,
}

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum ClockMethod {
    GetMultipleAccounts,
    GetAccountInfo,
}

impl ClockMethod {
    fn request(self) -> RpcRequest {
        match self {
            Self::GetMultipleAccounts => RpcRequest::GetMultipleAccounts,
            Self::GetAccountInfo => RpcRequest::GetAccountInfo,
        }
    }
}

fn finalized_base64(data_slice: Option<UiDataSliceConfig>) -> RpcAccountInfoConfig {
    RpcAccountInfoConfig {
        encoding: Some(UiAccountEncoding::Base64),
        data_slice,
        commitment: Some(CommitmentConfig::finalized()),
        min_context_slot: None,
    }
}

fn read_clock(client: &Client, method: ClockMethod) -> Result<ClockRead, String> {
    let clock = solana_sdk_ids::sysvar::clock::ID.to_string();
    let config = finalized_base64(None);
    let (context, account, zid, cf_ray) = match method {
        ClockMethod::GetMultipleAccounts => {
            let answer: Answer<Response<Vec<Option<UiAccount>>>> = client.call(
                method.request(),
                serde_json::to_value(([&clock], &config)).expect("the params serialize"),
            )?;
            let account = answer.result.value.into_iter().next().flatten();
            (answer.result.context, account, answer.zid, answer.cf_ray)
        }
        ClockMethod::GetAccountInfo => {
            let answer: Answer<Response<Option<UiAccount>>> = client.call(
                method.request(),
                serde_json::to_value((&clock, &config)).expect("the params serialize"),
            )?;
            (
                answer.result.context,
                answer.result.value,
                answer.zid,
                answer.cf_ray,
            )
        }
    };
    let request = method.request();
    let account = account.ok_or_else(|| format!("{request}: no Clock account"))?;
    let data = account
        .data
        .decode()
        .ok_or_else(|| format!("{request}: the Clock's data does not decode"))?;
    let Some(slot_bytes) = data
        .get(..8)
        .and_then(|bytes| <[u8; 8]>::try_from(bytes).ok())
    else {
        return Err(format!(
            "{request}: the Clock's data is {} bytes",
            data.len()
        ));
    };
    Ok(ClockRead {
        context,
        clock_slot: u64::from_le_bytes(slot_bytes),
        zid,
        cf_ray,
    })
}

fn clock(client: &Client, reads: u32, interval: Duration) {
    // How many answers had each Clock slot minus context.slot, by method and apiVersion.
    let mut deltas: BTreeMap<(ClockMethod, String, i64), u32> = BTreeMap::new();
    let mut failures: BTreeMap<ClockMethod, u32> = BTreeMap::new();
    for i in 0..reads {
        let method = if i % 2 == 0 {
            ClockMethod::GetMultipleAccounts
        } else {
            ClockMethod::GetAccountInfo
        };
        match read_clock(client, method) {
            Ok(ClockRead {
                context,
                clock_slot,
                zid,
                cf_ray,
            }) => {
                let delta = i64::try_from(clock_slot).unwrap_or(i64::MAX)
                    - i64::try_from(context.slot).unwrap_or(i64::MAX);
                let version = api_version(&context);
                if delta != 0 {
                    println!(
                        "read {i} {}: context.slot {}, Clock slot {clock_slot} ({delta:+}), apiVersion {version}, zid {zid}, cf-ray {cf_ray}",
                        method.request(),
                        context.slot
                    );
                }
                *deltas.entry((method, version, delta)).or_default() += 1;
                if delta != 0 {
                    break;
                }
            }
            Err(error) => {
                println!("read {i} failed: {error}");
                *failures.entry(method).or_default() += 1;
            }
        }
        thread::sleep(interval);
    }

    for ((method, version, delta), count) in &deltas {
        println!(
            "{} apiVersion {version}: Clock slot minus context.slot {delta:+} in {count} answers",
            method.request()
        );
    }
    for (method, count) in &failures {
        println!("{}: {count} failed", method.request());
    }
}

fn gpa(client: &Client, calls: u32, interval: Duration) {
    let config = RpcProgramAccountsConfig {
        filters: Some(vec![
            RpcFilterType::DataSize(OBLIGATION_SIZE),
            RpcFilterType::Memcmp(Memcmp::new_raw_bytes(0, OBLIGATION_DISCRIMINATOR.to_vec())),
        ]),
        account_config: finalized_base64(Some(UiDataSliceConfig {
            offset: 0,
            length: 0,
        })),
        with_context: Some(true),
        sort_results: None,
    };
    let params = serde_json::to_value((LENDING_PROGRAM, &config)).expect("the params serialize");
    // How many answers held the closed obligation and how many did not, by apiVersion.
    let mut held: BTreeMap<String, (u32, u32)> = BTreeMap::new();
    for i in 0..calls {
        let answer: Result<Answer<OptionalContext<Vec<RpcKeyedAccount>>>, String> =
            client.call(RpcRequest::GetProgramAccounts, params.clone());
        match answer.map(|answer| (answer.result, answer.zid, answer.cf_ray)) {
            Ok((OptionalContext::Context(Response { context, value }), zid, cf_ray)) => {
                let closed = value.iter().find(|keyed| keyed.pubkey == CLOSED_OBLIGATION);
                let version = api_version(&context);
                let counts = held.entry(version.clone()).or_default();
                match closed {
                    Some(RpcKeyedAccount { account, .. }) => {
                        counts.0 += 1;
                        println!(
                            "call {i}: context.slot {}, apiVersion {version}, {} obligations, HOLDS {CLOSED_OBLIGATION} with {} lamports, owner {}, space {:?}; zid {zid}, cf-ray {cf_ray}",
                            context.slot,
                            value.len(),
                            account.lamports,
                            account.owner,
                            account.space
                        );
                        break;
                    }
                    None => {
                        counts.1 += 1;
                        println!(
                            "call {i}: context.slot {}, apiVersion {version}, {} obligations, without it; zid {zid}, cf-ray {cf_ray}",
                            context.slot,
                            value.len()
                        );
                    }
                }
            }
            Ok((OptionalContext::NoContext(_), zid, cf_ray)) => {
                println!(
                    "call {i}: an answer without the context asked for; zid {zid}, cf-ray {cf_ray}"
                );
            }
            Err(error) => println!("call {i} failed: {error}"),
        }
        thread::sleep(interval);
    }

    for (version, (holding, without)) in &held {
        println!("getProgramAccounts apiVersion {version}: {holding} answers held the closed obligation, {without} did not");
    }
    let info: Result<Answer<Response<Option<UiAccount>>>, String> = client.call(
        RpcRequest::GetAccountInfo,
        serde_json::to_value((
            CLOSED_OBLIGATION,
            finalized_base64(Some(UiDataSliceConfig {
                offset: 0,
                length: 0,
            })),
        ))
        .expect("the params serialize"),
    );
    match info {
        Ok(answer) => println!(
            "getAccountInfo of {CLOSED_OBLIGATION}: context.slot {}, apiVersion {}, account {}",
            answer.result.context.slot,
            api_version(&answer.result.context),
            if answer.result.value.is_some() {
                "present"
            } else {
                "null"
            }
        ),
        Err(error) => println!("getAccountInfo of {CLOSED_OBLIGATION} failed: {error}"),
    }
    let read: Result<Answer<Response<Vec<Option<UiAccount>>>>, String> = client.call(
        RpcRequest::GetMultipleAccounts,
        serde_json::to_value((
            [CLOSED_OBLIGATION],
            finalized_base64(Some(UiDataSliceConfig {
                offset: 0,
                length: 0,
            })),
        ))
        .expect("the params serialize"),
    );
    match read {
        Ok(answer) => println!(
            "getMultipleAccounts of {CLOSED_OBLIGATION}: context.slot {}, apiVersion {}, account {}",
            answer.result.context.slot,
            api_version(&answer.result.context),
            if answer.result.value.first().is_some_and(Option::is_some) { "present" } else { "null" }
        ),
        Err(error) => println!("getMultipleAccounts of {CLOSED_OBLIGATION} failed: {error}"),
    }
}

fn main() {
    let url = env::var("RPC_URL").expect("RPC_URL is the RPC endpoint to read");
    let mut args = env::args().skip(1);
    let mode = args.next().expect("the first argument is clock or gpa");
    let reads: u32 = args
        .next()
        .map_or(300, |reads| reads.parse().expect("reads is a number"));
    let interval = Duration::from_millis(args.next().map_or(300, |interval| {
        interval.parse().expect("interval-ms is a number")
    }));
    let client = Client {
        http: reqwest::blocking::Client::builder()
            .timeout(Duration::from_secs(120))
            .build()
            .expect("an HTTP client builds"),
        url,
    };

    println!("{}", client.version());
    match mode.as_str() {
        "clock" => clock(&client, reads, interval),
        "gpa" => gpa(&client, reads, interval),
        "versions" => versions(&client, reads, interval),
        "deprioritized" => deprioritized(&client, &start_loads(), reads, interval),
        "deprioritized-markets" => deprioritized(&client, &markets_burst(reads), 1, interval),
        other => panic!(
            "unknown mode {other}: clock, gpa, versions, deprioritized or deprioritized-markets"
        ),
    }
}

/// The getProgramAccounts params of every account of the lending program of `size` and `discriminator`, whole and zstd-compressed,
/// as an account state loads them.
fn program_accounts_params(size: u64, discriminator: [u8; 8]) -> serde_json::Value {
    let config = RpcProgramAccountsConfig {
        filters: Some(vec![
            RpcFilterType::DataSize(size),
            RpcFilterType::Memcmp(Memcmp::new_raw_bytes(0, discriminator.to_vec())),
        ]),
        account_config: RpcAccountInfoConfig {
            encoding: Some(UiAccountEncoding::Base64Zstd),
            data_slice: None,
            commitment: Some(CommitmentConfig::finalized()),
            min_context_slot: None,
        },
        with_context: Some(true),
        sort_results: None,
    };
    serde_json::to_value((LENDING_PROGRAM, &config)).expect("the params serialize")
}

/// The JSON-RPC error of an answer, with its HTTP status, or `None` for a result.
fn rpc_error(raw: &Raw) -> Option<(i64, String)> {
    match &raw.output {
        jsonrpc_core::Output::Success(_) => None,
        jsonrpc_core::Output::Failure(failure) => {
            Some((failure.error.code.code(), failure.error.message.clone()))
        }
    }
}

/// Prints an answer's HTTP status, JSON-RPC error and every header.
fn print_raw(label: &str, raw: &Raw) {
    let error = rpc_error(raw).map_or_else(
        || "a result".to_owned(),
        |(code, message)| format!("error {code}: {message}"),
    );
    println!("{label}: HTTP {}, {error}", raw.status);
    for (name, value) in &raw.headers {
        println!("  {name}: {}", value.to_str().unwrap_or("(not text)"));
    }
}

/// The lending markets, reserves and obligations, as a client loads them at start.
fn start_loads() -> Vec<(String, serde_json::Value)> {
    vec![
        (
            "lending markets".to_owned(),
            program_accounts_params(LENDING_MARKET_SIZE, LENDING_MARKET_DISCRIMINATOR),
        ),
        (
            "reserves".to_owned(),
            program_accounts_params(RESERVE_SIZE, RESERVE_DISCRIMINATOR),
        ),
        (
            "obligations".to_owned(),
            program_accounts_params(OBLIGATION_SIZE, OBLIGATION_DISCRIMINATOR),
        ),
    ]
}

/// `calls` loads of the lending markets, sent together as one burst.
fn markets_burst(calls: u32) -> Vec<(String, serde_json::Value)> {
    let params = program_accounts_params(LENDING_MARKET_SIZE, LENDING_MARKET_DISCRIMINATOR);
    (0..calls)
        .map(|call| (format!("lending markets {call}"), params.clone()))
        .collect()
}

/// Sends `loads` together, `rounds` times, until an answer is -32600, then asks for the lending markets alone every
/// `interval` until they are answered.
fn deprioritized(
    client: &Client,
    loads: &[(String, serde_json::Value)],
    rounds: u32,
    interval: Duration,
) {
    let mut refused = None;
    let mut counts = (0, 0);
    for round in 0..rounds {
        let answers: Vec<(&str, Result<Raw, String>)> = thread::scope(|scope| {
            let calls: Vec<_> = loads
                .iter()
                .map(|(name, params)| {
                    scope.spawn(move || {
                        (
                            name.as_str(),
                            client.post(RpcRequest::GetProgramAccounts, params.clone()),
                        )
                    })
                })
                .collect();
            calls
                .into_iter()
                .map(|call| call.join().expect("a call thread ends"))
                .collect()
        });
        for (name, answer) in answers {
            match answer {
                Ok(raw) => {
                    let summary = rpc_error(&raw).map_or_else(
                        || "answered".to_owned(),
                        |(code, _)| format!("error {code}"),
                    );
                    println!(
                        "round {round} {name}: HTTP {}, {summary}; zid {}, cf-ray {}",
                        raw.status,
                        raw.header("zid"),
                        raw.header("cf-ray")
                    );
                    let is_refusal =
                        rpc_error(&raw).is_some_and(|(code, _)| code == INVALID_REQUEST);
                    if is_refusal {
                        counts.0 += 1;
                    } else {
                        counts.1 += 1;
                    }
                    if is_refusal && refused.is_none() {
                        print_raw(&format!("round {round} {name}, refused"), &raw);
                        refused = Some(std::time::Instant::now());
                    }
                }
                Err(error) => println!("round {round} {name} failed: {error}"),
            }
        }
        if refused.is_some() {
            break;
        }
    }
    println!("{} answers were -32600, {} were not", counts.0, counts.1);
    let Some(refused_at) = refused else {
        println!("no answer was -32600 in {rounds} rounds");
        return;
    };

    let markets = program_accounts_params(LENDING_MARKET_SIZE, LENDING_MARKET_DISCRIMINATOR);
    for attempt in 0..rounds {
        thread::sleep(interval);
        match client.post(RpcRequest::GetProgramAccounts, markets.clone()) {
            Ok(raw) if rpc_error(&raw).is_none() => {
                println!(
                    "lending markets answered again {} ms after the refusal, attempt {attempt}",
                    refused_at.elapsed().as_millis()
                );
                return;
            }
            Ok(raw) => print_raw(
                &format!(
                    "lending markets attempt {attempt}, {} ms after the refusal",
                    refused_at.elapsed().as_millis()
                ),
                &raw,
            ),
            Err(error) => println!("lending markets attempt {attempt} failed: {error}"),
        }
    }
    println!("lending markets still refused after {rounds} attempts");
}

/// The `apiVersion` of an answer that has a context, or why the call gave none.
fn context_version<T: DeserializeOwned>(
    client: &Client,
    request: RpcRequest,
    params: serde_json::Value,
    context: impl Fn(T) -> Option<RpcResponseContext>,
) -> String {
    match client.call::<T>(request, params) {
        Ok(answer) => context(answer.result)
            .map_or_else(|| "no context".to_owned(), |context| api_version(&context)),
        Err(error) => format!("failed: {error}"),
    }
}

/// Calls getVersion and each method whose answer has a context, once per round, and counts the versions they report.
fn versions(client: &Client, rounds: u32, interval: Duration) {
    let clock = solana_sdk_ids::sysvar::clock::ID.to_string();
    let account_config = finalized_base64(Some(UiDataSliceConfig {
        offset: 0,
        length: 0,
    }));
    let commitment = serde_json::json!({ "commitment": "finalized" });
    // The lending markets, a few hundred accounts, addresses only.
    let markets = RpcProgramAccountsConfig {
        filters: Some(vec![RpcFilterType::DataSize(LENDING_MARKET_SIZE)]),
        account_config: account_config.clone(),
        with_context: Some(true),
        sort_results: None,
    };
    let params =
        |value: Result<serde_json::Value, serde_json::Error>| value.expect("the params serialize");
    let mut seen: BTreeMap<(&str, String), u32> = BTreeMap::new();
    for _ in 0..rounds {
        let reads = [
            ("getVersion", client.version()),
            (
                "getAccountInfo",
                context_version(
                    client,
                    RpcRequest::GetAccountInfo,
                    params(serde_json::to_value((&clock, &account_config))),
                    |response: Response<IgnoredAny>| Some(response.context),
                ),
            ),
            (
                "getMultipleAccounts",
                context_version(
                    client,
                    RpcRequest::GetMultipleAccounts,
                    params(serde_json::to_value(([&clock], &account_config))),
                    |response: Response<IgnoredAny>| Some(response.context),
                ),
            ),
            (
                "getBalance",
                context_version(
                    client,
                    RpcRequest::GetBalance,
                    params(serde_json::to_value((&clock, &commitment))),
                    |response: Response<IgnoredAny>| Some(response.context),
                ),
            ),
            (
                "getLatestBlockhash",
                context_version(
                    client,
                    RpcRequest::GetLatestBlockhash,
                    params(serde_json::to_value([&commitment])),
                    |response: Response<IgnoredAny>| Some(response.context),
                ),
            ),
            (
                "getSignatureStatuses",
                context_version(
                    client,
                    RpcRequest::GetSignatureStatuses,
                    params(serde_json::to_value([[CLOSING_TRANSACTION]])),
                    |response: Response<IgnoredAny>| Some(response.context),
                ),
            ),
            (
                "getProgramAccounts",
                context_version(
                    client,
                    RpcRequest::GetProgramAccounts,
                    params(serde_json::to_value((LENDING_PROGRAM, &markets))),
                    |response: OptionalContext<IgnoredAny>| match response {
                        OptionalContext::Context(response) => Some(response.context),
                        OptionalContext::NoContext(_) => None,
                    },
                ),
            ),
        ];
        for (method, version) in reads {
            *seen.entry((method, version)).or_default() += 1;
        }
        thread::sleep(interval);
    }

    for ((method, version), count) in &seen {
        println!("{method}: {version} in {count} of {rounds} answers");
    }
}
