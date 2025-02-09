use std::{fmt, sync::Arc};

use alloy::{
    primitives::{aliases::I24, Address, Log as AbiLog, I256, U160, U256},
    sol_types::SolEvent,
};
use eyre::{ContextCompat, Result};
use tracing::{error, warn};

use crate::{
    abi::{
        INonfungiblePositionManager::{
            Collect, CollectParams, DecreaseLiquidityParams, INonfungiblePositionManagerInstance,
        },
        ISwapRouter::{ExactInputSingleParams, ISwapRouterInstance},
        UniswapV3Pool::{Mint, UniswapV3PoolInstance},
    },
    fee_analyzer::simulation_events::{
        DecreaseLiquidityWithParams, IncreaseLiquidityWithParams, SimulationEvent,
    },
};

use crate::fee_analyzer::{ArcAnvilHttpProvider, HttpClient};

use super::PoolConfig;

#[derive(Debug, Clone)]
pub(crate) enum PositionAction {
    Open,
    IncreaseLiquidity,
    DecreaseLiquidity,
    ClosePosition,
}

impl fmt::Display for PositionAction {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            PositionAction::Open => write!(f, "Open"),
            PositionAction::IncreaseLiquidity => write!(f, "IncreaseLiquidity"),
            PositionAction::DecreaseLiquidity => write!(f, "DecreaseLiquidity"),
            PositionAction::ClosePosition => write!(f, "ClosePosition"),
        }
    }
}

#[derive(Debug, Clone)]
pub(crate) struct PositionInfo {
    // metadata
    pub token_id: U256,
    pub original_token_id: U256,
    pub lower_tick: I24,
    pub upper_tick: I24,
    pub index: u64,
    pub position_action: PositionAction,
    pub closed: bool,
    // opening info
    pub block_in: u64,
    pub token_amount_in: U256,
    pub weth_amount_in: U256,
    pub sqrt_price_limit_x96_in: U160,
    pub tick_in: I24,
    pub liquidity_in: u128,
    // closing info
    pub block_out: u64,
    pub token_amount_out: U256,
    pub weth_amount_out: U256,
    pub sqrt_price_limit_x96_out: U160,
    pub tick_out: I24,
    // fees info
    pub fees_earned_token: U256,
    pub fees_earned_weth: U256,
    // approximate values for pnl calc
    // to try to represent impermanent loss
    // with fee offset
    pub approx_starting_weth: U256, // weth in + weth value of token in
    pub approx_ending_weth: U256,   // weth out + weth fees + weth value of (token out + token fees)
    pub end_token_gain_separate: I256, // token out + token fees - token in
    pub end_weth_gain_separate: I256, // weth out + weth fees - weth in
    pub end_weth_gain_converted: I256, // approx_ending_weth - approx_starting_weth
}

impl fmt::Display for PositionInfo {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "\nPosition Info:\n\
             ├─ Token ID:                  {}\n\
             ├─ Token Action Index:        {}\n\
             ├─ Action Taken:              {}\n\
             ├─ Lower Tick:                {}\n\
             ├─ Upper Tick:                {}\n\
             ├─ Opening info:\n\
             │  ├─ Block In:                  {}\n\
             │  ├─ Token Amount In:           {}\n\
             │  ├─ WETH Amount In:            {}\n\
             │  ├─ SqrtPriceLimitX96 In:      {}\n\
             │  ├─ Tick In:                   {}\n\
             │  ├─ Liquidity In:              {}\n\
             ├─ Closing info:\n\
             │  ├─ Block Out:                 {}\n\
             │  ├─ Token Amount Out:          {}\n\
             │  ├─ WETH Amount Out:           {}\n\
             │  ├─ SqrtPriceLimitX96 Out:     {}\n\
             │  └─ Tick Out:                   {}\n\
             ├─ Position PNL ---\n\
             │  token fees earned:                   {}\n\
             │  weth fees earned:                    {}\n\
             │  net token gain (if position closed): {}\n\
             │  net weth gain (if position closed):  {}\n\
             │  approx starting weth:  {}\n\
             │  approx ending weth:    {}\n\
             └─ net pnl in weth:       {}",
            self.original_token_id,
            self.index,
            self.position_action,
            self.lower_tick,
            self.upper_tick,
            self.block_in,
            self.token_amount_in,
            self.weth_amount_in,
            self.sqrt_price_limit_x96_in,
            self.tick_in,
            self.liquidity_in,
            self.block_out,
            self.token_amount_out,
            self.weth_amount_out,
            self.sqrt_price_limit_x96_out,
            self.tick_out,
            self.fees_earned_token,
            self.fees_earned_weth,
            self.end_token_gain_separate,
            self.end_weth_gain_separate,
            self.approx_starting_weth,
            self.approx_ending_weth,
            self.end_weth_gain_converted,
        )
    }
}

