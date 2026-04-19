import http from 'k6/http';
import { check, sleep } from 'k6';
import { Rate, Trend, Counter } from 'k6/metrics';

const errorRate      = new Rate('error_rate');
const clickDuration  = new Trend('click_duration', true);
const syncDuration   = new Trend('sync_duration',  true);
const dataDuration   = new Trend('data_duration',  true);
const blockedCounter = new Counter('rate_limit_blocked');

const BASE_URL = __ENV.BASE_URL || 'http://localhost:3719';

// 6000 токенов для 5000 VU с запасом
const TOKENS = Array.from({ length: 6000 }, (_, i) => `test-token-vu-${i + 1}`);

export const options = {
  scenarios: {
    ramp_up: {
      executor: 'ramping-vus',
      startVUs: 0,
      stages: [
        { duration: '30s', target: 500  },
        { duration: '60s', target: 2000 },
        { duration: '60s', target: 5000 },
        { duration: '60s', target: 5000 }, // держим пик
        { duration: '30s', target: 1000 },
        { duration: '15s', target: 0    },
      ],
      gracefulRampDown: '15s',
    },
  },

  thresholds: {
    http_req_duration: ['p(95)<500', 'p(99)<1500'],
    // rate-limit (429/403) ожидаем, поэтому порог высокий
    http_req_failed:   ['rate<0.95'],
    // реальные ошибки приложения — строгий порог
    error_rate:        ['rate<0.05'],
    click_duration:    ['p(95)<500'],
    sync_duration:     ['p(95)<500'],
    data_duration:     ['p(95)<500'],
  },
};

function getToken() {
  return TOKENS[(__VU - 1) % TOKENS.length];
}

function makeHeaders(token) {
  return {
    'Authorization': `Bearer ${token}`,
    'Content-Type': 'application/json',
  };
}

function isRateLimited(status) {
  return status === 429 || status === 403;
}

export default function () {
  const token = getToken();
  const headers = makeHeaders(token);

  // 1. GET /api/data
  {
    const res = http.get(`${BASE_URL}/api/data`, {
      headers,
      tags: { name: 'GET /api/data' },
    });

    dataDuration.add(res.timings.duration);

    if (isRateLimited(res.status)) {
      blockedCounter.add(1);
      sleep(randomBetween(0.5, 1.5));
      return;
    }

    const ok = check(res, {
      'data: status 200':    (r) => r.status === 200,
      'data: has balance':   (r) => r.json('balance') !== undefined,
      'data: has per_click': (r) => r.json('per_click') !== undefined,
    });
    errorRate.add(ok ? 0 : 1);
  }

  // Rate limit 500ms — спим чуть больше
  sleep(randomBetween(0.6, 1.5));

  // 2. POST /api/click
  {
    const clicks = Math.floor(Math.random() * 100) + 1;
    const res = http.post(
      `${BASE_URL}/api/click`,
      JSON.stringify({ data: clicks }),
      { headers, tags: { name: 'POST /api/click' } }
    );

    clickDuration.add(res.timings.duration);

    if (isRateLimited(res.status)) {
      blockedCounter.add(1);
      sleep(randomBetween(0.5, 1.5));
      return;
    }

    const ok = check(res, {
      'click: status 200':        (r) => r.status === 200,
      'click: has balance':       (r) => r.json('balance') !== undefined,
      'click: balance is number': (r) => typeof r.json('balance') === 'number',
      'click: balance >= 0':      (r) => r.json('balance') >= 0,
    });
    errorRate.add(ok ? 0 : 1);
  }

  sleep(randomBetween(0.6, 1.5));

  // 3. POST /api/sync
  {
    const res = http.post(
      `${BASE_URL}/api/sync`,
      null,
      { headers, tags: { name: 'POST /api/sync' } }
    );

    syncDuration.add(res.timings.duration);

    if (isRateLimited(res.status)) {
      blockedCounter.add(1);
      sleep(randomBetween(0.5, 1.5));
      return;
    }

    const ok = check(res, {
      'sync: status 200':    (r) => r.status === 200,
      'sync: has balance':   (r) => r.json('balance') !== undefined,
      'sync: has last_sync': (r) => r.json('last_sync') !== undefined,
    });
    errorRate.add(ok ? 0 : 1);
  }

  sleep(randomBetween(0.6, 1.5));
}

function randomBetween(min, max) {
  return Math.random() * (max - min) + min;
}

export function handleSummary(data) {
  const rps      = data.metrics.http_reqs?.values?.rate?.toFixed(1)           ?? 'N/A';
  const p95      = data.metrics.http_req_duration?.values['p(95)']?.toFixed(1) ?? 'N/A';
  const p99      = data.metrics.http_req_duration?.values['p(99)']?.toFixed(1) ?? 'N/A';
  const errors   = ((data.metrics.error_rate?.values?.rate ?? 0) * 100).toFixed(2);
  const blocked  = data.metrics.rate_limit_blocked?.values?.count ?? 0;
  const total    = data.metrics.http_reqs?.values?.count ?? 0;
  const blockedPct = total > 0 ? ((blocked / total) * 100).toFixed(1) : '0';

  const summary = `
========================================
        K6 LOAD TEST — SUMMARY
========================================
  VUs (max):          5000
  RPS:                ${rps} req/s
  Total requests:     ${total}

  Latency p(95):      ${p95} ms
  Latency p(99):      ${p99} ms

  App error rate:     ${errors}%   (исключая rate-limit)
  Rate-limited 429:   ${blocked} requests (${blockedPct}% от всех)

  Click  p(95):       ${data.metrics.click_duration?.values['p(95)']?.toFixed(1) ?? 'N/A'} ms
  Sync   p(95):       ${data.metrics.sync_duration?.values['p(95)']?.toFixed(1)  ?? 'N/A'} ms
  Data   p(95):       ${data.metrics.data_duration?.values['p(95)']?.toFixed(1)  ?? 'N/A'} ms
========================================
`;

  console.log(summary);
}