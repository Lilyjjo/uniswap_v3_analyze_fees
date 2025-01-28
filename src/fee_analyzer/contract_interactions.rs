use std::{str::FromStr, sync::Arc};

use alloy::{
    node_bindings::{Anvil, AnvilInstance},
    primitives::{Address, Log as AbiLog, I256, U160, U256},
    providers::{ext::AnvilApi, layers::AnvilProvider, ProviderBuilder},
    sol_types::SolEvent,
    transports::http::reqwest::Url,
};
use eyre::{bail, Context, ContextCompat, Result};
use tracing::{error, info};

use crate::abi::{
    ClankerToken::{self, ClankerTokenInstance},
    INonfungiblePositionManager::{INonfungiblePositionManagerInstance, MintParams},
    ISwapRouter::{ExactInputSingleParams, ISwapRouterInstance},
    IUniswapV3Factory::{IUniswapV3FactoryInstance, PoolCreated},
    UniswapV3Pool::{self, Initialize, Mint, Swap, UniswapV3PoolInstance},
    Weth::WethInstance,
};

use super::{ArcAnvilHttpProvider, HttpClient};

pub(crate) async fn anvil_connection(
    http_url: String,
    fork_block: u64,
) -> Result<(Arc<AnvilInstance>, ArcAnvilHttpProvider)> {
    info!("Connecting to anvil...");
    let parsed_url: Url = http_url.parse()?;

    let anvil = Arc::new(
        Anvil::new()
            .fork(parsed_url)
            .fork_block_number(fork_block)
            .spawn(),
    );

    let provider = ProviderBuilder::new().on_http(anvil.endpoint().parse().unwrap());
    let anvil_provider = Arc::new(AnvilProvider::new(provider, anvil.clone()));

    Ok((anvil, anvil_provider))
}

pub(crate) async fn deploy_and_initialize_pool(
    anvil_provider: ArcAnvilHttpProvider,
    uniswap_factory: Arc<IUniswapV3FactoryInstance<HttpClient, ArcAnvilHttpProvider>>,
    deployer: Address,
    weth: Address,
    pool_create_event: PoolCreated,
    initialization_event: Initialize,
) -> Result<(
    Arc<UniswapV3PoolInstance<HttpClient, ArcAnvilHttpProvider>>,
    Arc<ClankerTokenInstance<HttpClient, ArcAnvilHttpProvider>>,
)> {
    // deploy clanker token with token0/token1 in same order
    let clanker_token_address = if pool_create_event.token0 == weth {
        pool_create_event.token1
    } else {
        pool_create_event.token0
    };
    let clanker_token = deploy_clanker_token(
        anvil_provider.clone(),
        deployer,
        deployer,
        clanker_token_address,
        weth,
    )
    .await?;

    // sort tokens
    let (token0, token1) = if pool_create_event.token0 == weth {
        (weth, clanker_token.address().clone())
    } else {
        (clanker_token.address().clone(), weth)
    };

    // deploy pool
    let receipt = uniswap_factory
        .createPool(token0, token1, pool_create_event.fee)
        .from(deployer)
        .send()
        .await?
        .get_receipt()
        .await?;

    if !receipt.inner.status() {
        bail!("Failed to create pool");
    }

    // fetch pool
    let pool = uniswap_factory
        .getPool(token0, token1, pool_create_event.fee)
        .from(deployer)
        .call()
        .await?;
    let pool = Arc::new(UniswapV3Pool::new(pool.pool, anvil_provider.clone()));

    info!("pool address: {:?}", pool.address());

    // initialize pool
    let receipt = pool
        .initialize(initialization_event.sqrtPriceX96)
        .from(deployer)
        .send()
        .await?
        .get_receipt()
        .await?;

    if !receipt.inner.status() {
        bail!("Failed to initialize pool");
    }

    // ensure initialization log matches event we're copying
    let initialization_log = receipt
        .inner
        .logs()
        .iter()
        .find(|log| log.inner.topics()[0] == Initialize::SIGNATURE_HASH)
        .and_then(|log| {
            let log = AbiLog::new(
                log.address(),
                log.topics().to_vec(),
                log.data().data.clone(),
            )
            .unwrap_or_default();
            Initialize::decode_log(&log, true).ok()
        })
        .context("Failed to decode mint event")?;

    if initialization_log.sqrtPriceX96 != initialization_event.sqrtPriceX96 {
        error!("Mismatch in initialization outcomes");
        error!("initialization event: {:?}", initialization_event);
        error!("initialization log: {:?}", initialization_log);
        bail!("Mismatch in initialization outcomes");
    }

    info!("pool initialized");
    Ok((pool, clanker_token))
}

