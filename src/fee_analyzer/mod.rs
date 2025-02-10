use std::{collections::HashMap, sync::Arc};

use crate::{
    abi::IQuoterV2,
    chain_interactions::{
        anvil_connection, approve_token,
        burn::pool_burn,
        collect::{
            create_position_info_from_mint_event, pool_close_out_position,
            pool_collect_fees_post_decrease_liquidity, pool_collect_fees_post_increase_liquidity,
            PositionInfo,
        },
        deploy_and_initialize_pool, initialize_simulation_account,
        mint::{pool_increase_liquidity, pool_mint, send_clanker_tokens},
        swap::pool_swap,
        PoolConfig,
    },
};
use alloy::{
    node_bindings::AnvilInstance,
    primitives::{Address, U256},
    providers::{layers::AnvilProvider, RootProvider},
    transports::http::{reqwest, Http},
};
use csv_input_reader::{pool_events, CSVReaderConfig};
use csv_output_writer::write_positions_to_csv;
use eyre::{bail, eyre, Context, ContextCompat, Result};
use simulation_events::{
    find_first_event, DecreaseLiquidityWithParams, Event, EventType, IncreaseLiquidityWithParams,
    SimulationEvent,
};
use tracing::{error, info, warn};

use crate::abi::{
    ClankerToken::ClankerTokenInstance,
    INonfungiblePositionManager::{self},
    ISwapRouter,
    IUniswapV3Factory::{self},
    UniswapV3Pool::UniswapV3PoolInstance,
    Weth,
};

pub mod csv_input_reader;
pub mod csv_output_writer;
pub(crate) mod simulation_events;

pub type HttpClient = Http<reqwest::Client>;
pub type ArcAnvilHttpProvider = Arc<AnvilProvider<RootProvider<HttpClient>, HttpClient>>;

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
    quoter: Arc<IQuoterV2::IQuoterV2Instance<HttpClient, ArcAnvilHttpProvider>>,
    pool_simulation_events: Option<Vec<SimulationEvent>>,
    address_map: HashMap<Address, Address>,
    token_id_map: HashMap<U256, U256>,
    clanker: Address,
    swap_account: Address,
    mint_account: Address,
    pool_config: PoolConfig,
    position_info: HashMap<U256, Vec<PositionInfo>>,
    output_csv_file_path: String,
}

