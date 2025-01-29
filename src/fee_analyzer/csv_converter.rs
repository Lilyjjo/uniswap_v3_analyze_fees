use std::str::FromStr;

use alloy::primitives::{
    aliases::{I24, U24},
    Address, I256, U160, U256,
};
use chrono::{DateTime, Utc};
use eyre::{bail, Result};
use serde::{Deserialize, Serialize};
use tracing::info;

use super::simulation_events::{Event, SimulationEvent};
use crate::abi::{
    INonfungiblePositionManager::{Collect as CollectNpm, DecreaseLiquidity, IncreaseLiquidity},
    IUniswapV3Factory::PoolCreated,
    UniswapV3Pool::{Burn, Collect as CollectPool, Initialize, Mint, Swap},
};

pub struct CSVReaderConfig {
    pub initialize_events_path: String,
    pub swap_events_path: String,
    pub mint_events_path: String,
    pub burn_events_path: String,
    pub collect_pool_events_path: String,
    pub collect_npm_events_path: String,
    pub pool_created_events_path: String,
    pub increase_liquidity_events_path: String,
    pub decrease_liquidity_events_path: String,
}

pub(crate) async fn pool_events(config: CSVReaderConfig) -> Result<Vec<SimulationEvent>> {
    let initialize_events = read_initialize_events(&config.initialize_events_path)?;
    let initialize_simulation_events = convert_initialize_events(initialize_events)?;

    let swap_events = read_swap_events(&config.swap_events_path)?;
    let swap_simulation_events = convert_swap_events(swap_events)?;

    let mint_events = read_mint_events(&config.mint_events_path)?;
    let mint_simulation_events = convert_mint_events(mint_events)?;

    let burn_events = read_burn_events(&config.burn_events_path)?;
    let burn_simulation_events = convert_burn_events(burn_events)?;

    let collect_pool_events = read_collect_pool_events(&config.collect_pool_events_path)?;
    let collect_pool_simulation_events = convert_collect_pool_events(collect_pool_events)?;

    let collect_npm_events = read_collect_npm_events(&config.collect_npm_events_path)?;
    let collect_npm_simulation_events = convert_collect_npm_events(collect_npm_events)?;

    let pool_created_events = read_pool_created_events(&config.pool_created_events_path)?;
    let pool_created_simulation_events = convert_pool_created_events(pool_created_events)?;

    let increase_liquidity_events =
        read_increase_liquidity_events(&config.increase_liquidity_events_path)?;
    let increase_liquidity_simulation_events =
        convert_increase_liquidity_events(increase_liquidity_events)?;

    let decrease_liquidity_events =
        read_decrease_liquidity_events(&config.decrease_liquidity_events_path)?;
    let decrease_liquidity_simulation_events =
        convert_decrease_liquidity_events(decrease_liquidity_events)?;

    info!("Initialize events: {:?}", initialize_simulation_events);
    info!("Pool created events: {:?}", pool_created_simulation_events);
    info!("Mint events lengeth: {:?}", mint_simulation_events.len());
    info!("Burn events lengeth: {:?}", burn_simulation_events.len());
    info!(
        "Collect pool events lengeth: {:?}",
        collect_pool_simulation_events.len()
    );
    info!(
        "Collect npm events lengeth: {:?}",
        collect_npm_simulation_events.len()
    );
    info!(
        "Increase liquidity events lengeth: {:?}",
        increase_liquidity_simulation_events.len()
    );
    info!(
        "Decrease liquidity events lengeth: {:?}",
        decrease_liquidity_simulation_events.len()
    );

    if collect_npm_simulation_events.len() != collect_pool_simulation_events.len() {
        bail!("Collect npm events and collect pool events have different lengths, check if the same block range is used for all events or if positions are being created without use of the position manager");
    }

    let mut simulation_events = [
        initialize_simulation_events,
        pool_created_simulation_events,
        mint_simulation_events,
        burn_simulation_events,
        collect_pool_simulation_events,
        swap_simulation_events,
        collect_npm_simulation_events,
        increase_liquidity_simulation_events,
        decrease_liquidity_simulation_events,
    ]
    .concat();

    // sort events by block number and
    simulation_events.sort();

    Ok(simulation_events)
}

#[allow(non_snake_case, dead_code)]
#[derive(Debug, Deserialize, Serialize)]
struct CSVInitializeEvent {
    contract_address: String,
    evt_tx_hash: String,
    evt_tx_from: String,
    evt_tx_to: String,
    evt_index: u64,
    evt_block_time: DateTime<Utc>,
    evt_block_number: u64,
    sqrtPriceX96: String,
    tick: String,
}

fn read_initialize_events(path: &str) -> Result<Vec<CSVInitializeEvent>> {
    let file = std::fs::File::open(path)?;
    let mut rdr = csv::Reader::from_reader(file);
    let mut events = Vec::new();

    for result in rdr.deserialize() {
        let event: CSVInitializeEvent = result?;
        events.push(event);
    }

    Ok(events)
}

