use std::path::Path;

use csv::WriterBuilder;
use eyre::Result;
use serde::Serialize;

use crate::chain_interactions::collect::PositionInfo;

pub fn write_positions_to_csv(
    positions: Vec<PositionInfo>,
    path: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    let path = Path::new(path);

    if let Some(parent) = path.parent() {
        if !parent.exists() {
            std::fs::create_dir_all(parent)?;
        }
    }

    let mut writer = WriterBuilder::new().has_headers(true).from_path(path)?;

    for position in positions {
        writer.serialize(convert_position_info_to_csv(position))?;
    }
    writer.flush()?;
    Ok(())
}

#[derive(Serialize)]
struct CSVPositionInfo {
    token_id: String,
    token_action_index: String,
    action_taken: String,
    lower_tick: String,
    upper_tick: String,
    opening_block: String,
    token_amount_in: String,
    weth_amount_in: String,
    sqrt_price_limit_x96_in: String,
    tick_in: String,
    liquidity_in: String,
    closing_block: String,
    token_amount_out: String,
    weth_amount_out: String,
    sqrt_price_limit_x96_out: String,
    tick_out: String,
    token_fees_earned: String,
    weth_fees_earned: String,
    net_token_gain: String,
    net_weth_gain: String,
    approx_starting_weth: String,
    approx_ending_weth: String,
    net_pnl_in_weth: String,
}

fn convert_position_info_to_csv(position_info: PositionInfo) -> CSVPositionInfo {
    CSVPositionInfo {
        token_id: position_info.token_id.to_string(),
        token_action_index: position_info.index.to_string(),
        action_taken: position_info.position_action.to_string(),
        lower_tick: position_info.lower_tick.to_string(),
        upper_tick: position_info.upper_tick.to_string(),
        opening_block: position_info.block_in.to_string(),
        token_amount_in: position_info.token_amount_in.to_string(),
        weth_amount_in: position_info.weth_amount_in.to_string(),
        sqrt_price_limit_x96_in: position_info.sqrt_price_limit_x96_in.to_string(),
        tick_in: position_info.tick_in.to_string(),
        liquidity_in: position_info.liquidity_in.to_string(),
        closing_block: position_info.block_out.to_string(),
        token_amount_out: position_info.token_amount_out.to_string(),
        weth_amount_out: position_info.weth_amount_out.to_string(),
        sqrt_price_limit_x96_out: position_info.sqrt_price_limit_x96_out.to_string(),
        tick_out: position_info.tick_out.to_string(),
        token_fees_earned: position_info.fees_earned_token.to_string(),
        weth_fees_earned: position_info.fees_earned_weth.to_string(),
        net_token_gain: position_info.end_token_gain_separate.to_string(),
        net_weth_gain: position_info.end_weth_gain_separate.to_string(),
        approx_starting_weth: position_info.approx_starting_weth.to_string(),
        approx_ending_weth: position_info.approx_ending_weth.to_string(),
        net_pnl_in_weth: position_info.end_weth_gain_converted.to_string(),
    }
}
