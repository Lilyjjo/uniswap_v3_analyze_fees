use std::{str::FromStr, sync::Arc};

use alloy::{
    primitives::{Address, Log as AbiLog, U256},
    rpc::types::TransactionReceipt,
    sol_types::SolEvent,
};
use eyre::{bail, ContextCompat, Result};
use tracing::error;

use crate::{
    abi::{
        INonfungiblePositionManager::{
            DecreaseLiquidityParams, INonfungiblePositionManagerInstance,
        },
        UniswapV3Pool::Burn,
    },
    fee_analyzer::simulation_events::DecreaseLiquidityWithParams,
};

use crate::fee_analyzer::{ArcAnvilHttpProvider, HttpClient};

pub(crate) async fn pool_burn(
    position_manager: Arc<INonfungiblePositionManagerInstance<HttpClient, ArcAnvilHttpProvider>>,
    token_id: U256,
    minter: Address,
    burn_event: &Burn,
    decrease_liquidity_event: &DecreaseLiquidityWithParams,
) -> Result<()> {
    let decrease_liquidity_params = DecreaseLiquidityParams {
        tokenId: token_id,
        liquidity: decrease_liquidity_event.event.liquidity,
        amount0Min: U256::ZERO,
        amount1Min: U256::ZERO,
        deadline: U256::from_str("8737924142").unwrap(),
    };

    let receipt = position_manager
        .decreaseLiquidity(decrease_liquidity_params)
        .from(minter)
        .send()
        .await?
        .get_receipt()
        .await?;

    if !receipt.inner.status() {
        bail!("Failed to burn");
    }

    // check burn outcomes
    check_burn_outcomes(burn_event, &receipt).await?;

    Ok(())
}

async fn check_burn_outcomes(burn_event: &Burn, receipt: &TransactionReceipt) -> Result<()> {
    let burn_log = receipt
        .inner
        .logs()
        .iter()
        .find(|log| log.inner.topics()[0] == Burn::SIGNATURE_HASH)
        .and_then(|log| {
            let log = AbiLog::new(
                log.address(),
                log.topics().to_vec(),
                log.data().data.clone(),
            )
            .unwrap_or_default();
            Burn::decode_log(&log, true).ok()
        })
        .context("Failed to decode mint event")?;

    if burn_log.amount0 != burn_event.amount0
        || burn_log.amount1 != burn_event.amount1
        || burn_log.amount != burn_event.amount
        || burn_log.tickLower != burn_event.tickLower
        || burn_log.tickUpper != burn_event.tickUpper
    {
        error!("Mismatch in burn outcomes");
        error!("burn event: {:?}", burn_event);
        error!("burn log: {:?}", burn_log);
        bail!("Mismatch in burn outcomes");
    }

    Ok(())
}