pub(crate) async fn pool_mint(
    position_manager: Arc<INonfungiblePositionManagerInstance<HttpClient, ArcAnvilHttpProvider>>,
    pool: Arc<UniswapV3PoolInstance<HttpClient, ArcAnvilHttpProvider>>,
    minter: Address,
    mint_event: &Mint,
) -> Result<()> {
    info!("minting");

    let token0 = pool.token0().call().await?._0;
    let token1 = pool.token1().call().await?._0;
    let fee = pool.fee().call().await?._0;

    // copy mint params
    let mint_params = MintParams {
        token0,
        token1,
        fee,
        tickLower: mint_event.tickLower,
        tickUpper: mint_event.tickUpper,
        amount0Desired: mint_event.amount0,
        amount1Desired: mint_event.amount1,
        amount0Min: U256::from(0),
        amount1Min: U256::from(0),
        recipient: minter,
        deadline: U256::from_str("8737924142").unwrap(), // timestamp need to just be in future
    };

    let receipt = position_manager
        .mint(mint_params)
        .from(minter)
        .send()
        .await?
        .get_receipt()
        .await?;
    if !receipt.inner.status() {
        bail!("Failed to mint");
    }

    let mint_log = receipt
        .inner
        .logs()
        .iter()
        .find(|log| log.inner.topics()[0] == Mint::SIGNATURE_HASH)
        .and_then(|log| {
            let log = AbiLog::new(
                log.address(),
                log.topics().to_vec(),
                log.data().data.clone(),
            )
            .unwrap_or_default();
            Mint::decode_log(&log, true).ok()
        })
        .context("Failed to decode mint event")?;

    // check mint outcomes
    if mint_log.amount0 != mint_event.amount0
        || mint_log.amount1 != mint_event.amount1
        || mint_log.tickLower != mint_event.tickLower
        || mint_log.tickUpper != mint_event.tickUpper
        || mint_log.amount != mint_event.amount
    {
        error!("Mismatch in mint outcomes");
        error!("mint event: {:?}", mint_event);
        error!("mint log: {:?}", mint_log);
        bail!("Mismatch in mint outcomes");
    }

    Ok(())
}

pub(crate) async fn pool_swap(
    pool: Arc<UniswapV3PoolInstance<HttpClient, ArcAnvilHttpProvider>>,
    swap_router: Arc<ISwapRouterInstance<HttpClient, ArcAnvilHttpProvider>>,
    swapper: Address,
    swap_event: &Swap,
) -> Result<()> {
    info!("swapping");

    // grab token ordering
    let token_0 = pool.token0().call().await?._0;
    let token_1 = pool.token1().call().await?._0;
    let fee = pool.fee().call().await?._0;

    // get token in/out and amount in
    let (token_in, token_out, amount_in) =
        if swap_event.amount0 < I256::from_str("0").context("failed to create zero I25 value")? {
            (token_1, token_0, swap_event.amount1.abs())
        } else {
            (token_0, token_1, swap_event.amount0.abs())
        };

    // copy swap params
    let swap_params = ExactInputSingleParams {
        tokenIn: token_in,
        tokenOut: token_out,
        fee,
        recipient: swapper,
        amountIn: U256::try_from(amount_in).context("failed to convert amount_in to U256")?,
        amountOutMinimum: U256::from(0),
        sqrtPriceLimitX96: U160::from(0),
    };

    let receipt = swap_router
        .exactInputSingle(swap_params)
        .from(swapper)
        .send()
        .await?
        .get_receipt()
        .await?;
    if !receipt.inner.status() {
        bail!("Failed to swap");
    }

    let swap_log = receipt
        .inner
        .logs()
        .iter()
        .find(|log| log.inner.topics()[0] == Swap::SIGNATURE_HASH)
        .and_then(|log| {
            let log = AbiLog::new(
                log.address(),
                log.topics().to_vec(),
                log.data().data.clone(),
            )
            .unwrap_or_default();
            Swap::decode_log(&log, true).ok()
        })
        .context("Failed to decode swap event")?;

    // check swap outcomes
    if swap_log.amount0 != swap_event.amount0
        || swap_log.amount1 != swap_event.amount1
        || swap_log.sqrtPriceX96 != swap_event.sqrtPriceX96
        || swap_log.liquidity != swap_event.liquidity
        || swap_log.tick != swap_event.tick
    {
        error!("Mismatch in swap outcomes");
        error!("swap event: {:?}", swap_event);
        error!("swap log: {:?}", swap_log);
        bail!("Mismatch in swap outcomes");
    }

    Ok(())
}

