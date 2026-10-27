//! Reads the Clock sysvar at finalized commitment, alternating getMultipleAccounts and getAccountInfo, and reports every
//! answer whose Clock slot (the first 8 bytes of the account's data) differs from the answer's `context.slot`.
//!
//! The Clock sysvar's `slot` field is the slot of the bank its data was read from, so an answer whose Clock slot is
//! greater than `context.slot` returned account data from a later bank than the slot it reports.
//!
//! Usage: `RPC_URL=<url> cargo run --release -- [reads] [interval-ms]`, by default 300 reads 300 ms apart.

use std::{collections::BTreeMap, env, thread, time::Duration};

use base64::{engine::general_purpose::STANDARD, Engine};
use serde::{Deserialize, Serialize};

const CLOCK: &str = "SysvarC1ock11111111111111111111111111111111";

#[derive(Serialize)]
struct Request<'a, P> {
    jsonrpc: &'static str,
    id: u64,
    method: &'a str,
    params: P,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct AccountConfig {
    encoding: &'static str,
    commitment: &'static str,
}

#[derive(Deserialize)]
struct Response<T> {
    result: Option<T>,
    error: Option<RpcError>,
}

#[derive(Deserialize, Debug)]
struct RpcError {
    code: i64,
    message: String,
}

#[derive(Deserialize)]
struct WithContext<T> {
    context: Context,
    value: T,
}

#[derive(Deserialize)]
struct Context {
    slot: u64,
}

#[derive(Deserialize)]
struct Account {
    /// The account data and its encoding, `["<base64>", "base64"]`.
    data: (String, String),
}

#[derive(Deserialize)]
#[serde(rename_all = "kebab-case")]
struct Version {
    solana_core: String,
    feature_set: Option<u64>,
}

/// One read of the Clock: the answer's `context.slot` and the Clock's own slot.
struct Read {
    context_slot: u64,
    clock_slot: u64,
}

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Method {
    GetMultipleAccounts,
    GetAccountInfo,
}

impl Method {
    fn name(self) -> &'static str {
        match self {
            Self::GetMultipleAccounts => "getMultipleAccounts",
            Self::GetAccountInfo => "getAccountInfo",
        }
    }
}

struct Client {
    http: reqwest::blocking::Client,
    url: String,
}

impl Client {
    fn call<P: Serialize, T: for<'de> Deserialize<'de>>(
        &self,
        method: &str,
        params: P,
    ) -> Result<T, String> {
        let request = Request {
            jsonrpc: "2.0",
            id: 1,
            method,
            params,
        };
        let response: Response<T> = self
            .http
            .post(&self.url)
            .json(&request)
            .send()
            .and_then(reqwest::blocking::Response::json)
            .map_err(|e| format!("{method}: {e}"))?;
        match response {
            Response {
                error: Some(RpcError { code, message }),
                ..
            } => Err(format!("{method}: {code} {message}")),
            Response {
                result: Some(result),
                error: None,
            } => Ok(result),
            Response {
                result: None,
                error: None,
            } => Err(format!("{method}: neither a result nor an error")),
        }
    }

    fn read_clock(&self, method: Method) -> Result<Read, String> {
        let config = AccountConfig {
            encoding: "base64",
            commitment: "finalized",
        };
        let (context_slot, account) = match method {
            Method::GetMultipleAccounts => {
                let answer: WithContext<Vec<Option<Account>>> =
                    self.call(method.name(), ([CLOCK], config))?;
                let account = answer.value.into_iter().next().flatten();
                (answer.context.slot, account)
            }
            Method::GetAccountInfo => {
                let answer: WithContext<Option<Account>> =
                    self.call(method.name(), (CLOCK, config))?;
                (answer.context.slot, answer.value)
            }
        };
        let account = account.ok_or_else(|| format!("{}: no Clock account", method.name()))?;
        let data = STANDARD
            .decode(&account.data.0)
            .map_err(|e| format!("{}: the Clock's data is not base64: {e}", method.name()))?;
        let slot_bytes: [u8; 8] = data
            .get(..8)
            .and_then(|bytes| bytes.try_into().ok())
            .ok_or_else(|| {
                format!(
                    "{}: the Clock's data is {} bytes",
                    method.name(),
                    data.len()
                )
            })?;
        Ok(Read {
            context_slot,
            clock_slot: u64::from_le_bytes(slot_bytes),
        })
    }
}

fn main() {
    let url = env::var("RPC_URL").expect("RPC_URL is the RPC endpoint to read");
    let mut args = env::args().skip(1);
    let reads: u32 = args
        .next()
        .map_or(300, |reads| reads.parse().expect("reads is a number"));
    let interval_ms: u64 = args.next().map_or(300, |interval| {
        interval.parse().expect("interval-ms is a number")
    });
    let client = Client {
        http: reqwest::blocking::Client::builder()
            .timeout(Duration::from_secs(30))
            .build()
            .expect("an HTTP client builds"),
        url,
    };

    match client.call::<_, Version>("getVersion", ()) {
        Ok(Version {
            solana_core,
            feature_set,
        }) => println!("solana-core {solana_core}, feature-set {feature_set:?}"),
        Err(error) => println!("getVersion failed: {error}"),
    }

    // Clock slot minus context.slot, and how many answers had each, by method.
    let mut deltas: BTreeMap<(Method, i64), u32> = BTreeMap::new();
    let mut failures: BTreeMap<Method, u32> = BTreeMap::new();
    for i in 0..reads {
        let method = if i % 2 == 0 {
            Method::GetMultipleAccounts
        } else {
            Method::GetAccountInfo
        };
        match client.read_clock(method) {
            Ok(Read {
                context_slot,
                clock_slot,
            }) => {
                let delta = i64::try_from(clock_slot).unwrap_or(i64::MAX)
                    - i64::try_from(context_slot).unwrap_or(i64::MAX);
                if delta != 0 {
                    println!(
                        "read {i} {}: context.slot {context_slot}, Clock slot {clock_slot} ({delta:+})",
                        method.name()
                    );
                }
                *deltas.entry((method, delta)).or_default() += 1;
            }
            Err(error) => {
                println!("read {i} failed: {error}");
                *failures.entry(method).or_default() += 1;
            }
        }
        thread::sleep(Duration::from_millis(interval_ms));
    }

    for method in [Method::GetMultipleAccounts, Method::GetAccountInfo] {
        let counts: Vec<String> = deltas
            .iter()
            .filter(|((counted, _), _)| *counted == method)
            .map(|((_, delta), count)| format!("{delta:+}: {count}"))
            .collect();
        let answered: u32 = deltas
            .iter()
            .filter(|((counted, _), _)| *counted == method)
            .map(|(_, count)| count)
            .sum();
        println!(
            "{}: {answered} answers; Clock slot minus context.slot: {}; failed {}",
            method.name(),
            counts.join(", "),
            failures.get(&method).copied().unwrap_or(0)
        );
    }
}
