import http from 'k6/http';
import { check, sleep } from 'k6';

const BASE_URL = __ENV.BASE_URL || 'http://localhost:8080';

const ids = [
  '731b0395-4888-4822-b516-05b4b7bf2089',
  '9601c044-6130-4ee5-a155-96570e05a02f',
  '933dde0f-4744-4a66-9a38-bf5cb1f67553',
  'ec53f1d5-fc7c-4d70-bf6f-f01fbb4d3f77',
  '8e2e4a90-6f68-4e14-a6e0-8e9fb9d6a110',
];

function buildItems() {
  const items = [];
  for (let i = 0; i < 50; i++) {
    items.push({ content_type: 'post', content_id: ids[i % ids.length] });
  }
  return items;
}

export const options = {
  scenarios: {
    batch_counts: {
      executor: 'constant-arrival-rate',
      rate: 1000,
      timeUnit: '1s',
      duration: '30s',
      preAllocatedVUs: 100,
      maxVUs: 500,
    },
  },
  thresholds: {
    http_req_failed: ['rate<0.01'],
    http_req_duration: ['p(99)<50'],
  },
};

export default function () {
  const payload = JSON.stringify({ items: buildItems() });
  const res = http.post(`${BASE_URL}/v1/likes/batch/counts`, payload, {
    headers: { 'Content-Type': 'application/json' },
  });

  check(res, {
    'batch status 200': (r) => r.status === 200,
  });
}