fn convert_initialize_events(events: Vec<CSVInitializeEvent>) -> Result<Vec<SimulationEvent>> {
    Ok(events
        .into_iter()
        .map(|event| SimulationEvent {
            pool_address: Address::from_str(&event.contract_address).unwrap(),
            block: event.evt_block_number,
            log_index: event.evt_index,
            from: Address::from_str(&event.evt_tx_from).unwrap(),
            event: Event::Initialize(Initialize {
                sqrtPriceX96: U160::from_str(&event.sqrtPriceX96).unwrap(),
                tick: I24::from_dec_str(&event.tick).unwrap(),
            }),
        })
        .collect())
}

#[allow(non_snake_case, dead_code)]
#[derive(Debug, Deserialize)]
struct CSVPoolCreatedEvent {
    contract_address: String,
    evt_tx_hash: String,
    evt_tx_from: String,
    evt_tx_to: String,
    evt_index: u64,
    evt_block_time: String,
    evt_block_number: u64,
    fee: String,
    pool: String,
    tickSpacing: String,
    token0: String,
    token1: String,
}

fn read_pool_created_events(path: &str) -> Result<Vec<CSVPoolCreatedEvent>> {
    let file = std::fs::File::open(path)?;
    let mut rdr = csv::Reader::from_reader(file);
    let mut events = Vec::new();

    for result in rdr.deserialize() {
        let event: CSVPoolCreatedEvent = result?;
        events.push(event);
    }

    Ok(events)
}

fn convert_pool_created_events(events: Vec<CSVPoolCreatedEvent>) -> Result<Vec<SimulationEvent>> {
    Ok(events
        .into_iter()
        .map(|event| SimulationEvent {
            pool_address: Address::from_str(&event.contract_address).unwrap(),
            block: event.evt_block_number,
            log_index: event.evt_index,
            from: Address::from_str(&event.evt_tx_from).unwrap(),
            event: Event::PoolCreated(PoolCreated {
                fee: U24::from_str(&event.fee).unwrap(),
                tickSpacing: I24::from_dec_str(&event.tickSpacing).unwrap(),
                pool: Address::from_str(&event.pool).unwrap(),
                token0: Address::from_str(&event.token0).unwrap(),
                token1: Address::from_str(&event.token1).unwrap(),
            }),
        })
        .collect())
}

#[allow(non_snake_case, dead_code)]
#[derive(Debug, Deserialize)]
struct CSVSwapEvent {
    contract_address: String,
    evt_tx_hash: String,
    evt_tx_from: String,
    evt_tx_to: String,
    evt_index: u64,
    evt_block_time: String,
    evt_block_number: u64,
    amount0: String,
    amount1: String,
    liquidity: String,
    recipient: String,
    sender: String,
    sqrtPriceX96: String,
    tick: String,
}

fn read_swap_events(path: &str) -> Result<Vec<CSVSwapEvent>> {
    let file = std::fs::File::open(path)?;
    let mut rdr = csv::Reader::from_reader(file);
    let mut events = Vec::new();

    for result in rdr.deserialize() {
        let event: CSVSwapEvent = result?;
        events.push(event);
    }

    Ok(events)
}

fn convert_swap_events(events: Vec<CSVSwapEvent>) -> Result<Vec<SimulationEvent>> {
    Ok(events
        .into_iter()
        .map(|event| SimulationEvent {
            pool_address: Address::from_str(&event.contract_address).unwrap(),
            block: event.evt_block_number,
            log_index: event.evt_index,
            from: Address::from_str(&event.evt_tx_from).unwrap(),
            event: Event::Swap(Swap {
                amount0: I256::from_str(&event.amount0).unwrap(),
                amount1: I256::from_str(&event.amount1).unwrap(),
                liquidity: u128::from_str(&event.liquidity).unwrap(),
                recipient: Address::from_str(&event.recipient).unwrap(),
                sender: Address::from_str(&event.sender).unwrap(),
                sqrtPriceX96: U160::from_str(&event.sqrtPriceX96).unwrap(),
                tick: I24::from_dec_str(&event.tick).unwrap(),
            }),
        })
        .collect())
}

#[allow(non_snake_case, dead_code)]
#[derive(Debug, Deserialize)]
struct CSVMintEvent {
    contract_address: String,
    evt_tx_hash: String,
    evt_tx_from: String,
    evt_tx_to: String,
    evt_index: u64,
    evt_block_time: String,
    evt_block_number: u64,
    amount: String,
    amount0: String,
    amount1: String,
    owner: String,
    sender: String,
    tickLower: String,
    tickUpper: String,
}

