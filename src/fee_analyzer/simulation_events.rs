use std::cmp::Ordering;

use alloy::primitives::{Address, TxHash, U256};
use eyre::Result;

use crate::abi::{
    INonfungiblePositionManager::{Collect as CollectNpm, DecreaseLiquidity, IncreaseLiquidity},
    IUniswapV3Factory::PoolCreated,
    UniswapV3Pool::{Burn, Collect as CollectPool, Initialize, Mint, Swap},
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IncreaseLiquidityWithParams {
    pub amount_0_desired: U256,
    pub amount_1_desired: U256,
    pub event: IncreaseLiquidity,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Event {
    PoolCreated(PoolCreated),
    Mint(Mint),
    Burn(Burn),
    Swap(Swap),
    CollectPool(CollectPool),
    CollectNpm(CollectNpm),
    IncreaseLiquidity(IncreaseLiquidityWithParams),
    DecreaseLiquidity(DecreaseLiquidity),
    Initialize(Initialize),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum EventType {
    PoolCreated,
    Mint,
    Burn,
    Swap,
    CollectPool,
    CollectNpm,
    IncreaseLiquidity,
    DecreaseLiquidity,
    Initialize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SimulationEvent {
    pub block: u64,
    pub tx_hash: TxHash,
    pub log_index: u64,
    pub pool_address: Address,
    pub from: Address,
    pub event: Event,
}

impl Ord for SimulationEvent {
    fn cmp(&self, other: &Self) -> Ordering {
        let block = self.block.cmp(&other.block);

        if block != Ordering::Equal {
            return block;
        }

        self.log_index.cmp(&other.log_index)
    }
}

impl PartialOrd for SimulationEvent {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Event {
    pub(crate) fn event_type(&self) -> EventType {
        match self {
            Event::PoolCreated(_) => EventType::PoolCreated,
            Event::Mint(_) => EventType::Mint,
            Event::Burn(_) => EventType::Burn,
            Event::Swap(_) => EventType::Swap,
            Event::CollectPool(_) => EventType::CollectPool,
            Event::CollectNpm(_) => EventType::CollectNpm,
            Event::IncreaseLiquidity(_) => EventType::IncreaseLiquidity,
            Event::DecreaseLiquidity(_) => EventType::DecreaseLiquidity,
            Event::Initialize(_) => EventType::Initialize,
        }
    }
}

pub(crate) fn find_first_event(
    events: &Vec<SimulationEvent>,
    event_type: EventType,
) -> Result<SimulationEvent> {
    let event = events
        .iter()
        .find(|event| event.event.event_type() == event_type)
        .ok_or_else(|| eyre::eyre!("Event not found"))?;

    Ok(event.clone())
}

impl TryFrom<SimulationEvent> for PoolCreated {
    type Error = eyre::Report;

    fn try_from(event: SimulationEvent) -> eyre::Result<Self> {
        match event.event {
            Event::PoolCreated(e) => Ok(e),
            _ => Err(eyre::eyre!("Event is not PoolCreated")),
        }
    }
}

impl TryFrom<SimulationEvent> for Mint {
    type Error = eyre::Report;

    fn try_from(event: SimulationEvent) -> eyre::Result<Self> {
        match event.event {
            Event::Mint(e) => Ok(e),
            _ => Err(eyre::eyre!("Event is not Mint")),
        }
    }
}

impl TryFrom<SimulationEvent> for Burn {
    type Error = eyre::Report;

    fn try_from(event: SimulationEvent) -> eyre::Result<Self> {
        match event.event {
            Event::Burn(e) => Ok(e),
            _ => Err(eyre::eyre!("Event is not Burn")),
        }
    }
}

impl TryFrom<SimulationEvent> for Swap {
    type Error = eyre::Report;

    fn try_from(event: SimulationEvent) -> eyre::Result<Self> {
        match event.event {
            Event::Swap(e) => Ok(e),
            _ => Err(eyre::eyre!("Event is not Swap")),
        }
    }
}

impl TryFrom<SimulationEvent> for CollectPool {
    type Error = eyre::Report;

    fn try_from(event: SimulationEvent) -> eyre::Result<Self> {
        match event.event {
            Event::CollectPool(e) => Ok(e),
            _ => Err(eyre::eyre!("Event is not CollectPool")),
        }
    }
}

impl TryFrom<SimulationEvent> for Initialize {
    type Error = eyre::Report;

    fn try_from(event: SimulationEvent) -> eyre::Result<Self> {
        match event.event {
            Event::Initialize(e) => Ok(e),
            _ => Err(eyre::eyre!("Event is not Initialize")),
        }
    }
}

impl TryFrom<SimulationEvent> for CollectNpm {
    type Error = eyre::Report;

    fn try_from(event: SimulationEvent) -> eyre::Result<Self> {
        match event.event {
            Event::CollectNpm(e) => Ok(e),
            _ => Err(eyre::eyre!("Event is not CollectNpm")),
        }
    }
}

impl TryFrom<SimulationEvent> for IncreaseLiquidityWithParams {
    type Error = eyre::Report;

    fn try_from(event: SimulationEvent) -> eyre::Result<Self> {
        match event.event {
            Event::IncreaseLiquidity(e) => Ok(e),
            _ => Err(eyre::eyre!("Event is not IncreaseLiquidity")),
        }
    }
}

impl TryFrom<SimulationEvent> for DecreaseLiquidity {
    type Error = eyre::Report;

    fn try_from(event: SimulationEvent) -> eyre::Result<Self> {
        match event.event {
            Event::DecreaseLiquidity(e) => Ok(e),
            _ => Err(eyre::eyre!("Event is not DecreaseLiquidity")),
        }
    }
}
