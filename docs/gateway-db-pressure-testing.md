# Aether Gateway DB pressure testing

目标：验证 6k+ 并发长连接/流式请求时，gateway 不把请求并发线性放大成 DB 连接并发，并确认 DB pool、usage queue、后台维护任务不会成为瓶颈。

## 1. 预设环境

生产/压测环境建议显式设置：

```bash
export AETHER_GATEWAY_DATA_POSTGRES_MAX_CONNECTIONS=80
export AETHER_GATEWAY_DATA_POSTGRES_MIN_CONNECTIONS=12
export AETHER_GATEWAY_MAINTENANCE_POOL_IDLE_RESERVE=8

export AETHER_GATEWAY_USAGE_QUEUE_TERMINAL_EVENTS=true
export AETHER_GATEWAY_USAGE_QUEUE_LIFECYCLE_EVENTS=true
export AETHER_GATEWAY_USAGE_QUEUE_STREAM_MAXLEN=200000
export AETHER_GATEWAY_USAGE_QUEUE_BATCH_SIZE=500
export AETHER_GATEWAY_USAGE_QUEUE_RECLAIM_COUNT=500

# Pool score DB feedback is rate-limited per provider key to avoid one
# synchronous score UPDATE per successful request.
export AETHER_GATEWAY_POOL_SCORE_SUCCESS_FEEDBACK_MIN_INTERVAL_SECS=5
export AETHER_GATEWAY_POOL_SCORE_FAILURE_FEEDBACK_MIN_INTERVAL_SECS=1

# Invalid API keys are cached briefly so repeated bad credentials cannot
# linearly amplify into DB lookups. Set 0 to disable during auth debugging.
export AETHER_GATEWAY_AUTH_CONTEXT_NEGATIVE_CACHE_TTL_SECS=10
```

SQLite 不适合 6k 并发压测；请使用 Postgres/MySQL 和 Redis runtime backend。

## 2. Gateway HTTP 压测

准备请求体：

```bash
cat >/tmp/aether-pressure-request.json <<'JSON'
{"model":"gpt-5-mini","messages":[{"role":"user","content":"ping"}],"stream":true}
JSON
```

运行 6k 并发（高并发结论必须使用 release；debug 构建会把 CPU/调试开销误判成 planning timeout）：

```bash
TARGET_URL=http://127.0.0.1:18080/v1/chat/completions \
METRICS_URL=http://127.0.0.1:18080/_gateway/metrics \
PRESSURE_METHOD=POST \
PRESSURE_REQUESTS=60000 \
PRESSURE_CONCURRENCY=6000 \
PRESSURE_TIMEOUT_MS=120000 \
PRESSURE_BODY_FILE=/tmp/aether-pressure-request.json \
AUTH_HEADER='Authorization: Bearer <api-key>' \
EXTRA_HEADERS='Content-Type: application/json' \
PRESSURE_RESPONSE_MODE=full \
PRESSURE_CARGO_PROFILE=release \
OUTPUT=/tmp/aether_gateway_pressure_6k.json \
tools/pressure/run_gateway_6k_pressure.sh
```

如果压测流式长连接，建议让 probe 读完整响应体，否则客户端拿到 headers 后会立刻断开：

```bash
PRESSURE_RESPONSE_MODE=full
```

## 3. 本地 mock upstream

没有可承载 6k 并发的真实上游 key 时，先用 testkit 启一个 OpenAI-compatible mock upstream：

```bash
cargo run --release -p aether-testkit --bin mock_openai_upstream -- \
  --bind 127.0.0.1:18181 \
  --chunks 8 \
  --first-byte-delay-ms 0 \
  --chunk-delay-ms 20 \
  --payload-bytes 32
```

可直接压 mock 网络栈：

```bash
cat >/tmp/aether-mock-request.json <<'JSON'
{"model":"mock-model","messages":[{"role":"user","content":"ping"}],"stream":true}
JSON

cargo run --release -p aether-testkit --bin http_load_probe -- \
  --url http://127.0.0.1:18181/v1/chat/completions \
  --method POST \
  --requests 60000 \
  --concurrency 6000 \
  --timeout-ms 120000 \
  --header 'Content-Type: application/json' \
  --body-file /tmp/aether-mock-request.json \
  --response-mode full
```

要做 gateway 端到端压测，把一个本地压测 provider/endpoint 指到
`http://127.0.0.1:18181/v1`，provider key 用 dummy 值即可；gateway 侧仍需一个
本地 Aether API key，但不再消耗真实上游额度。

报告重点字段：

```json
{
  "load": {
    "throughput_rps": 0,
    "failed_requests": 0,
    "error_counts": {},
    "p95_ms": 0,
    "p99_ms": 0
  },
  "metrics": {
    "db_pool_max_checked_out": 0,
    "db_pool_min_idle": 0,
    "db_pool_max_usage_basis_points": 0,
    "db_pool_pressure_samples": 0,
    "gateway_requests_max_rejected_total": 0
  }
}
```


### 本地全链路 release 基线（mock upstream）

本地全链路压测固定为：release gateway + release mock upstream + 本地 Postgres/Redis + 临时 Aether API key/provider/model。
不要把 6k 长连接理解为 6k DB 连接；目标是 6k 前端连接下 DB pool 维持在几十级。

最近可复现基线：