fn read_mint_events(path: &str) -> Result<Vec<CSVMintEvent>> {
    let file = std::fs::File::open(path)?;
    let mut rdr = csv::Reader::from_reader(file);
    let mut events = Vec::new();

    for result in rdr.deserialize() {
        let event: CSVMintEvent = result?;
        events.push(event);
    }

    Ok(events)
}

fn convert_mint_events(events: Vec<CSVMintEvent>) -> Result<Vec<SimulationEvent>> {
    Ok(events
        .into_iter()
        .map(|event| SimulationEvent {
            pool_address: Address::from_str(&event.contract_address).unwrap(),
            block: event.evt_block_number,
            log_index: event.evt_index,
            from: Address::from_str(&event.evt_tx_from).unwrap(),
            event: Event::Mint(Mint {
                amount: u128::from_str(&event.amount).unwrap(),
                amount0: U256::from_str(&event.amount0).unwrap(),
                amount1: U256::from_str(&event.amount1).unwrap(),
                owner: Address::from_str(&event.owner).unwrap(),
                sender: Address::from_str(&event.sender).unwrap(),
                tickLower: I24::from_dec_str(&event.tickLower).unwrap(),
                tickUpper: I24::from_dec_str(&event.tickUpper).unwrap(),
            }),
        })
        .collect())
}

#[allow(non_snake_case, dead_code)]
#[derive(Debug, Deserialize)]
struct CSVBurnEvent {
    contract_address: String,
    evt_tx_hash: String,
    evt_tx_from: String,
    evt_tx_to: String,
    evt_index: u64,
    evt_block_time: String,
    evt_block_number: u64,
    amount: String,
    amount0: String,
    amount1: String,
    owner: String,
    tickLower: String,
    tickUpper: String,
}

fn read_burn_events(path: &str) -> Result<Vec<CSVBurnEvent>> {
    let file = std::fs::File::open(path)?;
    let mut rdr = csv::Reader::from_reader(file);
    let mut events = Vec::new();

    for result in rdr.deserialize() {
        let event: CSVBurnEvent = result?;
        events.push(event);
    }

    Ok(events)
}

fn convert_burn_events(events: Vec<CSVBurnEvent>) -> Result<Vec<SimulationEvent>> {
    Ok(events
        .into_iter()
        .map(|event| SimulationEvent {
            pool_address: Address::from_str(&event.contract_address).unwrap(),
            block: event.evt_block_number,
            log_index: event.evt_index,
            from: Address::from_str(&event.evt_tx_from).unwrap(),
            event: Event::Burn(Burn {
                amount: u128::from_str(&event.amount).unwrap(),
                amount0: U256::from_str(&event.amount0).unwrap(),
                amount1: U256::from_str(&event.amount1).unwrap(),
                owner: Address::from_str(&event.owner).unwrap(),
                tickLower: I24::from_dec_str(&event.tickLower).unwrap(),
                tickUpper: I24::from_dec_str(&event.tickUpper).unwrap(),
            }),
        })
        .collect())
}

#[allow(non_snake_case, dead_code)]
#[derive(Debug, Deserialize)]
struct CSVCollectPoolEvent {
    contract_address: String,
    evt_tx_hash: String,
    evt_tx_from: String,
    evt_tx_to: String,
    evt_index: u64,
    evt_block_time: String,
    evt_block_number: u64,
    amount0: String,
    amount1: String,
    owner: String,
    recipient: String,
    tickLower: String,
    tickUpper: String,
}

fn read_collect_pool_events(path: &str) -> Result<Vec<CSVCollectPoolEvent>> {
    let file = std::fs::File::open(path)?;
    let mut rdr = csv::Reader::from_reader(file);
    let mut events = Vec::new();

    for result in rdr.deserialize() {
        let event: CSVCollectPoolEvent = result?;
        events.push(event);
    }

    Ok(events)
}

fn convert_collect_pool_events(events: Vec<CSVCollectPoolEvent>) -> Result<Vec<SimulationEvent>> {
    Ok(events
        .into_iter()
        .map(|event| SimulationEvent {
            pool_address: Address::from_str(&event.contract_address).unwrap(),
            block: event.evt_block_number,
            log_index: event.evt_index,
            from: Address::from_str(&event.evt_tx_from).unwrap(),
            event: Event::CollectPool(CollectPool {
                amount0: u128::from_str(&event.amount0).unwrap(),
                amount1: u128::from_str(&event.amount1).unwrap(),
                owner: Address::from_str(&event.owner).unwrap(),
                recipient: Address::from_str(&event.recipient).unwrap(),
                tickLower: I24::from_dec_str(&event.tickLower).unwrap(),
                tickUpper: I24::from_dec_str(&event.tickUpper).unwrap(),
            }),
        })
        .collect())
}

