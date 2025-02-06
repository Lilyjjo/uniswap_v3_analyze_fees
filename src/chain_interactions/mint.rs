use std::{str::FromStr, sync::Arc};

use alloy::{
    primitives::{Address, Log as AbiLog, U256},
    rpc::types::TransactionReceipt,
    sol_types::SolEvent,
};
use eyre::{bail, Context, ContextCompat, Result};
use tracing::error;

use crate::{
    abi::{
        ClankerToken::ClankerTokenInstance,
        INonfungiblePositionManager::{
            INonfungiblePositionManagerInstance, IncreaseLiquidityParams, MintParams,
        },
        UniswapV3Pool::Mint,
    },
    fee_analyzer::simulation_events::IncreaseLiquidityWithParams,
};

use crate::fee_analyzer::{ArcAnvilHttpProvider, HttpClient};

use super::PoolConfig;

pub(crate) async fn send_clanker_tokens(
    token: Arc<ClankerTokenInstance<HttpClient, ArcAnvilHttpProvider>>,
    pool_config: &PoolConfig,
    minter: Address,
    swap_account: &Address,
    mint_event: &Mint,
) -> Result<()> {
    // send needed clanker tokens for mint
    let transfer = if pool_config.clanker_is_token0 {
        if mint_event.amount0 == U256::ZERO {
            return Ok(());
        }
        token
            .transfer(minter, mint_event.amount0)
            .from(swap_account.clone())
            .send()
            .await?
            .get_receipt()
            .await?
    } else {
        if mint_event.amount1 == U256::ZERO {
            return Ok(());
        }
        token
            .transfer(minter, mint_event.amount1)
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

    Ok(())
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

    let mut attempts = 0;
    let max_attempts = 4;
    let mut receipt = None;

    while attempts < max_attempts {
        match position_manager
            .mint(mint_params.clone())
            .from(minter)
            .send()
            .await?
            .get_receipt()
            .await
        {
            Ok(r) => {
                if r.inner.status() {
                    receipt = Some(r);
                    break;
                }
            }
            Err(e) => {
                error!("Failed to mint, retrying: {:?}", e);
            }
        }
        attempts += 1;
    }

    let receipt =
        receipt.ok_or_else(|| eyre::eyre!("Failed to mint after {} attempts", max_attempts))?;

    check_mint_outcomes(mint_event, &receipt).await?;

    Ok(token_id)
}

pub(crate) async fn pool_increase_liquidity(
    position_manager: Arc<INonfungiblePositionManagerInstance<HttpClient, ArcAnvilHttpProvider>>,
    minter: Address,
    mint_event: &Mint,
    increase_liquidity_event: &IncreaseLiquidityWithParams,
    token_id: U256,
) -> Result<()> {
    let increase_liquidity_params = IncreaseLiquidityParams {
        tokenId: token_id,
        amount0Desired: increase_liquidity_event.amount_0_desired,
        amount1Desired: increase_liquidity_event.amount_1_desired,
        amount0Min: U256::ZERO,
        amount1Min: U256::ZERO,
        deadline: U256::from_str("8737924142").unwrap(),
    };

    let mut attempts = 0;
    let max_attempts = 4;
    let mut receipt = None;

    while attempts < max_attempts {
        match position_manager
            .increaseLiquidity(increase_liquidity_params.clone())
            .from(minter)
            .send()
            .await?
            .get_receipt()
            .await
        {
            Ok(r) => {
                if r.inner.status() {
                    receipt = Some(r);
                    break;
                }
            }
            Err(_) => {}
        }
        attempts += 1;
    }

    let receipt = receipt.ok_or_else(|| {
        eyre::eyre!(
            "Failed to increase liquidity after {} attempts",
            max_attempts
        )
    })?;

    // check increase liquidity outcomes
    check_mint_outcomes(mint_event, &receipt).await?;

    Ok(())
}

async fn check_mint_outcomes(mint_event: &Mint, receipt: &TransactionReceipt) -> Result<()> {
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

    Ok(())
}