// Prepares an account for use in simulation by:
// 1. Registering the account for impersonation
// 2. Giving the account the native token
// 3. Swapping half for WETH
// 4. Approving the swap router and position manager
pub(crate) async fn initialize_simulation_account(
    anvil_provider: ArcAnvilHttpProvider,
    address: Address,
    amount: U256,
    token: Option<Arc<ClankerTokenInstance<HttpClient, ArcAnvilHttpProvider>>>,
    weth: Arc<WethInstance<HttpClient, ArcAnvilHttpProvider>>,
    swap_router: &Address,
    position_manager: &Address,
) -> Result<()> {
    anvil_provider.anvil_set_balance(address, amount).await?;
    anvil_provider.anvil_impersonate_account(address).await?;

    // convert half of the native token to WETH
    weth.deposit()
        .from(address)
        .value(
            amount
                .checked_div(U256::from(2))
                .context("Failed to divide amount by 2")?,
        )
        .send()
        .await?
        .watch()
        .await?;

    if let Some(token) = token {
        approve_token(token, position_manager, swap_router, address).await?;
    }

    approve_weth(weth, position_manager, swap_router, address).await?;

    Ok(())
}

pub(crate) async fn approve_token(
    token: Arc<ClankerTokenInstance<HttpClient, ArcAnvilHttpProvider>>,
    position_manager: &Address,
    swap_router: &Address,
    approver: Address,
) -> Result<()> {
    let max_approval =
        U256::from_str("0xffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff")
            .unwrap();

    let receipt = token
        .approve(swap_router.clone(), max_approval)
        .from(approver)
        .send()
        .await?
        .get_receipt()
        .await?;
    if !receipt.inner.status() {
        bail!("Failed to approve token for swap router");
    }

    let receipt = token
        .approve(position_manager.clone(), max_approval)
        .from(approver)
        .send()
        .await?
        .get_receipt()
        .await?;
    if !receipt.inner.status() {
        bail!("Failed to approve token for position manager");
    }
    Ok(())
}

// TODO combine with approve_token if have time to figure
// out generics over the Sol types
pub(crate) async fn approve_weth(
    weth: Arc<WethInstance<HttpClient, ArcAnvilHttpProvider>>,
    position_manager: &Address,
    swap_router: &Address,
    approver: Address,
) -> Result<()> {
    let max_approval =
        U256::from_str("0xffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff")
            .unwrap();

    let receipt = weth
        .approve(swap_router.clone(), max_approval)
        .from(approver)
        .send()
        .await?
        .get_receipt()
        .await?;
    if !receipt.inner.status() {
        bail!("Failed to approve weth for swap router");
    }

    let receipt = weth
        .approve(position_manager.clone(), max_approval)
        .from(approver)
        .send()
        .await?
        .get_receipt()
        .await?;
    if !receipt.inner.status() {
        bail!("Failed to approve weth for position manager");
    }

    Ok(())
}

pub(crate) async fn deploy_clanker_token(
    anvil_provider: ArcAnvilHttpProvider,
    deployer: Address,
    fid_deployer: Address,
    target_address: Address,
    weth: Address,
) -> Result<Arc<ClankerTokenInstance<HttpClient, ArcAnvilHttpProvider>>> {
    let mut contract: ClankerTokenInstance<HttpClient, ArcAnvilHttpProvider>;
    loop {
        contract = ClankerToken::new(
            ClankerToken::deploy_builder(
                anvil_provider.clone(),
                String::from("ClankerToken"),
                String::from("CLNK"),
                U256::from_str("100000000000000000000000000000".into()).unwrap(),
                fid_deployer,
                U256::from(1),
                String::from("0x1234567890"),
                String::from("0x1234567890"),
            )
            .from(deployer)
            .deploy()
            .await?,
            anvil_provider.clone(),
        );

        if (weth < target_address) == (&weth < contract.address()) {
            break;
        }
    }
    info!(
        "New clanker token address: {:?}, original token address: {:?}",
        contract.address(),
        target_address
    );
    Ok(Arc::new(contract))
}