#[allow(non_snake_case, dead_code)]
#[derive(Debug, Deserialize)]
struct CSVIncreaseLiquidityEvent {
    contract_address: String,
    evt_tx_hash: String,
    evt_tx_from: String,
    evt_tx_to: String,
    evt_index: u64,
    evt_block_time: String,
    evt_block_number: u64,
    tokenId: String,
    liquidity: String,
    amount0: String,
    amount1: String,
}

fn read_increase_liquidity_events(path: &str) -> Result<Vec<CSVIncreaseLiquidityEvent>> {
    let file = std::fs::File::open(path)?;
    let mut rdr = csv::Reader::from_reader(file);
    let mut events = Vec::new();

    for result in rdr.deserialize() {
        let event: CSVIncreaseLiquidityEvent = result?;
        events.push(event);
    }

    Ok(events)
}

fn convert_increase_liquidity_events(
    events: Vec<CSVIncreaseLiquidityEvent>,
) -> Result<Vec<SimulationEvent>> {
    Ok(events
        .into_iter()
        .map(|event| SimulationEvent {
            pool_address: Address::from_str(&event.contract_address).unwrap(),
            block: event.evt_block_number,
            log_index: event.evt_index,
            from: Address::from_str(&event.evt_tx_from).unwrap(),
            event: Event::IncreaseLiquidity(IncreaseLiquidity {
                tokenId: U256::from_str(&event.tokenId).unwrap(),
                liquidity: u128::from_str(&event.liquidity).unwrap(),
                amount0: U256::from_str(&event.amount0).unwrap(),
                amount1: U256::from_str(&event.amount1).unwrap(),
            }),
        })
        .collect())
}

#[allow(non_snake_case, dead_code)]
#[derive(Debug, Deserialize)]
struct CSVDecreaseLiquidityEvent {
    contract_address: String,
    evt_tx_hash: String,
    evt_tx_from: String,
    evt_tx_to: String,
    evt_index: u64,
    evt_block_time: String,
    evt_block_number: u64,
    tokenId: String,
    liquidity: String,
    amount0: String,
    amount1: String,
}

fn read_decrease_liquidity_events(path: &str) -> Result<Vec<CSVDecreaseLiquidityEvent>> {
    let file = std::fs::File::open(path)?;
    let mut rdr = csv::Reader::from_reader(file);
    let mut events = Vec::new();

    for result in rdr.deserialize() {
        let event: CSVDecreaseLiquidityEvent = result?;
        events.push(event);
    }

    Ok(events)
}

fn convert_decrease_liquidity_events(
    events: Vec<CSVDecreaseLiquidityEvent>,
) -> Result<Vec<SimulationEvent>> {
    Ok(events
        .into_iter()
        .map(|event| SimulationEvent {
            pool_address: Address::from_str(&event.contract_address).unwrap(),
            block: event.evt_block_number,
            log_index: event.evt_index,
            from: Address::from_str(&event.evt_tx_from).unwrap(),
            event: Event::DecreaseLiquidity(DecreaseLiquidity {
                tokenId: U256::from_str(&event.tokenId).unwrap(),
                liquidity: u128::from_str(&event.liquidity).unwrap(),
                amount0: U256::from_str(&event.amount0).unwrap(),
                amount1: U256::from_str(&event.amount1).unwrap(),
            }),
        })
        .collect())
}

#[allow(non_snake_case, dead_code)]
#[derive(Debug, Deserialize)]
struct CSVCollectNpmEvent {
    contract_address: String,
    evt_tx_hash: String,
    evt_tx_from: String,
    evt_tx_to: String,
    evt_index: u64,
    evt_block_time: String,
    evt_block_number: u64,
    tokenId: String,
    recipient: String,
    amount0: String,
    amount1: String,
}

fn read_collect_npm_events(path: &str) -> Result<Vec<CSVCollectNpmEvent>> {
    let file = std::fs::File::open(path)?;
    let mut rdr = csv::Reader::from_reader(file);
    let mut events = Vec::new();

    for result in rdr.deserialize() {
        let event: CSVCollectNpmEvent = result?;
        events.push(event);
    }

    Ok(events)
}

fn convert_collect_npm_events(events: Vec<CSVCollectNpmEvent>) -> Result<Vec<SimulationEvent>> {
    Ok(events
        .into_iter()
        .map(|event| SimulationEvent {
            pool_address: Address::from_str(&event.contract_address).unwrap(),
            block: event.evt_block_number,
            log_index: event.evt_index,
            from: Address::from_str(&event.evt_tx_from).unwrap(),
            event: Event::CollectNpm(CollectNpm {
                tokenId: U256::from_str(&event.tokenId).unwrap(),
                recipient: Address::from_str(&event.recipient).unwrap(),
                amount0: U256::from_str(&event.amount0).unwrap(),
                amount1: U256::from_str(&event.amount1).unwrap(),
            }),
        })
        .collect())
}
