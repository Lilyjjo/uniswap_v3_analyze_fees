use std::{collections::HashMap, sync::Arc};

use alloy::{
    node_bindings::AnvilInstance,
    primitives::{Address, U256},
    providers::{layers::AnvilProvider, RootProvider},
    transports::http::{reqwest, Http},
};
use contract_interactions::{
    anvil_connection, approve_token, deploy_and_initialize_pool, initialize_simulation_account,
    pool_mint, pool_swap,
};
use csv_converter::{pool_events, CSVReaderConfig};
use eyre::{eyre, Context, Result};
use simulation_events::{find_first_event, EventType, SimulationEvent};
use tracing::info;

use crate::abi::{
    ClankerToken::ClankerTokenInstance,
    INonfungiblePositionManager, ISwapRouter, IUniswapV3Factory,
    UniswapV3Pool::{Mint, Swap, UniswapV3PoolInstance},
    Weth,
};

mod contract_interactions;
pub mod csv_converter;
mod simulation_events;

pub(crate) type HttpClient = Http<reqwest::Client>;
pub(crate) type ArcAnvilHttpProvider = Arc<AnvilProvider<RootProvider<HttpClient>, HttpClient>>;

#[allow(unused)]
pub struct PoolAnalyzer {
    anvil: Arc<AnvilInstance>,
    anvil_provider: ArcAnvilHttpProvider,
    pool: Arc<UniswapV3PoolInstance<HttpClient, ArcAnvilHttpProvider>>,
    clanker_token: Arc<ClankerTokenInstance<HttpClient, ArcAnvilHttpProvider>>,
    weth: Arc<Weth::WethInstance<HttpClient, ArcAnvilHttpProvider>>,
    factory: Arc<IUniswapV3Factory::IUniswapV3FactoryInstance<HttpClient, ArcAnvilHttpProvider>>,
    nonfungible_position_manager: Arc<
        INonfungiblePositionManager::INonfungiblePositionManagerInstance<
            HttpClient,
            ArcAnvilHttpProvider,
        >,
    >,
    swap_router: Arc<ISwapRouter::ISwapRouterInstance<HttpClient, ArcAnvilHttpProvider>>,
    pool_simulation_events: Vec<SimulationEvent>,
    address_map: HashMap<Address, Address>,
    clanker: Address,
}

pub struct PoolAnalyzerConfig {
    pub http_url: String,
    pub fork_block: u64,
    pub uniswap_v3_factory_address: Address,
    pub uniswap_v3_position_manager_address: Address,
    pub uniswap_v3_swap_router_address: Address,
    pub weth_address: Address,
    pub config: CSVReaderConfig,
}

impl PoolAnalyzer {
    pub async fn initialize(config: PoolAnalyzerConfig) -> Result<Self> {
        let (anvil, anvil_provider) = anvil_connection(config.http_url, config.fork_block)
            .await
            .context("Failed to connect to anvil")?;
        let weth = Arc::new(Weth::new(config.weth_address, anvil_provider.clone()));
        let factory = Arc::new(IUniswapV3Factory::new(
            config.uniswap_v3_factory_address,
            anvil_provider.clone(),
        ));
        let nonfungible_position_manager = Arc::new(INonfungiblePositionManager::new(
            config.uniswap_v3_position_manager_address,
            anvil_provider.clone(),
        ));
        let swap_router = Arc::new(ISwapRouter::new(
            config.uniswap_v3_swap_router_address,
            anvil_provider.clone(),
        ));
        let pool_simulation_events = pool_events(config.config)
            .await
            .context("Failed to get pool events from CSV")?;

        let create_event = find_first_event(&pool_simulation_events, EventType::PoolCreated)?;
        let init_event = find_first_event(&pool_simulation_events, EventType::Initialize)?;

        let mut address_map = HashMap::<Address, Address>::new();

        let clanker = create_event.from;
        let deployer = Address::random();
        address_map.insert(clanker, deployer);
        info!("Deployer: {}", deployer);
        info!("Clanker: {}", clanker);

        // add funds to clanker address
        initialize_simulation_account(
            anvil_provider.clone(),
            deployer,
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
            deployer,
            weth.address().clone(),
            create_event.try_into()?,
            init_event.try_into()?,
        )
        .await?;

        // approve clanker token for position manager and swap router for deployer
        approve_token(
            clanker_token.clone(),
            nonfungible_position_manager.address(),
            swap_router.address(),
            deployer,
        )
        .await?;

        Ok(Self {
            anvil,
            anvil_provider,
            pool,
            clanker_token,
            weth,
            factory,
            nonfungible_position_manager,
            swap_router,
            pool_simulation_events,
            address_map,
            clanker,
        })
    }

    pub async fn run_simulation(&self) -> Result<()> {
        let mint_event: Mint =
            find_first_event(&self.pool_simulation_events, EventType::Mint)?.try_into()?;
        let swap_event: Swap =
            find_first_event(&self.pool_simulation_events, EventType::Swap)?.try_into()?;

        let deployer = self
            .address_map
            .get(&self.clanker)
            .ok_or(eyre!("Deployer not found"))?;

        // mint clanker token
        pool_mint(
            self.nonfungible_position_manager.clone(),
            self.pool.clone(),
            deployer.clone(),
            &mint_event,
        )
        .await?;

        // first swap
        pool_swap(
            self.pool.clone(),
            self.swap_router.clone(),
            deployer.clone(),
            &swap_event,
        )
        .await?;

        Ok(())
    }
}
