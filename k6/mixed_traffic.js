import http from 'k6/http';
import { check, sleep } from 'k6';

const BASE_URL = __ENV.BASE_URL || 'http://localhost:8080';
const AUTH_TOKEN = __ENV.AUTH_TOKEN || 'tok_user_1';
const CONTENT_ID = '731b0395-4888-4822-b516-05b4b7bf2089';
const ALT_CONTENT_ID = '9601c044-6130-4ee5-a155-96570e05a02f';

export const options = {
  scenarios: {
    mixed: {
      executor: 'constant-vus',
      vus: 100,
      duration: '30s',
    },
  },
  thresholds: {
    http_req_failed: ['rate<0.02'],
    http_req_duration: ['p(95)<100'],
  },
};

function batchBody() {
  return JSON.stringify({
    items: [
      { content_type: 'post', content_id: CONTENT_ID },
      { content_type: 'post', content_id: ALT_CONTENT_ID },
    ],
  });
}

export default function () {
  const choice = Math.random();

  if (choice < 0.80) {
    const res = http.get(`${BASE_URL}/v1/likes/post/${CONTENT_ID}/count`);
    check(res, { 'read 200': (r) => r.status === 200 });
  } else if (choice < 0.95) {
    const res = http.post(`${BASE_URL}/v1/likes/batch/counts`, batchBody(), {
      headers: { 'Content-Type': 'application/json' },
    });
    check(res, { 'batch 200': (r) => r.status === 200 });
  } else {
    const res = http.post(
      `${BASE_URL}/v1/likes`,
      JSON.stringify({ content_type: 'post', content_id: CONTENT_ID }),
      {
        headers: {
          'Content-Type': 'application/json',
          Authorization: `Bearer ${AUTH_TOKEN}`,
        },
      },
    );
    check(res, { 'write created': (r) => r.status === 201 || r.status === 429 });
  }
}
