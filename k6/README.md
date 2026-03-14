# k6 scripts

These scripts are included as bonus artifacts for load testing the hot paths.

## Environment

- `BASE_URL` — defaults to `http://localhost:8080`
- `AUTH_TOKEN` — defaults to `tok_user_1` for authenticated write traffic

## Examples

```bash
k6 run -e BASE_URL=http://localhost:8080 k6/count_read_10k_rps.js
k6 run -e BASE_URL=http://localhost:8080 k6/batch_counts_1k_rps.js
k6 run -e BASE_URL=http://localhost:8080 -e AUTH_TOKEN=tok_user_1 k6/mixed_traffic.js
```

These scripts are meant to be practical starting points rather than perfect benchmark proofs. Laptop numbers will vary based on Docker networking, CPU limits, and whether Redis/Postgres are sharing the same machine.
