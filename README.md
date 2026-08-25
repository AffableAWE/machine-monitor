# Machine Monitor

A small Rust program that processes unreliable temperature, vibration, and power readings and reports the machine state at each timestamp.

## Run

```bash
cargo run -- sample.csv
```

To read from standard input:

```bash
cat sample.csv | cargo run
```

Run the tests with:

```bash
cargo test
```

## Input

Each CSV line contains:

```text
message_id,timestamp,sensor,value
m1,1000,power,6.2
```

Blank lines and lines beginning with `#` are ignored. Malformed lines are reported and skipped.

## Workflow

1. Validate the four message fields.
2. Reject duplicate message IDs.
3. Buffer and reorder messages using a watermark. Messages arriving after their timestamp was finalized are dropped.
4. Ignore stale readings and classify the machine as `IDLE`, `STARTING`, `RUNNING`, `WARNING`, or `UNKNOWN`.

In one sentence: read a line, validate it, remove duplicates, restore timestamp order, update the latest sensor values, discard stale information, determine the state, and print one result per timestamp.

## MVP assumptions

- Messages may arrive up to 5 timestamp-seconds out of order.
- Sensor readings become stale after 30 seconds.
- Message IDs uniquely identify messages.
- Thresholds are illustrative and would be configurable in production.

More detailed reasoning, trade-offs, limitations, and assistance disclosure are provided in the accompanying **My Thought Process and Declaration** document.
