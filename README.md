# uniswap_v3_analyze_fees

Note: this repo is a work in progress and is not finished yet

This repo analyzes which LP positions are making the most fees on a target Uniswap V3 pool using historial data about activity on the pool.

The example data in the `example_pool_data` folder is from the [`based_fartcoin` pool](https://basescan.org/token/0x2f6c17fa9f9bc3600346ab4e48c0701e1d5962ae?a=0xfdbaf04326acc24e3d1788333826b71e3291863a) on Base. Similar data can be found by querying Dune like such:

```sql
-- For uniswap v3 pool events
SELECT *
FROM uniswap_v3_base.UniswapV3Pool_evt_Initialize
WHERE contract_address = 0xFdbAf04326AcC24e3d1788333826b71E3291863a ORDER BY (evt_block_number, evt_index);

-- For uniswap v3 factory events
SELECT *
FROM uniswap_v3_base.UniswapV3Factory_evt_PoolCreated
WHERE pool = 0xFdbAf04326AcC24e3d1788333826b71E3291863a ORDER BY (evt_block_number, evt_index);
```
The default Dune column names and ordering is assumed by the program, so if you want to use a different csv format you will need to modify the code.

## Usage

```bash
## Copy env file and fill in the values
just copy-env 

## Run the program
just run
```
