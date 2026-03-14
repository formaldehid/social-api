import http from 'k6/http';
import { check, sleep } from 'k6';

const BASE_URL = __ENV.BASE_URL || 'http://localhost:8080';
const CONTENT_ID = '731b0395-4888-4822-b516-05b4b7bf2089';

export const options = {
  scenarios: {
    count_reads: {
      executor: 'constant-arrival-rate',
      rate: 10000,
      timeUnit: '1s',
      duration: '30s',
      preAllocatedVUs: 200,
      maxVUs: 1000,
    },
  },
  thresholds: {
    http_req_failed: ['rate<0.01'],
    http_req_duration: ['p(99)<10'],
  },
};

export default function () {
  const res = http.get(`${BASE_URL}/v1/likes/post/${CONTENT_ID}/count`);
  check(res, {
    'count status 200': (r) => r.status === 200,
  });
}
