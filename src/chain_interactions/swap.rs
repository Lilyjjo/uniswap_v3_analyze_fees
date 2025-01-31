use std::sync::Arc;

use alloy::{
    primitives::{aliases::U24, ruint::aliases::U256, Address, Log as AbiLog, I256, U160},
    rpc::types::TransactionReceipt,
    sol_types::SolEvent,
};
use eyre::{bail, Context, ContextCompat, Result};
use tracing::{error, info};

use crate::{
    abi::{
        IQuoterV2::{IQuoterV2Instance, QuoteExactInputSingleParams},
        ISwapRouter::{ExactInputSingleParams, ExactOutputSingleParams, ISwapRouterInstance},
        UniswapV3Pool::{Swap, UniswapV3PoolInstance},
    },
    fee_analyzer::{ArcAnvilHttpProvider, HttpClient},
};

struct SwapParams {
    token_in: Address,
    token_out: Address,
    amount_in: U256,
    amount_out: U256,
    fee: U24,
}

enum SwapDirection {
    ExactInput,
    ExactOutput,
}

pub async fn pool_swap(
    pool: Arc<UniswapV3PoolInstance<HttpClient, ArcAnvilHttpProvider>>,
    swap_router: Arc<ISwapRouterInstance<HttpClient, ArcAnvilHttpProvider>>,
    quoter: Arc<IQuoterV2Instance<HttpClient, ArcAnvilHttpProvider>>,
    swap_event: &Swap,
    swapper: Address,
) -> Result<()> {
    let swap_params = swap_params(swap_event, &pool).await?;
    let swap_direction = swap_direction(&swap_params, &quoter).await?;

    match swap_direction {
        SwapDirection::ExactInput => {
            pool_swap_exact_input(swap_router, swapper, swap_event, &swap_params).await
        }
        SwapDirection::ExactOutput => {
            pool_swap_exact_output(swap_router, swapper, swap_event, &swap_params).await
        }
    }
}

async fn swap_params(
    swap_event: &Swap,
    pool: &UniswapV3PoolInstance<HttpClient, ArcAnvilHttpProvider>,
) -> Result<SwapParams> {
    let token_0 = pool.token0().call().await?._0;
    let token_1 = pool.token1().call().await?._0;
    let fee = pool.fee().call().await?._0;

    // get token in/out and amount in
    let (token_in, token_out, amount_in, amount_out) = if swap_event.amount0 < I256::ZERO {
        (
            token_1,
            token_0,
            swap_event.amount1.abs(),
            swap_event.amount0.abs(),
        )
    } else {
        (
            token_0,
            token_1,
            swap_event.amount0.abs(),
            swap_event.amount1.abs(),
        )
    };

    Ok(SwapParams {
        token_in,
        token_out,
        amount_in: U256::try_from(amount_in).context("failed to convert amount_in to U256")?,
        amount_out: U256::try_from(amount_out).context("failed to convert amount_out to U256")?,
        fee,
    })
}

async fn swap_direction(
    swap_params: &SwapParams,
    quoter: &IQuoterV2Instance<HttpClient, ArcAnvilHttpProvider>,
) -> Result<SwapDirection> {
    // get quote for swap exact in, if matches event's out, then swap ExactIn
    let quote_params = QuoteExactInputSingleParams {
        tokenIn: swap_params.token_in,
        tokenOut: swap_params.token_out,
        fee: swap_params.fee,
        amountIn: swap_params.amount_in,
        sqrtPriceLimitX96: U160::from(0),
    };

    let quote = quoter
        .quoteExactInputSingle(quote_params)
        .call()
        .await
        .context("failed to get quote for swap exact in")?;

    if quote.amountOut == swap_params.amount_out {
        Ok(SwapDirection::ExactInput)
    } else {
        Ok(SwapDirection::ExactOutput)
    }
}

async fn check_swap_outcomes(swap_event: &Swap, tx_receipt: &TransactionReceipt) -> Result<()> {
    let swap_log = tx_receipt
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
        .context("Failed to find swap log in tx receipt")?;

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

async fn pool_swap_exact_input(
    swap_router: Arc<ISwapRouterInstance<HttpClient, ArcAnvilHttpProvider>>,
    swapper: Address,
    swap_event: &Swap,
    swap_params: &SwapParams,
) -> Result<()> {
    info!("swapping");

    // copy swap params
    let exact_input_params = ExactInputSingleParams {
        tokenIn: swap_params.token_in,
        tokenOut: swap_params.token_out,
        fee: swap_params.fee,
        recipient: swapper,
        amountIn: swap_params.amount_in,
        amountOutMinimum: U256::from(0),
        sqrtPriceLimitX96: U160::from(0),
    };

    let mut attempts = 0;
    let max_attempts = 4;
    let mut receipt = None;

    while attempts < max_attempts {
        match swap_router
            .exactInputSingle(exact_input_params.clone())
            .from(swapper)
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

    let receipt =
        receipt.ok_or_else(|| eyre::eyre!("Failed to swap after {} attempts", max_attempts))?;

    check_swap_outcomes(swap_event, &receipt).await?;

    Ok(())
}

async fn pool_swap_exact_output(
    swap_router: Arc<ISwapRouterInstance<HttpClient, ArcAnvilHttpProvider>>,
    swapper: Address,
    swap_event: &Swap,
    swap_params: &SwapParams,
) -> Result<()> {
    let exact_output_params = ExactOutputSingleParams {
        tokenIn: swap_params.token_in,
        tokenOut: swap_params.token_out,
        fee: swap_params.fee,
        recipient: swapper,
        amountOut: swap_params.amount_out,
        amountInMaximum: swap_params.amount_in,
        sqrtPriceLimitX96: U160::from(0),
    };

    let mut attempts = 0;
    let max_attempts = 4;
    let mut receipt = None;

    while attempts < max_attempts {
        match swap_router
            .exactOutputSingle(exact_output_params.clone())
            .from(swapper)
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

    let receipt =
        receipt.ok_or_else(|| eyre::eyre!("Failed to swap after {} attempts", max_attempts))?;

    check_swap_outcomes(swap_event, &receipt).await?;

    Ok(())
}
