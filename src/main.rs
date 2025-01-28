use alloy::primitives::Address;
use eyre::{Result, WrapErr};
use fee_analyzer::{csv_converter::CSVReaderConfig, PoolAnalyzer, PoolAnalyzerConfig};
use tracing::info;
use tracing_subscriber::{fmt::format::FmtSpan, EnvFilter};

mod abi;
mod fee_analyzer;

#[tokio::main]
async fn main() -> Result<()> {
    // Initialize tracing
    let subscriber = tracing_subscriber::fmt::Subscriber::builder()
        .compact()
        .with_env_filter(EnvFilter::from_default_env())
        .with_thread_ids(false)
        .with_target(false)
        .with_span_events(FmtSpan::NONE)
        .with_line_number(false)
        .finish();

    tracing::subscriber::set_global_default(subscriber)
        .context("Failed to set tracing subscriber")?;

    // get http urls
    let http_url = std::env::var("HTTP_URL").expect("HTTP_URL is required");

    // get deployed addresses
    let uniswap_v3_factory_address: Address = std::env::var("UNISWAP_V3_FACTORY_ADDRESS")
        .expect("UNISWAP_V3_FACTORY_ADDRESS is required")
        .parse()
        .expect("UNISWAP_V3_FACTORY_ADDRESS must be a valid address");

    let uniswap_v3_position_manager_address: Address =
        std::env::var("UNISWAP_V3_POSITION_MANAGER_ADDRESS")
            .expect("UNISWAP_V3_POSITION_MANAGER_ADDRESS is required")
            .parse()
            .expect("UNISWAP_V3_POSITION_MANAGER_ADDRESS must be a valid address");

    let uniswap_v3_swap_router_address: Address = std::env::var("UNISWAP_V3_SWAP_ROUTER_ADDRESS")
        .expect("UNISWAP_V3_SWAP_ROUTER_ADDRESS is required")
        .parse()
        .expect("UNISWAP_V3_SWAP_ROUTER_ADDRESS must be a valid address");

    let weth_address: Address = std::env::var("WETH_ADDRESS")
        .expect("WETH_ADDRESS is required")
        .parse()
        .expect("WETH_ADDRESS must be a valid address");

    let fork_block = std::env::var("BLOCK_FORK_NUMBER")
        .expect("FORK_BLOCK is required")
        .parse()
        .expect("FORK_BLOCK must be a valid number");

    // read csv file paths
    let initialize_events_path =
        std::env::var("INITIALIZE_CSV_FILE_PATH").expect("INITIALIZE_CSV_FILE_PATH is required");

    let swap_events_path =
        std::env::var("SWAP_CSV_FILE_PATH").expect("SWAP_CSV_FILE_PATH is required");

    let mint_events_path =
        std::env::var("MINT_CSV_FILE_PATH").expect("MINT_CSV_FILE_PATH is required");

    let burn_events_path =
        std::env::var("BURN_CSV_FILE_PATH").expect("BURN_CSV_FILE_PATH is required");

    let collect_events_path =
        std::env::var("COLLECT_CSV_FILE_PATH").expect("COLLECT_CSV_FILE_PATH is required");

    let pool_created_events_path = std::env::var("POOL_CREATED_CSV_FILE_PATH")
        .expect("POOL_CREATED_CSV_FILE_PATH is required");

    let csv_reader_config = CSVReaderConfig {
        initialize_events_path,
        swap_events_path,
        mint_events_path,
        burn_events_path,
        collect_events_path,
        pool_created_events_path,
    };

    let pool_analyzer = PoolAnalyzer::initialize(PoolAnalyzerConfig {
        http_url,
        fork_block,
        uniswap_v3_factory_address,
        uniswap_v3_position_manager_address,
        uniswap_v3_swap_router_address,
        weth_address,
        config: csv_reader_config,
    })
    .await?;

    pool_analyzer.run_simulation().await?;

    info!("Pool analysis complete");

    Ok(())
}