// simulates the amount of weth that would be received from swapping the given token amount,
// used to approximate the starting and ending weth value of the positions. note that this is
// not 100% accurate because sometimes this is ran when the position is still open and could
// be consumed during the swap.
async fn sim_swap_token_for_weth(
    swap_router: Arc<ISwapRouterInstance<HttpClient, ArcAnvilHttpProvider>>,
    pool_config: &PoolConfig,
    token_amount_out: U256,
    swap_account: Address,
) -> Result<U256> {
    if token_amount_out == U256::ZERO {
        return Ok(U256::ZERO);
    }

    let (clanker_address, weth_address) = if pool_config.clanker_is_token0 {
        (pool_config.token0, pool_config.token1)
    } else {
        (pool_config.token1, pool_config.token0)
    };

    let exact_input_params = ExactInputSingleParams {
        tokenIn: clanker_address,
        tokenOut: weth_address,
        fee: pool_config.fee,
        recipient: swap_account,
        amountIn: token_amount_out,
        amountOutMinimum: U256::from(0),
        sqrtPriceLimitX96: U160::from(0),
    };

    let swap_router_call = swap_router
        .exactInputSingle(exact_input_params)
        .from(swap_account)
        .call()
        .await?;
    Ok(swap_router_call.amountOut)
}

#[derive(Debug, Clone)]
struct DecreaseLiquidityResult {
    token_out: U256,
    weth_out: U256,
}

async fn sim_decrease_liquidity(
    position_manager: Arc<INonfungiblePositionManagerInstance<HttpClient, ArcAnvilHttpProvider>>,
    pool_config: &PoolConfig,
    token_id: U256,
    minter: Address,
    liquidity: u128,
) -> Result<DecreaseLiquidityResult> {
    let decrease_liquidity_params = DecreaseLiquidityParams {
        tokenId: token_id,
        liquidity: liquidity,
        amount0Min: U256::ZERO,
        amount1Min: U256::ZERO,
        deadline: U256::MAX,
    };

    let decrease_liquidity_return = position_manager
        .decreaseLiquidity(decrease_liquidity_params)
        .from(minter)
        .call()
        .await?;

    if pool_config.clanker_is_token0 {
        Ok(DecreaseLiquidityResult {
            token_out: decrease_liquidity_return.amount0,
            weth_out: decrease_liquidity_return.amount1,
        })
    } else {
        Ok(DecreaseLiquidityResult {
            token_out: decrease_liquidity_return.amount1,
            weth_out: decrease_liquidity_return.amount0,
        })
    }
}

