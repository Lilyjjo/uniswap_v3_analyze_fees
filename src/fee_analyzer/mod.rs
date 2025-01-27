use std::{str::FromStr, sync::Arc};

use alloy::primitives::{Address, U256};
use contract_interactions::{
    anvil_connection, approve_token, deploy_and_initialize_pool, initialize_simulation_account,
    pool_mint, pool_swap, AnvilHttpProvider, HttpClient,
};
use csv_converter::{pool_events, CSVReaderConfig};
use eyre::Result;
use simulation_events::{find_first_event, EventType};

use crate::abi::{
    INonfungiblePositionManager, ISwapRouter,
    IUniswapV3Factory::{self, PoolCreated},
    UniswapV3Pool::{Initialize, Mint, Swap},
    Weth,
};

mod contract_interactions;
pub mod csv_converter;
mod simulation_events;

pub async fn analyze_pool(
    http_url: String,
    fork_block: u64,
    uniswap_v3_factory_address: Address,
    uniswap_v3_position_manager_address: Address,
    uniswap_v3_swap_router_address: Address,
    weth_address: Address,
    config: CSVReaderConfig,
) -> Result<()> {
    // get first few pool events for testing
    let pool_events = pool_events(config).await?;
    let create_event: PoolCreated =
        find_first_event(&pool_events, EventType::PoolCreated)?.try_into()?;
    let init_event: Initialize =
        find_first_event(&pool_events, EventType::Initialize)?.try_into()?;
    let mint_event: Mint = find_first_event(&pool_events, EventType::Mint)?.try_into()?;
    let swap_event: Swap = find_first_event(&pool_events, EventType::Swap)?.try_into()?;

    let (_, anvil_provider) = anvil_connection(http_url, fork_block).await?;

    // create weth instance
    let weth = Arc::new(Weth::new(weth_address, anvil_provider.clone()));

    // create uniswap v3 factory instance
    let factory: Arc<IUniswapV3Factory::IUniswapV3FactoryInstance<HttpClient, AnvilHttpProvider>> =
        Arc::new(IUniswapV3Factory::new(
            uniswap_v3_factory_address,
            anvil_provider.clone(),
        ));

    // create uniswap nonfungible position manager instance
    let nonfungible_position_manager: Arc<
        INonfungiblePositionManager::INonfungiblePositionManagerInstance<
            HttpClient,
            AnvilHttpProvider,
        >,
    > = Arc::new(INonfungiblePositionManager::new(
        uniswap_v3_position_manager_address,
        anvil_provider.clone(),
    ));

    // create swap router instance
    let swap_router = Arc::new(ISwapRouter::new(
        uniswap_v3_swap_router_address,
        anvil_provider.clone(),
    ));

    // create addresses with funds
    let clanker = Address::from_str("0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266")?;

    // add funds to addresses
    initialize_simulation_account(
        anvil_provider.clone(),
        clanker,
        U256::from(1e18 as u64),
        None,
        weth.clone(),
        swap_router.address(),
        nonfungible_position_manager.address(),
    )
    .await?;

    // deploy pool
    let (pool, clanker_token) = deploy_and_initialize_pool(
        anvil_provider.clone(),
        factory.clone(),
        clanker,
        weth_address,
        create_event.try_into()?,
        init_event.try_into()?,
    )
    .await?;

    // approve clanker token for position manager and swap router for deployer
    approve_token(
        anvil_provider.clone(),
        clanker_token.clone(),
        nonfungible_position_manager.address(),
        swap_router.address(),
        clanker,
    )
    .await?;

    // mint clanker token
    pool_mint(
        anvil_provider.clone(),
        nonfungible_position_manager.clone(),
        pool.clone(),
        clanker,
        &mint_event,
    )
    .await?;

    // first swap
    pool_swap(
        anvil_provider.clone(),
        pool.clone(),
        swap_router.clone(),
        clanker,
        &swap_event,
    )
    .await?;

    Ok(())
}
