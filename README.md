# uniswap_v3_analyze_fees

Note: this repo is a work in progress and is not finished yet

This repo analyzes which LP positions are making the most fees on a target Uniswap V3 pool using historial event and transaction data about activity on the pool.

### Progress
- [x] Reads in historical data from csv files into memory
- [x] Parses the data into a format that can be used by the program
- [x] Forks the http endpoint's target chain at the target block number and connects to it with Anvil (Uniswap V3 Factory and Weth need to be deployed by this block number)
- [X] Simulates the pool's activity on the forked endpoint
- [X] Calculates the fees earned by each LP position
- [ ] Outputs the results into CSV file for further analysis

The output of this program is a CSV file with the following information:
```
Position Info:
├─ Token ID:                  1487610
├─ Token Action Index:        4
├─ Action Taken:              IncreaseLiquidity
├─ Lower Tick:                -887200
├─ Upper Tick:                887200
├─ Opening info:
│  ├─ Block In:                  23811582
│  ├─ Token Amount In:           208168263364547375450278112
│  ├─ WETH Amount In:            282484755239206530
│  ├─ SqrtPriceLimitX96 In:      7518598854285689184029296
│  ├─ Tick In:                   -185264
│  ├─ Liquidity In:              7618565952586391586621
├─ Closing info:
│  ├─ Block Out:                 23849520
│  ├─ Token Amount Out:          98149514122120427525956866
│  ├─ WETH Amount Out:           591368665378112390
│  ├─ SqrtPriceLimitX96 Out:     6149851956130248128746138
│  └─ Tick Out:                   -189283
├─ Position PNL ---
│  token fees earned:                   99877808608147915287192192
│  weth fees earned:                    601160642580099050
│  net token gain (if position closed): -10140940634279032637129054
│  net weth gain (if position closed):  910044552719004910
│  approx starting weth:  2102874230578705641
│  approx ending weth:    2356092089989234879
└─ net pnl in weth:       253217859410529238
```

The program treats each position modification (open, increase liquidity, decrease liquidity) as a separate position for the purposes of calculating fees earned and position PNL. The index plus token ID can show the history of actions on the position (e.g. 1487610, 4 is the 4th action taken on position 1487610, and the action was an increase in liquidity).

### Expected Data format
The example data in the `example_pool_data` folder is from the [`based_fartcoin` pool](https://basescan.org/token/0x2f6c17fa9f9bc3600346ab4e48c0701e1d5962ae?a=0xfdbaf04326acc24e3d1788333826b71e3291863a) on Base. Similar data can be found by querying Dune like such:

```sql
-- For uniswap v3 pool events
SELECT *
FROM uniswap_v3_base.UniswapV3Pool_evt_Initialize
WHERE contract_address = 0xFdbAf04326AcC24e3d1788333826b71E3291863a ORDER BY (evt_block_number, evt_index);

-- For uniswap v3 nonfungible position manager increaseLiquidity events (includes amount0Desired and amount1Desired as additional columns)
WITH token_calls AS (SELECT 
    call_tx_hash,
    output_tokenId,
    output_liquidity as liquidity,
    CAST(json_extract_scalar(params, '$.amount0Desired') AS varchar) as amount0Desired,
    CAST(json_extract_scalar(params, '$.amount1Desired') AS varchar) as amount1Desired
FROM uniswap_v3_base.nonfungiblepositionmanager_call_mint 
WHERE LOWER(json_extract_scalar(params, '$.token1')) = LOWER('0x4200000000000000000000000000000000000006') 
    AND LOWER(json_extract_scalar(params, '$.token0')) = LOWER('0x2f6c17fa9f9bC3600346ab4e48C0701e1d5962AE')
    AND call_success = true
    AND call_block_number <= 25601659

UNION ALL

SELECT 
    call_tx_hash,
    CAST(json_extract_scalar(params, '$.tokenId') AS uint256) as output_tokenId,
    output_liquidity as liquidity,
    CAST(json_extract_scalar(params, '$.amount0Desired') AS varchar) as amount0Desired,
    CAST(json_extract_scalar(params, '$.amount1Desired') AS varchar) as amount1Desired
FROM uniswap_v3_base.nonfungiblepositionmanager_call_increaseliquidity
WHERE call_success = true
    AND call_block_number <= 25601659
    AND CAST(json_extract_scalar(params, '$.tokenId') AS uint256) IN (
        SELECT output_tokenId 
        FROM uniswap_v3_base.nonfungiblepositionmanager_call_mint
        WHERE LOWER(json_extract_scalar(params, '$.token1')) = LOWER('0x4200000000000000000000000000000000000006') 
            AND LOWER(json_extract_scalar(params, '$.token0')) = LOWER('0x2f6c17fa9f9bC3600346ab4e48C0701e1d5962AE')
            AND call_success = true
            AND call_block_number <= 25601659
    )
)
SELECT il.*, m.amount0Desired, m.amount1Desired
FROM token_calls m
LEFT JOIN 
    uniswap_v3_base.nonfungiblepositionmanager_evt_increaseliquidity il 
    ON m.output_tokenId = il.tokenId
    AND m.call_tx_hash = il.evt_tx_hash
    AND m.liquidity = il.liquidity

-- For uniswap v3 nonfungible position manager decreaseLiquidity events (includes amount0Min and amount1Min as additional columns)
WITH base_mints AS (
    SELECT 
    output_tokenId
    FROM uniswap_v3_base.nonfungiblepositionmanager_call_mint 
    WHERE LOWER(json_extract_scalar(params, '$.token1')) = LOWER('0x4200000000000000000000000000000000000006') 
    AND LOWER(json_extract_scalar(params, '$.token0')) = LOWER('0x2f6c17fa9f9bC3600346ab4e48C0701e1d5962AE')
    AND call_success = true
)
SELECT 
    dl.*,
    dcl.amount0Min,
    dcl.amount1Min
FROM base_mints m
LEFT JOIN 
    uniswap_v3_base.nonfungiblepositionmanager_evt_decreaseliquidity dl
    ON m.output_tokenId = dl.tokenId
LEFT JOIN (
    SELECT 
        call_tx_hash as tx_hash,
        CAST(json_extract_scalar(params, '$.tokenId') AS uint256) as tokenId,
        CAST(json_extract_scalar(params, '$.liquidity') AS uint256) as liquidity,
        CAST(json_extract_scalar(params, '$.amount0Min') AS varchar) as amount0Min,
        CAST(json_extract_scalar(params, '$.amount1Min') AS varchar) as amount1Min
    FROM uniswap_v3_base.nonfungiblepositionmanager_call_decreaseliquidity
    WHERE call_success = true
) dcl 
    ON dl.evt_tx_hash = dcl.tx_hash 
    AND dl.tokenId = dcl.tokenId 
    AND dl.liquidity = dcl.liquidity

-- For uniswap v3 factory events
SELECT *
FROM uniswap_v3_base.UniswapV3Factory_evt_PoolCreated
WHERE pool = 0xFdbAf04326AcC24e3d1788333826b71E3291863a ORDER BY (evt_block_number, evt_index);
```
The default Dune decoded uniswap event column format is assumed by the program, so if you want to use a different csv format you will need to modify the code's parsing logic in `src/fee_analyzer/csv_converter.rs`. The increase_liquidity event has an additional `amount0Desired` and `amount1Desired` column that is not present in the default Dune decoded uniswap event column format, it's pulled from the transaction's function call params that Dune stores. 

Note: all queries should be restricted to the same max blocknumber or the program could fail. 

## Usage

```bash
## Copy env file and fill in the values
just copy-env 

## Run the program
just run
```