async fn collect_max_fees(
    position_manager: Arc<INonfungiblePositionManagerInstance<HttpClient, ArcAnvilHttpProvider>>,
    token_id: U256,
    minter: Address,
) -> Result<AbiLog<Collect>> {
    let collect_params = CollectParams {
        tokenId: token_id,
        recipient: minter,
        amount0Max: u128::MAX,
        amount1Max: u128::MAX,
    };

    let mut attempts = 0;
    let max_attempts = 4;
    let mut receipt = None;

    while attempts < max_attempts {
        match position_manager
            .collect(collect_params.clone())
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

    let collect_receipt = receipt
        .ok_or_else(|| eyre::eyre!("Failed to collect fees after {} attempts", max_attempts))?;

    let collect_log = collect_receipt
        .inner
        .logs()
        .iter()
        .find(|log| log.inner.topics()[0] == Collect::SIGNATURE_HASH)
        .and_then(|log| {
            let log = AbiLog::new(
                log.address(),
                log.topics().to_vec(),
                log.data().data.clone(),
            )
            .unwrap_or_default();
            Collect::decode_log(&log, true).ok()
        })
        .context("Failed to decode collect event")?;

    Ok(collect_log)
}

pub async fn create_position_info_from_mint_event(
    pool: Arc<UniswapV3PoolInstance<HttpClient, ArcAnvilHttpProvider>>,
    pool_config: &PoolConfig,
    swap_router: Arc<ISwapRouterInstance<HttpClient, ArcAnvilHttpProvider>>,
    swap_account: Address,
    original_mint_event: SimulationEvent,
    token_id: U256,
    original_token_id: U256,
) -> Result<PositionInfo> {
    let mint_event = Mint::try_from(original_mint_event.clone())?;

    let (token_amount_in, weth_amount_in) = if pool_config.clanker_is_token0 {
        (mint_event.amount0, mint_event.amount1)
    } else {
        (mint_event.amount1, mint_event.amount0)
    };

    // approximate the starting value of the position in weth
    // by converting the starting token amount into weth

    // check that this isn't the first mint event using fee growth as proxy
    let fee_growth_check = if pool_config.clanker_is_token0 {
        pool.feeGrowthGlobal0X128().call().await?._0
    } else {
        pool.feeGrowthGlobal1X128().call().await?._0
    };

    let token_converted_to_weth = if token_amount_in > U256::ZERO && fee_growth_check > U256::ZERO {
        sim_swap_token_for_weth(swap_router, pool_config, token_amount_in, swap_account).await?
    } else {
        U256::ZERO
    };

    let slot0 = pool.slot0().call().await?;

    let position_info = PositionInfo {
        token_id,
        original_token_id,
        index: 0,
        lower_tick: mint_event.tickLower,
        upper_tick: mint_event.tickUpper,
        tick_in: slot0.tick,
        tick_out: I24::ZERO,
        closed: false,
        block_in: original_mint_event.block,
        token_amount_in,
        weth_amount_in,
        sqrt_price_limit_x96_in: slot0.sqrtPriceX96,
        liquidity_in: mint_event.amount,
        block_out: 0,
        token_amount_out: U256::ZERO,
        weth_amount_out: U256::ZERO,
        sqrt_price_limit_x96_out: U160::ZERO,
        fees_earned_token: U256::ZERO,
        fees_earned_weth: U256::ZERO,
        position_action: PositionAction::Open,
        approx_ending_weth: U256::ZERO,
        approx_starting_weth: token_converted_to_weth + weth_amount_in,
        end_token_gain_separate: I256::ZERO,
        end_weth_gain_separate: I256::ZERO,
        end_weth_gain_converted: I256::ZERO,
    };

    Ok(position_info)
}

async fn close_out_position_info(
    position_manager: Arc<INonfungiblePositionManagerInstance<HttpClient, ArcAnvilHttpProvider>>,
    pool: Arc<UniswapV3PoolInstance<HttpClient, ArcAnvilHttpProvider>>,
    swap_router: Arc<ISwapRouterInstance<HttpClient, ArcAnvilHttpProvider>>,
    pool_config: &PoolConfig,
    minter: Address,
    swap_account: Address,
    token_id: U256,
    position_info: &mut PositionInfo,
    block_out: u64,
    decrease_liquidity_event: Option<DecreaseLiquidityWithParams>,
) -> Result<()> {
    // set position as closed and record the block number
    position_info.closed = true;
    position_info.block_out = block_out;

    // collect all of the fees earned by the position
    let collect_log = collect_max_fees(position_manager.clone(), token_id, minter).await?;
    if pool_config.clanker_is_token0 {
        position_info.fees_earned_token = collect_log.amount0;
        position_info.fees_earned_weth = collect_log.amount1;
    } else {
        position_info.fees_earned_token = collect_log.amount1;
        position_info.fees_earned_weth = collect_log.amount0;
    }

    // get the closing price and tick of the position
    let slot0 = pool.slot0().call().await?;
    position_info.sqrt_price_limit_x96_out = slot0.sqrtPriceX96;
    position_info.tick_out = slot0.tick;

    // figure out ending token and weth balances if position was closed out
    //
    // cases are:
    // (1) position was fully closed out, no need to sim liquidity decrease
    // (2) position was partially closed out, simluate closing out the rest
    // (3) position was not closed out, simulate closing it fully out
    if let Some(decrease_liquidity_event) = decrease_liquidity_event {
        // case (1) and (2)
        let (dl_token_out_amount, dl_weth_out_amount) = if pool_config.clanker_is_token0 {
            (
                decrease_liquidity_event.event.amount0,
                decrease_liquidity_event.event.amount1,
            )
        } else {
            (
                decrease_liquidity_event.event.amount1,
                decrease_liquidity_event.event.amount0,
            )
        };

        if position_info.liquidity_in == decrease_liquidity_event.event.liquidity {
            // case (1)
            position_info.token_amount_out = dl_token_out_amount;
            position_info.weth_amount_out = dl_weth_out_amount;
        } else {
            // case (2)
            let decrease_liquidity_result = sim_decrease_liquidity(
                position_manager.clone(),
                pool_config,
                token_id,
                minter,
                position_info.liquidity_in - decrease_liquidity_event.event.liquidity,
            )
            .await?;

            position_info.token_amount_out =
                decrease_liquidity_result.token_out + dl_token_out_amount;
            position_info.weth_amount_out = decrease_liquidity_result.weth_out + dl_weth_out_amount;
        }
    } else {
        // case (3)
        let decrease_liquidity_result = sim_decrease_liquidity(
            position_manager.clone(),
            pool_config,
            token_id,
            minter,
            position_info.liquidity_in,
        )
        .await?;
        position_info.token_amount_out = decrease_liquidity_result.token_out;
        position_info.weth_amount_out = decrease_liquidity_result.weth_out;
    }

    // simulate selling the token for weth for pnl estimate
    // and add the weth out amount to get the total weth amount
    let token_amount_to_sell = position_info.token_amount_out + position_info.fees_earned_token;
    let token_converted_to_weth =
        sim_swap_token_for_weth(swap_router, pool_config, token_amount_to_sell, swap_account)
            .await?;

    position_info.approx_ending_weth =
        token_converted_to_weth + position_info.weth_amount_out + position_info.fees_earned_weth;

    position_info.end_weth_gain_separate = I256::try_from(position_info.weth_amount_out).unwrap()
        - I256::try_from(position_info.weth_amount_in).unwrap()
        + I256::try_from(position_info.fees_earned_weth).unwrap();
    position_info.end_token_gain_separate = I256::try_from(position_info.token_amount_out).unwrap()
        - I256::try_from(position_info.token_amount_in).unwrap()
        + I256::try_from(position_info.fees_earned_token).unwrap();
    position_info.end_weth_gain_converted = I256::try_from(position_info.approx_ending_weth)
        .unwrap()
        - I256::try_from(position_info.approx_starting_weth).unwrap();
    Ok(())
}

pub async fn pool_collect_fees_post_increase_liquidity(
    position_manager: Arc<INonfungiblePositionManagerInstance<HttpClient, ArcAnvilHttpProvider>>,
    pool: Arc<UniswapV3PoolInstance<HttpClient, ArcAnvilHttpProvider>>,
    swap_router: Arc<ISwapRouterInstance<HttpClient, ArcAnvilHttpProvider>>,
    pool_config: &PoolConfig,
    minter: Address,
    swap_account: Address,
    token_id: U256,
    position_info: &mut PositionInfo,
    block_out: u64,
    increase_liquidity_event: IncreaseLiquidityWithParams,
) -> Result<PositionInfo> {
    close_out_position_info(
        position_manager,
        pool,
        swap_router.clone(),
        pool_config,
        minter,
        swap_account,
        token_id,
        position_info,
        block_out,
        None,
    )
    .await?;

    // create new position info for the tokenid
    let (token_amount_increase, weth_amount_increase) = if pool_config.clanker_is_token0 {
        (
            increase_liquidity_event.event.amount0,
            increase_liquidity_event.event.amount1,
        )
    } else {
        (
            increase_liquidity_event.event.amount1,
            increase_liquidity_event.event.amount0,
        )
    };

    // get new position value by adding the increase amounts to the starting values
    let token_start = position_info.token_amount_in + token_amount_increase;
    let weth_start = position_info.weth_amount_in + weth_amount_increase;
    let token_converted_to_weth =
        sim_swap_token_for_weth(swap_router, pool_config, token_start, swap_account).await?;
    let starting_weth = token_converted_to_weth + weth_start;

    let new_position_info = PositionInfo {
        token_id: token_id,
        original_token_id: position_info.original_token_id,
        index: position_info.index + 1,
        lower_tick: position_info.lower_tick,
        upper_tick: position_info.upper_tick,
        tick_in: position_info.tick_out,
        tick_out: I24::ZERO,
        closed: false,
        block_in: block_out,
        token_amount_in: token_start,
        weth_amount_in: weth_start,
        sqrt_price_limit_x96_in: position_info.sqrt_price_limit_x96_out,
        liquidity_in: position_info.liquidity_in + increase_liquidity_event.event.liquidity,
        block_out: 0,
        token_amount_out: U256::ZERO,
        weth_amount_out: U256::ZERO,
        sqrt_price_limit_x96_out: U160::ZERO,
        fees_earned_token: U256::ZERO,
        fees_earned_weth: U256::ZERO,
        position_action: PositionAction::IncreaseLiquidity,
        approx_starting_weth: starting_weth,
        approx_ending_weth: U256::ZERO,
        end_token_gain_separate: I256::ZERO,
        end_weth_gain_separate: I256::ZERO,
        end_weth_gain_converted: I256::ZERO,
    };

    Ok(new_position_info)
}

pub(crate) async fn pool_collect_fees_post_decrease_liquidity(
    position_manager: Arc<INonfungiblePositionManagerInstance<HttpClient, ArcAnvilHttpProvider>>,
    pool: Arc<UniswapV3PoolInstance<HttpClient, ArcAnvilHttpProvider>>,
    swap_router: Arc<ISwapRouterInstance<HttpClient, ArcAnvilHttpProvider>>,
    pool_config: &PoolConfig,
    minter: Address,
    swap_account: Address,
    token_id: U256,
    position_info: &mut PositionInfo,
    block_out: u64,
    decrease_liquidity_event: DecreaseLiquidityWithParams,
) -> Result<PositionInfo> {
    // close out positon
    close_out_position_info(
        position_manager,
        pool,
        swap_router.clone(),
        pool_config,
        minter,
        swap_account,
        token_id,
        position_info,
        block_out,
        Some(decrease_liquidity_event.clone()),
    )
    .await?;

    // create next position info based on if the position was fully closed out
    if position_info.liquidity_in == decrease_liquidity_event.event.liquidity {
        warn!("position is fully closed, closing out");
        // create new position info with zero amounts in case
        // same position is used again in future (don't expect people to do this)
        Ok(PositionInfo {
            token_id: token_id,
            original_token_id: position_info.original_token_id,
            index: position_info.index + 1,
            lower_tick: position_info.lower_tick,
            upper_tick: position_info.upper_tick,
            closed: true,
            block_in: block_out,
            token_amount_in: U256::ZERO,
            weth_amount_in: U256::ZERO,
            sqrt_price_limit_x96_in: U160::ZERO,
            tick_in: I24::ZERO,
            liquidity_in: u128::try_from(0).unwrap(),
            block_out: 0,
            token_amount_out: U256::ZERO,
            weth_amount_out: U256::ZERO,
            sqrt_price_limit_x96_out: U160::ZERO,
            tick_out: I24::ZERO,
            fees_earned_token: U256::ZERO,
            fees_earned_weth: U256::ZERO,
            position_action: PositionAction::ClosePosition,
            approx_ending_weth: U256::ZERO,
            approx_starting_weth: U256::ZERO,
            end_token_gain_separate: I256::ZERO,
            end_weth_gain_separate: I256::ZERO,
            end_weth_gain_converted: I256::ZERO,
        })
    } else {
        warn!("position is partially closed, creating new position");
        // grab closed out token amounts to remove from the previous position
        let (dl_token_amount_out, dl_weth_amount_out) = if pool_config.clanker_is_token0 {
            (
                decrease_liquidity_event.event.amount0,
                decrease_liquidity_event.event.amount1,
            )
        } else {
            (
                decrease_liquidity_event.event.amount1,
                decrease_liquidity_event.event.amount0,
            )
        };

        let token_start = position_info
            .token_amount_in
            .checked_sub(dl_token_amount_out)
            .expect("token decrease larger than starting token amount");
        let weth_start = position_info
            .weth_amount_in
            .checked_sub(dl_weth_amount_out)
            .expect("weth decrease larger than starting weth amount");
        let token_converted_to_weth =
            sim_swap_token_for_weth(swap_router, pool_config, token_start, swap_account).await?;
        let starting_weth = token_converted_to_weth + weth_start;

        // positional partially closed, create new position with the remaining liquidity
        Ok(PositionInfo {
            token_id: token_id,
            original_token_id: position_info.original_token_id,
            index: position_info.index + 1,
            closed: false,
            lower_tick: position_info.lower_tick,
            upper_tick: position_info.upper_tick,
            tick_in: position_info.tick_out,
            tick_out: I24::ZERO,
            block_in: block_out,
            token_amount_in: token_start,
            weth_amount_in: weth_start,
            sqrt_price_limit_x96_in: position_info.sqrt_price_limit_x96_out,
            liquidity_in: position_info.liquidity_in - decrease_liquidity_event.event.liquidity,
            block_out: 0,
            token_amount_out: U256::ZERO,
            weth_amount_out: U256::ZERO,
            sqrt_price_limit_x96_out: U160::ZERO,
            fees_earned_token: U256::ZERO,
            fees_earned_weth: U256::ZERO,
            position_action: PositionAction::DecreaseLiquidity,
            approx_starting_weth: starting_weth,
            approx_ending_weth: U256::ZERO,
            end_token_gain_separate: I256::ZERO,
            end_weth_gain_separate: I256::ZERO,
            end_weth_gain_converted: I256::ZERO,
        })
    }
}

pub(crate) async fn pool_close_out_position(
    position_manager: Arc<INonfungiblePositionManagerInstance<HttpClient, ArcAnvilHttpProvider>>,
    pool: Arc<UniswapV3PoolInstance<HttpClient, ArcAnvilHttpProvider>>,
    swap_router: Arc<ISwapRouterInstance<HttpClient, ArcAnvilHttpProvider>>,
    pool_config: &PoolConfig,
    minter: Address,
    swap_account: Address,
    token_id: U256,
    position_info: &mut PositionInfo,
    block_out: u64,
) -> Result<()> {
    close_out_position_info(
        position_manager,
        pool,
        swap_router,
        pool_config,
        minter,
        swap_account,
        token_id,
        position_info,
        block_out,
        None,
    )
    .await?;

    Ok(())
}
