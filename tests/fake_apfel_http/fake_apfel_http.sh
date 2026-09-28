#!/usr/bin/env bash
# Fake apfel HTTP server for the cold-start elimination measurement test
# (issue #772).
#
# Simulates a persistent apfel server (`apfel --serve`) by listening on a
# TCP port and answering:
#   GET /health          → 200 with apfel-shaped JSON
#   POST /v1/chat/completions → 200 with an OpenAI-style chat completion
#
# The chat-completions response contains N `#n: summary_for_n` lines where
# N is the number of `--- Function: #` entries in the request body. This
# mirrors the real server's positional response shape.
#
# Usage:
#   ./fake_apfel_http.sh <port>
#
# The server runs until killed. It is designed for the measurement test in
# `apfel_server_tests.rs` which starts the server, runs `batch_summarize`
# against it, records the wall-clock, then kills the server.
#
# Note: this script is NOT gitignored (unlike tests/fixtures/*) because it
# lives under tests/fake_apfel_http/ (not tests/fixtures/).

set -euo pipefail

if [ $# -lt 1 ]; then
    echo "usage: $0 <port>" >&2
    exit 1
fi

PORT="$1"

# Build the health response body (matches apfel's /health shape).
HEALTH_BODY='{"active_requests":0,"context_window":4096,"model":"apple-foundationmodel","model_available":true,"prewarmed":true,"status":"ok","version":"1.9.1"}'

# Python one-liner that acts as the HTTP server. It reads the request,
# counts `--- Function: #` entries in the body, and responds with N
# `#n: summary_for_n` lines.
exec python3 -c "
import http.server, json, re, sys

PORT = int(sys.argv[1])

class Handler(http.server.BaseHTTPRequestHandler):
    def do_GET(self):
        if self.path == '/health':
            body = $HEALTH_BODY.encode()
            self.send_response(200)
            self.send_header('Content-Type', 'application/json')
            self.send_header('Content-Length', str(len(body)))
            self.end_headers()
            self.wfile.write(body)
        else:
            self.send_response(404)
            self.send_header('Content-Length', '2')
            self.end_headers()
            self.wfile.write(b'{}')

    def do_POST(self):
        length = int(self.headers.get('Content-Length', 0))
        body = self.rfile.read(length).decode()
        # Count `--- Function: #` entries in the request body.
        n = len(re.findall(r'--- Function: #\d+', body))
        if n == 0:
            n = 1  # fallback: at least one summary
        content = '\\n'.join(f'#{i}: summary_for_{i}' for i in range(n))
        response = json.dumps({
            'choices': [{'message': {'content': content, 'role': 'assistant'}}],
            'object': 'chat.completion',
        })
        resp_bytes = response.encode()
        self.send_response(200)
        self.send_header('Content-Type', 'application/json')
        self.send_header('Content-Length', str(len(resp_bytes)))
        self.end_headers()
        self.wfile.write(resp_bytes)

    def log_message(self, format, *args):
        pass  # suppress access logs

server = http.server.HTTPServer(('127.0.0.1', PORT), Handler)
server.serve_forever()
" "$PORT"
