use std::{str::FromStr, sync::Arc};

use alloy::{
    primitives::{Address, Log as AbiLog, U256},
    sol_types::SolEvent,
};
use eyre::{bail, Context, ContextCompat, Result};
use tracing::error;

use crate::{
    abi::{
        ClankerToken::ClankerTokenInstance,
        INonfungiblePositionManager::{INonfungiblePositionManagerInstance, MintParams},
        UniswapV3Pool::Mint,
        Weth::WethInstance,
    },
    fee_analyzer::simulation_events::IncreaseLiquidityWithParams,
};

use crate::fee_analyzer::{ArcAnvilHttpProvider, HttpClient};

use super::{initialize_simulation_account, PoolConfig};

pub(crate) async fn initialize_mint_account(
    anvil_provider: ArcAnvilHttpProvider,
    token: Arc<ClankerTokenInstance<HttpClient, ArcAnvilHttpProvider>>,
    weth: Arc<WethInstance<HttpClient, ArcAnvilHttpProvider>>,
    swap_router: &Address,
    swap_account: &Address,
    position_manager: &Address,
    mint: &Mint,
    pool_config: &PoolConfig,
) -> Result<Address> {
    let new_minter = Address::random();

    // initialize with weth and sign approvals
    initialize_simulation_account(
        anvil_provider,
        new_minter,
        Some(token.clone()),
        weth,
        swap_router,
        position_manager,
    )
    .await?;

    // send needed clanker tokens for mint
    let transfer = if pool_config.clanker_is_token0 {
        token
            .transfer(new_minter, mint.amount0)
            .from(swap_account.clone())
            .send()
            .await?
            .get_receipt()
            .await?
    } else {
        token
            .transfer(new_minter, mint.amount1)
            .from(swap_account.clone())
            .send()
            .await?
            .get_receipt()
            .await?
    };

    if !transfer.inner.status() {
        error!("Failed to transfer clanker tokens");
        bail!("Failed to transfer clanker tokens");
    }

    Ok(new_minter)
}

pub(crate) async fn pool_mint(
    position_manager: Arc<INonfungiblePositionManagerInstance<HttpClient, ArcAnvilHttpProvider>>,
    pool_config: &PoolConfig,
    minter: Address,
    mint_event: &Mint,
    increase_liquidity_event: &IncreaseLiquidityWithParams,
) -> Result<U256> {
    let mint_params = MintParams {
        token0: pool_config.token0,
        token1: pool_config.token1,
        fee: pool_config.fee,
        tickLower: mint_event.tickLower,
        tickUpper: mint_event.tickUpper,
        amount0Desired: increase_liquidity_event.amount_0_desired,
        amount1Desired: increase_liquidity_event.amount_1_desired,
        amount0Min: U256::ZERO,
        amount1Min: U256::ZERO,
        recipient: minter,
        deadline: U256::from_str("8737924142").unwrap(),
    };

    // simulate mint first to grab result
    let token_id = position_manager
        .mint(mint_params.clone())
        .from(minter)
        .call()
        .await
        .context("Failed to simulate mint")?
        .tokenId;

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
        error!("event amount0: {:?}", mint_event.amount0);
        error!("log   amount0: {:?}", mint_log.amount0);
        error!("event amount1: {:?}", mint_event.amount1);
        error!("log   amount1: {:?}", mint_log.amount1);
        error!("event tickLower: {:?}", mint_event.tickLower);
        error!("log   tickLower: {:?}", mint_log.tickLower);
        error!("event tickUpper: {:?}", mint_event.tickUpper);
        error!("log   tickUpper: {:?}", mint_log.tickUpper);
        error!("event amount: {:?}", mint_event.amount);
        error!("log   amount: {:?}", mint_log.amount);
        bail!("Mismatch in mint outcomes");
    }

    Ok(token_id)
}