pub struct PoolAnalyzerConfig {
    pub http_url: String,
    pub fork_block: u64,
    pub uniswap_v3_factory_address: Address,
    pub uniswap_v3_position_manager_address: Address,
    pub uniswap_v3_swap_router_address: Address,
    pub uniswap_v3_quoter_address: Address,
    pub weth_address: Address,
    pub config: CSVReaderConfig,
    pub output_csv_file_path: String,
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
        let quoter = Arc::new(IQuoterV2::new(
            config.uniswap_v3_quoter_address,
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
            None,
            weth.clone(),
            swap_router.address(),
            nonfungible_position_manager.address(),
        )
        .await?;

        // deploy pool
        let (pool, clanker_token, pool_config) = deploy_and_initialize_pool(
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

        // setup swap account, we use the same address for all swaps
        // because we don't care about swapper PNL in this simulation
        let swap_account = Address::random();
        initialize_simulation_account(
            anvil_provider.clone(),
            swap_account,
            Some(clanker_token.clone()),
            weth.clone(),
            swap_router.address(),
            nonfungible_position_manager.address(),
        )
        .await?;

        // setup mint account, we use the same address for all minting
        // because we only care about the PNL of the position, not the
        // address associated with the mint.
        //
        // we could use different addresses, but the simluations were being
        // slowed down in the mint account setup flow and we didn't
        // track NFT transfers (we could if needed for some other reason)
        let mint_account = Address::random();
        initialize_simulation_account(
            anvil_provider.clone(),
            mint_account,
            Some(clanker_token.clone()),
            weth.clone(),
            swap_router.address(),
            nonfungible_position_manager.address(),
        )
        .await?;

        // send all clanker tokens to swap account, tokens needed for minting
        // are pulled from this account on a per mint basis
        let total_supply = clanker_token.totalSupply().call().await?._0;
        clanker_token
            .transfer(swap_account, total_supply)
            .from(deployer.clone())
            .send()
            .await?
            .get_receipt()
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
            quoter,
            pool_simulation_events: Some(pool_simulation_events),
            address_map,
            token_id_map: HashMap::new(),
            clanker,
            swap_account,
            mint_account,
            pool_config,
            position_info: HashMap::new(),
            output_csv_file_path: config.output_csv_file_path,
        })
    }

    pub async fn run_simulation(&mut self) -> Result<()> {
        // TODO: figure out how to make this prettier
        let mut event_iter = self
            .pool_simulation_events
            .take()
            .unwrap()
            .into_iter()
            .peekable();
        // skip first two event, they are pool created and initialize TODO clean up
        event_iter.next();
        event_iter.next();
        let mut event_count = 0;

        while let Some(event) = event_iter.next() {
            info!("event: {:?}", event_count);
            info!("event: {:?}", event);
            event_count += 1;

            match event.event.clone() {
                Event::PoolCreated(create_event) => {
                    // first event is pool created, pool initialize should be next event
                    let initialize_event = if let Some(sim_event) = event_iter.peek() {
                        if sim_event.event.event_type() == EventType::Initialize {
                            event_iter
                                .next()
                                .context("Pool initialize event not found")?
                        } else {
                            bail!("Pool initialize event was not event after pool created");
                        }
                    } else {
                        bail!("No events after pool created");
                    };
                    deploy_and_initialize_pool(
                        self.anvil_provider.clone(),
                        self.factory.clone(),
                        self.clanker.clone(),
                        self.weth.address().clone(),
                        create_event,
                        initialize_event.try_into()?,
                    )
                    .await?;
                }
                Event::Initialize(e) => {
                    error!("Pool initialize event found in wrong positiong: {:?}", e);
                    bail!("Pool initialize events should be handled by pool created event");
                }
                Event::Mint(e) => {
                    warn!("Minting");

                    send_clanker_tokens(
                        self.clanker_token.clone(),
                        &self.pool_config,
                        self.mint_account.clone(),
                        &self.swap_account,
                        &e,
                    )
                    .await?;

                    // next event should be liquidity add
                    let increase_liquidity_event: IncreaseLiquidityWithParams =
                        if let Some(sim_event) = event_iter.peek() {
                            if sim_event.event.event_type() == EventType::IncreaseLiquidity {
                                event_iter
                                    .next()
                                    .context("Increase liquidity event not found")?
                                    .try_into()?
                            } else {
                                bail!("Increase liquidity event was not event after mint");
                            }
                        } else {
                            bail!("No events after mint");
                        };

                    // check if token id already exists, this means that it's a increaseLiqiudity call
                    // instead of a fresh nft mint, both have the same events emitted
                    if let Some(token_id) = self
                        .token_id_map
                        .get(&increase_liquidity_event.event.tokenId)
                    {
                        // position already exists, increase liquidity
                        pool_increase_liquidity(
                            self.nonfungible_position_manager.clone(),
                            self.mint_account.clone(),
                            &e,
                            &increase_liquidity_event,
                            token_id.clone(),
                        )
                        .await?;

                        // find position
                        let position = self
                            .position_info
                            .get_mut(&token_id)
                            .unwrap()
                            .last_mut()
                            .context("Position info not found for increase liquidity")?;

                        // update position pnl info as if new position was created
                        let position_info = pool_collect_fees_post_increase_liquidity(
                            self.nonfungible_position_manager.clone(),
                            self.pool.clone(),
                            self.swap_router.clone(),
                            &self.pool_config,
                            self.mint_account.clone(),
                            self.swap_account.clone(),
                            token_id.clone(),
                            position,
                            event.block,
                            increase_liquidity_event,
                        )
                        .await?;

                        // insert position info into map
                        let position_info_vec = self.position_info.get_mut(&token_id).unwrap();
                        position_info_vec.push(position_info);
                    } else {
                        // token id not found, this is a fresh mint
                        let token_id = pool_mint(
                            self.nonfungible_position_manager.clone(),
                            &self.pool_config,
                            self.mint_account.clone(),
                            &e,
                            &increase_liquidity_event,
                        )
                        .await?;

                        self.token_id_map
                            .insert(increase_liquidity_event.event.tokenId, token_id);

                        // create new position info
                        let position = create_position_info_from_mint_event(
                            self.pool.clone(),
                            &self.pool_config,
                            self.swap_router.clone(),
                            self.swap_account.clone(),
                            event.clone(),
                            token_id,
                            increase_liquidity_event.event.tokenId,
                        )
                        .await?;

                        // insert position info into map
                        self.position_info.insert(token_id, vec![position]);
                    }
                }
                Event::Swap(e) => {
                    info!("swapping");
                    pool_swap(
                        self.pool.clone(),
                        self.swap_router.clone(),
                        self.quoter.clone(),
                        &e,
                        self.swap_account,
                    )
                    .await?;
                }
                Event::Burn(e) => {
                    warn!("Burn: {:?}", e);

                    // burns are always followed by a collectPool or decreaseLiquidity event,
                    // only want to replay the decreaseLiquidity event as the collect event is
                    // a zero-liquditiy burn done to update the pool fees
                    let next_event = if let Some(sim_event) = event_iter.peek() {
                        if sim_event.event.event_type() == EventType::CollectPool
                            || sim_event.event.event_type() == EventType::DecreaseLiquidity
                        {
                            event_iter.next().unwrap()
                        } else {
                            bail!("Next event is not a collectPool or decreaseLiquidity");
                        }
                    } else {
                        bail!("No events after burn");
                    };

                    if next_event.event.event_type() == EventType::DecreaseLiquidity {
                        let decrease_liquidity_event: DecreaseLiquidityWithParams =
                            next_event.try_into()?;

                        // process decrease liquidity event which triggered the burn event
                        let token_id = self
                            .token_id_map.get(&decrease_liquidity_event.event.tokenId)
                            .context("Token id not found for Burn, mismatch between burn and mint position manager events")?;
                        pool_burn(
                            self.nonfungible_position_manager.clone(),
                            token_id.clone(),
                            self.mint_account.clone(),
                            &e,
                            &decrease_liquidity_event,
                        )
                        .await?;

                        // find the position info that should exist for the token id
                        let position = self
                            .position_info
                            .get_mut(&token_id)
                            .unwrap()
                            .last_mut()
                            .context("Position info not found DL")?;

                        // process the position info pnl
                        let position_info = pool_collect_fees_post_decrease_liquidity(
                            self.nonfungible_position_manager.clone(),
                            self.pool.clone(),
                            self.swap_router.clone(),
                            &self.pool_config,
                            self.mint_account.clone(),
                            self.swap_account.clone(),
                            token_id.clone(),
                            position,
                            event.block,
                            decrease_liquidity_event,
                        )
                        .await?;

                        // insert the new position into the map
                        let position_info_vec = self.position_info.get_mut(&token_id).unwrap();
                        position_info_vec.push(position_info);
                    }
                }
                Event::IncreaseLiquidity(e) => {
                    error!(
                        "Increase liquidity event not processed in mint handling: {:?}",
                        e
                    );
                    info!("tx hash: {:?}", event.tx_hash);
                    bail!("Increase liquidity event not processed in mint handling");
                }
                Event::DecreaseLiquidity(e) => {
                    error!(
                        "Decrease liquidity event not processed in burn handling: {:?}",
                        e
                    );
                    info!("tx hash: {:?}", event.tx_hash);
                    bail!("Decrease liquidity event not processed in burn handling");
                }
                _ => {
                    // not handling collect events as we do it manually after
                    // liquidity position changes
                    warn!("Unhandled event: {:?}", event);
                }
            }
        }

        // close out all positions
        for (token_id, position_infos) in self.position_info.iter_mut() {
            let mut closed_found = false;
            for position_info in position_infos.iter_mut() {
                if !position_info.closed {
                    if closed_found {
                        bail!("Multiple positions found for token id: {}", token_id);
                    } else {
                        closed_found = true;
                    }
                    info!("closing position: ---");
                    pool_close_out_position(
                        self.nonfungible_position_manager.clone(),
                        self.pool.clone(),
                        self.swap_router.clone(),
                        &self.pool_config,
                        self.mint_account.clone(),
                        self.swap_account.clone(),
                        token_id.clone(),
                        position_info,
                        0,
                    )
                    .await?;
                }
                if position_info.liquidity_in > u128::try_from(0).unwrap() {
                    info!("{}", position_info);
                }
            }
        }

        // filter out empty positions and write to csv
        write_positions_to_csv(
            self.position_info
                .values()
                .flatten()
                .filter(|p| p.liquidity_in > u128::try_from(0).unwrap())
                .cloned()
                .collect(),
            &self.output_csv_file_path,
        )
        .map_err(|e| eyre!("Failed to write positions to csv: {}", e))?;
        Ok(())
    }
}