| 场景 | 结果 | throughput | p50 | p95 | p99 | DB pool | mock in-flight |
| --- | --- | ---: | ---: | ---: | ---: | --- | ---: |
| 1000 req / 100 conc | 1000x 200 / 0 fail | 211 rps | 209ms | 729ms | 1171ms | max checked out 33/48 | - |
| 6000 req / 6000 conc | 6000x 200 / 0 fail, `error_counts={}` | 262 rps | 20198ms | 22496ms | 22748ms | max checked out 48/48, pressure samples 7 | 396 |

6000/6000 下实际打开约 6k FD，说明客户端长连接链路有效。剩余主要同步 DB 写入为每成功请求 1 条 terminal `request_candidates`；如需测纯转发极限，可临时把 `AETHER_GATEWAY_REQUEST_CANDIDATE_PERSISTENCE=none` 与 terminal 模式对照。

常用本地命令（不要打印 API key）：

```bash
source /tmp/aether_local_env.sh
KEY=$(cat /tmp/aether_fullchain_api_key)
CARGO_TARGET_DIR=/tmp/aether-release-pressure \
TARGET_URL=http://127.0.0.1:8088/v1/chat/completions \
METRICS_URL=http://127.0.0.1:8088/_gateway/metrics \
PRESSURE_METHOD=POST \
PRESSURE_REQUESTS=6000 \
PRESSURE_CONCURRENCY=6000 \
PRESSURE_TIMEOUT_MS=120000 \
PRESSURE_SAMPLE_INTERVAL_MS=250 \
PRESSURE_BODY_FILE=/tmp/aether-mock-request.json \
PRESSURE_RESPONSE_MODE=full \
PRESSURE_CARGO_PROFILE=release \
AUTH_HEADER="Authorization: Bearer ${KEY}" \
EXTRA_HEADERS='Content-Type: application/json' \
OUTPUT=/tmp/aether_gateway_release_6000_6000_full.json \
tools/pressure/run_gateway_6k_pressure.sh
```

```bash
docker compose exec -T -e PGPASSWORD="$DB_PASSWORD" postgres psql -U postgres -d aether -P pager=off -c "
SELECT calls, round(total_exec_time::numeric,1) total_ms,
       round(mean_exec_time::numeric,3) mean_ms, rows,
       left(regexp_replace(query, '\s+', ' ', 'g'), 280) query
FROM pg_stat_statements
ORDER BY calls DESC
LIMIT 60;"
```

## 4. 判定标准

优先看这些信号：

- `failed_requests == 0` 或仅包含预期的上游错误。
- `db_pool_max_checked_out` 不应接近 `AETHER_GATEWAY_DATA_POSTGRES_MAX_CONNECTIONS`。
- `db_pool_min_idle` 在大部分采样中应高于 idle reserve。
- `db_pool_max_usage_basis_points < 8000` 比较健康；持续 `>9000` 表示 DB pool 或 SQL 写入已接近瓶颈。
- `db_pool_pressure_samples` 可以短暂出现，但不应贯穿压测全程。
- `gateway_requests_max_rejected_total` 不应增长，除非有意测试 admission limit。

## 5. DB 热点写入专项压测

这些 testkit 场景会启动临时 Postgres，适合回归验证 counter/settlement 热点锁竞争：

```bash
cargo run -p aether-testkit --bin usage_counter_hotspot_baseline -- \
  --requests 20000 --concurrency 1000 \
  --flush-interval-ms 50 --monitor-interval-ms 20 \
  --output /tmp/aether_usage_counter_20000_1000.json

cargo run -p aether-testkit --bin usage_settlement_hotspot_baseline -- \
  --requests 20000 --concurrency 1000 \
  --flush-interval-ms 50 --monitor-interval-ms 20 \
  --output /tmp/aether_usage_settlement_20000_1000.json

cargo run -p aether-testkit --bin usage_aux_counter_hotspot_baseline -- \
  --requests 20000 --concurrency 1000 \
  --flush-interval-ms 50 --monitor-interval-ms 20 \
  --output /tmp/aether_usage_aux_counter_20000_1000.json
```

关注：

- `failed_requests`
- `throughput_rps`
- `p95_ms`
- `lock_monitor.max_*_update_waiters`
- `lock_monitor.max_oldest_lock_wait_ms`

定向 update waiter 持续大于 0，说明某类 counter/settlement 仍有热点行锁竞争，需要继续分桶或延迟聚合。

## 6. 本地回归基线

最近一次本地临时 Postgres 回归（`requests=60000, concurrency=6000, max_connections=64`）：

| suite | throughput | failed | p95 | 定向 update waiters |
| --- | ---: | ---: | ---: | ---: |
| `usage_settlement_hotspot_baseline` | 5452 rps | 0 | 740ms | usage 10 / wallet 0 / provider 0 |
| `usage_counter_hotspot_baseline` | 1358 rps | 0 | 1865ms | api_key 0 / provider_key 0 / model 0 / provider 0 |
| `usage_aux_counter_hotspot_baseline` | 1899 rps | 0 | 785ms | proxy 0 / management_token 0 / api_key 0 |

这些是 DB 热点写入专项结果，不等价于完整 gateway 端到端 6k 流式压测；完整压测仍需使用第 2 节的 gateway URL、真实 API key、Redis runtime backend 与目标模型请求体。
