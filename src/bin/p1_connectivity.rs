//! Manual public-only P1 discovery, book, and reconnect probe.

use std::{
    collections::{BTreeMap, BTreeSet},
    error::Error,
    io,
    sync::Arc,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use nautilus_hyperliquid::{
    common::enums::HyperliquidEnvironment,
    http::{HyperliquidHttpClient, parse::HyperliquidInstrumentDef},
    websocket::{
        client::HyperliquidWebSocketClient, messages::NautilusWsMessage as HyperliquidWsMessage,
    },
};
use nautilus_lighter::{
    common::{
        consts::LIGHTER_VENUE,
        enums::{LighterEnvironment, LighterMarketStatus},
    },
    config::LighterDataClientConfig,
    http::client::LighterHttpClient,
    websocket::{LighterWebSocketClient, NautilusWsMessage as LighterWsMessage},
};
use nautilus_live::{SocketControl, SocketReconnectRegistry, SocketReconnectRequestOutcome};
use nautilus_model::{
    data::OrderBookDepth10,
    identifiers::{ClientId, InstrumentId as NautilusInstrumentId},
    instruments::{Instrument, InstrumentAny},
};
use nautilus_network::websocket::TransportBackend;
use riftbot::{
    domain::{
        ids::{InstrumentId, VenueId},
        market::{BookVersion, FeedConnectionState, FeedHealth},
        numeric::{DurationMillis, UnixNanos},
    },
    market::{
        book_store::{BookStore, FeedKey},
        nautilus_bridge::depth10_snapshot,
        normalizer::MarketNormalizer,
    },
};
use serde::Serialize;

const NAUTILUS_REVISION: &str = "e96a4ab8c8a5a7cae0ea6d37770d5ce2dee6db5c";
const SELECTED_SYMBOL: &str = "SNDK";
const LIGHTER_ENDPOINT: &str = "p1-public-data";
const INITIAL_SAMPLES: u64 = 3;
const PROBE_TIMEOUT: Duration = Duration::from_secs(45);

type ProbeResult<T> = Result<T, Box<dyn Error>>;

#[derive(Debug, Serialize)]
struct HyperliquidListing {
    raw_symbol: String,
    instrument_id: String,
    base: String,
}

#[derive(Debug, Serialize)]
struct LighterListing {
    raw_symbol: String,
    instrument_id: String,
}

#[derive(Debug, Serialize)]
struct DiscoveryEvidence {
    schema_version: u16,
    nautilus_revision: &'static str,
    hyperliquid_environment: &'static str,
    lighter_environment: &'static str,
    io: Vec<HyperliquidListing>,
    xyz: Vec<HyperliquidListing>,
    lighter: Vec<LighterListing>,
    io_lighter_common_bases: Vec<String>,
    xyz_lighter_common_bases: Vec<String>,
    io_xyz_lighter_common_bases: Vec<String>,
}

struct DiscoveryBootstrap {
    evidence: DiscoveryEvidence,
    hyperliquid_instruments: Vec<InstrumentAny>,
    lighter_http: LighterHttpClient,
    lighter_instruments: Vec<InstrumentAny>,
}

#[derive(Clone, Debug, Serialize)]
struct BookObservation {
    venue: String,
    instrument_id: String,
    samples_before_reconnect: u64,
    samples_after_reconnect: u64,
    first_exchange_ts: Option<u64>,
    last_exchange_ts: Option<u64>,
    first_receive_ts: Option<u64>,
    last_receive_ts: Option<u64>,
    last_best_bid: Option<String>,
    last_best_ask: Option<String>,
}

#[derive(Debug, Serialize)]
struct ReconnectEvidence {
    hyperliquid_request_accepted: bool,
    hyperliquid_reconnected_event: bool,
    lighter_request_outcome: String,
    lighter_reconnected_event: bool,
    recovery_book_observed_on_all_feeds: bool,
}

#[derive(Debug, Serialize)]
struct ValidationEvidence {
    schema_version: u16,
    nautilus_revision: &'static str,
    selected_symbol: &'static str,
    selected_pair_venues: [&'static str; 2],
    discovered_counts: BTreeMap<&'static str, usize>,
    observations: Vec<BookObservation>,
    final_health: Vec<FeedHealth>,
    reconnect: ReconnectEvidence,
}

struct Targets {
    io: NautilusInstrumentId,
    xyz: NautilusInstrumentId,
    lighter: NautilusInstrumentId,
}

struct FeedProbe {
    store: BookStore,
    observations: BTreeMap<String, BookObservation>,
}

impl FeedProbe {
    fn new(targets: &Targets) -> ProbeResult<Self> {
        let mut observations = BTreeMap::new();
        for (label, venue, instrument_id) in [
            ("entropy_io", "entropy", targets.io),
            ("lighter", "lighter", targets.lighter),
            ("trade_xyz", "trade_xyz", targets.xyz),
        ] {
            observations.insert(
                label.to_owned(),
                BookObservation {
                    venue: venue.to_owned(),
                    instrument_id: instrument_id.to_string(),
                    samples_before_reconnect: 0,
                    samples_after_reconnect: 0,
                    first_exchange_ts: None,
                    last_exchange_ts: None,
                    first_receive_ts: None,
                    last_receive_ts: None,
                    last_best_bid: None,
                    last_best_ask: None,
                },
            );
        }
        Ok(Self {
            store: BookStore::new(DurationMillis(5_000))?,
            observations,
        })
    }

    fn transition(
        &mut self,
        label: &str,
        state: FeedConnectionState,
        transition_ts: UnixNanos,
    ) -> ProbeResult<()> {
        self.store
            .set_connection_state(self.key(label)?, state, transition_ts)?;
        Ok(())
    }

    fn transition_all(
        &mut self,
        state: FeedConnectionState,
        transition_ts: UnixNanos,
    ) -> ProbeResult<()> {
        for label in ["entropy_io", "lighter", "trade_xyz"] {
            self.transition(label, state, transition_ts)?;
        }
        Ok(())
    }

    fn ingest(
        &mut self,
        label: &str,
        venue: &str,
        depth: &OrderBookDepth10,
        recovery: bool,
    ) -> ProbeResult<()> {
        let observation = self
            .observations
            .get_mut(label)
            .ok_or_else(|| io::Error::other(format!("unknown feed label {label}")))?;
        let version = BookVersion(
            observation.samples_before_reconnect + observation.samples_after_reconnect + 1,
        );
        let raw = depth10_snapshot(VenueId::try_from(venue)?, depth, version)?;
        let book = MarketNormalizer::normalize(raw)?;
        let exchange_ts = book.exchange_ts.0;
        let receive_ts = book.receive_ts.0;
        let best_bid = book.bids[0].price.value().to_string();
        let best_ask = book.asks[0].price.value().to_string();
        self.store.update(book)?;

        if recovery {
            observation.samples_after_reconnect += 1;
        } else {
            observation.samples_before_reconnect += 1;
        }
        observation.first_exchange_ts.get_or_insert(exchange_ts);
        observation.first_receive_ts.get_or_insert(receive_ts);
        observation.last_exchange_ts = Some(exchange_ts);
        observation.last_receive_ts = Some(receive_ts);
        observation.last_best_bid = Some(best_bid);
        observation.last_best_ask = Some(best_ask);
        Ok(())
    }

    fn initial_complete(&self) -> bool {
        self.observations
            .values()
            .all(|observation| observation.samples_before_reconnect >= INITIAL_SAMPLES)
    }

    fn recovery_complete(&self) -> bool {
        self.observations
            .values()
            .all(|observation| observation.samples_after_reconnect >= 1)
    }

    fn key(&self, label: &str) -> ProbeResult<FeedKey> {
        let observation = self
            .observations
            .get(label)
            .ok_or_else(|| io::Error::other(format!("unknown feed label {label}")))?;
        Ok(FeedKey::new(
            VenueId::try_from(observation.venue.as_str())?,
            InstrumentId::try_from(observation.instrument_id.as_str())?,
        ))
    }

    fn final_health(&self, now: UnixNanos) -> ProbeResult<Vec<FeedHealth>> {
        self.observations
            .keys()
            .map(|label| {
                let key = self.key(label)?;
                self.store.health(&key, now).ok_or_else(|| {
                    io::Error::other(format!("missing final health for {label}")).into()
                })
            })
            .collect()
    }
}

#[tokio::main]
async fn main() -> ProbeResult<()> {
    let command = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "discover".to_owned());
    let bootstrap = discover().await?;
    match command.as_str() {
        "discover" => println!("{}", serde_json::to_string_pretty(&bootstrap.evidence)?),
        "validate" => validate(bootstrap).await?,
        _ => {
            return Err(io::Error::other(format!(
                "unsupported command '{command}'; expected 'discover' or 'validate'"
            ))
            .into());
        }
    }
    Ok(())
}

async fn discover() -> ProbeResult<DiscoveryBootstrap> {
    let hyperliquid = HyperliquidHttpClient::new(HyperliquidEnvironment::Mainnet, 60, None)?;
    let hyperliquid_defs = hyperliquid.request_instrument_defs().await?;
    let hyperliquid_instruments = hyperliquid.convert_defs(hyperliquid_defs.clone());

    let lighter_http = LighterHttpClient::new(LighterEnvironment::Mainnet, None, 60, None)?;
    let lighter_with_status = lighter_http.request_instruments_with_status().await?;
    let lighter_instruments = lighter_with_status
        .into_iter()
        .filter(|(instrument, status)| {
            *status == LighterMarketStatus::Active
                && instrument.id().to_string().ends_with("-PERP.LIGHTER")
        })
        .map(|(instrument, _)| instrument)
        .collect::<Vec<_>>();

    let io = hip3_listings(&hyperliquid_defs, &hyperliquid_instruments, "io:");
    let xyz = hip3_listings(&hyperliquid_defs, &hyperliquid_instruments, "xyz:");
    if io.is_empty() {
        return Err(io::Error::other(
            "official adapter did not discover active io HIP-3 instruments",
        )
        .into());
    }
    if xyz.is_empty() {
        return Err(io::Error::other(
            "official adapter did not discover active xyz HIP-3 instruments",
        )
        .into());
    }
    if lighter_instruments.is_empty() {
        return Err(io::Error::other(
            "official adapter did not discover active Lighter Mainnet perpetuals",
        )
        .into());
    }

    let lighter = lighter_instruments
        .iter()
        .map(|instrument| LighterListing {
            raw_symbol: instrument.raw_symbol().to_string(),
            instrument_id: instrument.id().to_string(),
        })
        .collect::<Vec<_>>();
    let lighter_bases = base_set(lighter.iter().map(|listing| listing.raw_symbol.as_str()));
    let io_bases = base_set(io.iter().map(|listing| listing.base.as_str()));
    let xyz_bases = base_set(xyz.iter().map(|listing| listing.base.as_str()));

    let evidence = DiscoveryEvidence {
        schema_version: 1,
        nautilus_revision: NAUTILUS_REVISION,
        hyperliquid_environment: "mainnet",
        lighter_environment: "mainnet",
        io_lighter_common_bases: intersection(&[&io_bases, &lighter_bases]),
        xyz_lighter_common_bases: intersection(&[&xyz_bases, &lighter_bases]),
        io_xyz_lighter_common_bases: intersection(&[&io_bases, &xyz_bases, &lighter_bases]),
        io,
        xyz,
        lighter,
    };
    Ok(DiscoveryBootstrap {
        evidence,
        hyperliquid_instruments,
        lighter_http,
        lighter_instruments,
    })
}

async fn validate(bootstrap: DiscoveryBootstrap) -> ProbeResult<()> {
    if !bootstrap
        .evidence
        .io_xyz_lighter_common_bases
        .iter()
        .any(|candidate| candidate == SELECTED_SYMBOL)
    {
        return Err(io::Error::other(format!(
            "selected symbol {SELECTED_SYMBOL} is not common to io, xyz, and Lighter"
        ))
        .into());
    }

    let io_instrument = find_instrument(&bootstrap.hyperliquid_instruments, "io:SNDK")?;
    let xyz_instrument = find_instrument(&bootstrap.hyperliquid_instruments, "xyz:SNDK")?;
    let lighter_instrument = find_instrument(&bootstrap.lighter_instruments, SELECTED_SYMBOL)?;
    let targets = Targets {
        io: io_instrument.id(),
        xyz: xyz_instrument.id(),
        lighter: lighter_instrument.id(),
    };

    let mut hyperliquid_ws = HyperliquidWebSocketClient::new(
        None,
        HyperliquidEnvironment::Mainnet,
        None,
        TransportBackend::default(),
        None,
    );
    hyperliquid_ws.cache_instruments(bootstrap.hyperliquid_instruments);

    let market_registry = bootstrap.lighter_http.market_registry();
    let lighter_cache = bootstrap
        .lighter_instruments
        .iter()
        .filter_map(|instrument| {
            market_registry
                .market_index(&instrument.id())
                .map(|index| (index, instrument.clone()))
        })
        .collect::<Vec<_>>();
    let reconnect_registry = SocketReconnectRegistry::default();
    let lighter_client_id = ClientId::from("P1-LIGHTER");
    let lighter_control = SocketControl::with_registry(
        lighter_client_id,
        Some(*LIGHTER_VENUE),
        LIGHTER_ENDPOINT,
        &reconnect_registry,
    );
    let lighter_data_config = LighterDataClientConfig::default();
    let mut lighter_ws = LighterWebSocketClient::new(
        Some(lighter_data_config.ws_url()),
        LighterEnvironment::Mainnet,
        Arc::clone(&market_registry),
        TransportBackend::default(),
        30,
        None,
    )
    .with_socket_control(lighter_control);
    lighter_ws.cache_instruments(lighter_cache);

    let mut probe = FeedProbe::new(&targets)?;
    probe.transition_all(FeedConnectionState::Connecting, now_nanos()?)?;
    hyperliquid_ws.connect().await.map_err(|error| {
        io::Error::other(format!(
            "official Hyperliquid WebSocket connect failed: {error}"
        ))
    })?;
    lighter_ws.connect().await.map_err(|error| {
        io::Error::other(format!(
            "official Lighter WebSocket connect failed: {error}"
        ))
    })?;
    probe.transition_all(FeedConnectionState::Connected, now_nanos()?)?;
    hyperliquid_ws.subscribe_book_depth10(targets.io).await?;
    hyperliquid_ws.subscribe_book_depth10(targets.xyz).await?;
    lighter_ws.subscribe_book_depth10(targets.lighter).await?;

    collect_initial(&mut hyperliquid_ws, &mut lighter_ws, &targets, &mut probe).await?;

    probe.transition_all(FeedConnectionState::Disconnected, now_nanos()?)?;
    probe.transition_all(FeedConnectionState::Reconnecting, now_nanos()?)?;
    let hyperliquid_request_accepted = hyperliquid_ws.request_reconnect();
    if !hyperliquid_request_accepted {
        return Err(io::Error::other("Hyperliquid reconnect request was rejected").into());
    }
    let lighter_handle = reconnect_registry
        .handle(lighter_client_id, LIGHTER_ENDPOINT.into())
        .ok_or_else(|| io::Error::other("Lighter reconnect handle was not registered"))?;
    let lighter_outcome = lighter_handle.request_reconnect();
    if lighter_outcome != SocketReconnectRequestOutcome::Accepted {
        return Err(io::Error::other(format!(
            "Lighter reconnect request was not accepted: {lighter_outcome:?}"
        ))
        .into());
    }

    let (hyperliquid_reconnected, lighter_reconnected) =
        collect_recovery(&mut hyperliquid_ws, &mut lighter_ws, &targets, &mut probe).await?;
    let final_health = probe.final_health(now_nanos()?)?;
    if !final_health.iter().all(FeedHealth::is_healthy) {
        return Err(io::Error::other("one or more feeds were not healthy after recovery").into());
    }

    hyperliquid_ws.disconnect().await?;
    lighter_ws.disconnect().await?;

    let mut discovered_counts = BTreeMap::new();
    discovered_counts.insert("entropy_io_active", bootstrap.evidence.io.len());
    discovered_counts.insert("trade_xyz_active", bootstrap.evidence.xyz.len());
    discovered_counts.insert(
        "lighter_active_perpetuals",
        bootstrap.evidence.lighter.len(),
    );
    let recovery_book_observed_on_all_feeds = probe.recovery_complete();
    let evidence = ValidationEvidence {
        schema_version: 1,
        nautilus_revision: NAUTILUS_REVISION,
        selected_symbol: SELECTED_SYMBOL,
        selected_pair_venues: ["entropy", "lighter"],
        discovered_counts,
        observations: probe.observations.into_values().collect(),
        final_health,
        reconnect: ReconnectEvidence {
            hyperliquid_request_accepted,
            hyperliquid_reconnected_event: hyperliquid_reconnected,
            lighter_request_outcome: format!("{lighter_outcome:?}"),
            lighter_reconnected_event: lighter_reconnected,
            recovery_book_observed_on_all_feeds,
        },
    };
    println!("{}", serde_json::to_string_pretty(&evidence)?);
    Ok(())
}

async fn collect_initial(
    hyperliquid: &mut HyperliquidWebSocketClient,
    lighter: &mut LighterWebSocketClient,
    targets: &Targets,
    probe: &mut FeedProbe,
) -> ProbeResult<()> {
    tokio::time::timeout(PROBE_TIMEOUT, async {
        while !probe.initial_complete() {
            tokio::select! {
                event = hyperliquid.next_event() => {
                    process_hyperliquid_depth(event, targets, probe, false)?;
                },
                event = lighter.next_event() => {
                    process_lighter_depth(event, targets, probe, false)?;
                },
            }
        }
        ProbeResult::Ok(())
    })
    .await
    .map_err(|_| io::Error::other("timed out collecting initial books"))??;
    Ok(())
}

async fn collect_recovery(
    hyperliquid: &mut HyperliquidWebSocketClient,
    lighter: &mut LighterWebSocketClient,
    targets: &Targets,
    probe: &mut FeedProbe,
) -> ProbeResult<(bool, bool)> {
    tokio::time::timeout(PROBE_TIMEOUT, async {
        let mut hyperliquid_reconnected = false;
        let mut lighter_reconnected = false;
        while !probe.recovery_complete() {
            tokio::select! {
                event = hyperliquid.next_event() => match event {
                    Some(HyperliquidWsMessage::Reconnected) => {
                        hyperliquid_reconnected = true;
                        let now = now_nanos()?;
                        probe.transition("entropy_io", FeedConnectionState::Connected, now)?;
                        probe.transition("trade_xyz", FeedConnectionState::Connected, now)?;
                    }
                    other if hyperliquid_reconnected => {
                        process_hyperliquid_depth(other, targets, probe, true)?;
                    }
                    Some(_) => {}
                    None => return Err(io::Error::other("Hyperliquid stream closed during recovery").into()),
                },
                event = lighter.next_event() => match event {
                    Some(LighterWsMessage::Reconnected { .. }) => {
                        lighter_reconnected = true;
                        probe.transition("lighter", FeedConnectionState::Connected, now_nanos()?)?;
                    }
                    other if lighter_reconnected => {
                        process_lighter_depth(other, targets, probe, true)?;
                    }
                    Some(_) => {}
                    None => return Err(io::Error::other("Lighter stream closed during recovery").into()),
                },
            }
        }
        Ok((hyperliquid_reconnected, lighter_reconnected))
    })
    .await
    .map_err(|_| io::Error::other("timed out collecting reconnect recovery books"))?
}

fn process_hyperliquid_depth(
    event: Option<HyperliquidWsMessage>,
    targets: &Targets,
    probe: &mut FeedProbe,
    recovery: bool,
) -> ProbeResult<()> {
    match event {
        Some(HyperliquidWsMessage::Depth10(depth)) if depth.instrument_id == targets.io => {
            probe.ingest("entropy_io", "entropy", &depth, recovery)?;
        }
        Some(HyperliquidWsMessage::Depth10(depth)) if depth.instrument_id == targets.xyz => {
            probe.ingest("trade_xyz", "trade_xyz", &depth, recovery)?;
        }
        Some(HyperliquidWsMessage::Error(message)) => {
            return Err(io::Error::other(format!("Hyperliquid stream error: {message}")).into());
        }
        Some(_) => {}
        None => return Err(io::Error::other("Hyperliquid stream closed").into()),
    }
    Ok(())
}

fn process_lighter_depth(
    event: Option<LighterWsMessage>,
    targets: &Targets,
    probe: &mut FeedProbe,
    recovery: bool,
) -> ProbeResult<()> {
    match event {
        Some(LighterWsMessage::Depth10(depth)) if depth.instrument_id == targets.lighter => {
            probe.ingest("lighter", "lighter", &depth, recovery)?;
        }
        Some(_) => {}
        None => return Err(io::Error::other("Lighter stream closed").into()),
    }
    Ok(())
}

fn hip3_listings(
    defs: &[HyperliquidInstrumentDef],
    instruments: &[InstrumentAny],
    prefix: &str,
) -> Vec<HyperliquidListing> {
    defs.iter()
        .filter(|definition| {
            definition.is_hip3
                && definition.active
                && definition.raw_symbol.as_str().starts_with(prefix)
        })
        .filter_map(|definition| {
            instruments
                .iter()
                .find(|instrument| {
                    instrument.raw_symbol().as_str() == definition.raw_symbol.as_str()
                })
                .map(|instrument| HyperliquidListing {
                    raw_symbol: definition.raw_symbol.to_string(),
                    instrument_id: instrument.id().to_string(),
                    base: definition
                        .raw_symbol
                        .as_str()
                        .strip_prefix(prefix)
                        .expect("prefix was filtered")
                        .to_owned(),
                })
        })
        .collect()
}

fn find_instrument<'a>(
    instruments: &'a [InstrumentAny],
    raw_symbol: &str,
) -> ProbeResult<&'a InstrumentAny> {
    instruments
        .iter()
        .find(|instrument| instrument.raw_symbol().as_str() == raw_symbol)
        .ok_or_else(|| {
            io::Error::other(format!("instrument {raw_symbol} was not discovered")).into()
        })
}

fn base_set<'a>(values: impl Iterator<Item = &'a str>) -> BTreeSet<String> {
    values.map(str::to_ascii_uppercase).collect()
}

fn intersection(sets: &[&BTreeSet<String>]) -> Vec<String> {
    let Some(first) = sets.first() else {
        return Vec::new();
    };
    first
        .iter()
        .filter(|candidate| sets[1..].iter().all(|set| set.contains(*candidate)))
        .cloned()
        .collect()
}

fn now_nanos() -> ProbeResult<UnixNanos> {
    let nanos = SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos();
    Ok(UnixNanos(u64::try_from(nanos)?))
}
