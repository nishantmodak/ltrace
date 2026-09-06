#!/usr/bin/env python3
"""Send two explicitly synthetic OTLP traces to the local ltrace app.

No dependencies or external services. Times describe a designed example, not
measured application performance. Run: python3 scripts/demo_traces.py
"""
import argparse
import json
import time
import urllib.request
import uuid
from collections import defaultdict


def examples():
    base = time.time_ns() - 3_000_000_000
    services = defaultdict(list)
    counts = {}

    def trace(name, offset):
        trace_id = uuid.uuid4().hex
        count = 0

        def span(operation, service, start, end, parent=None, error=False, **attrs):
            nonlocal count
            count += 1
            span_id = f"{count:016x}"
            attributes = {"demo.synthetic": True, "demo.scenario": name, **attrs}
            values = []
            for key, value in attributes.items():
                kind = "boolValue" if isinstance(value, bool) else "intValue" if isinstance(value, int) else "stringValue"
                values.append({"key": key, "value": {kind: str(value) if kind == "intValue" else value}})
            body = {"traceId": trace_id, "spanId": span_id, "name": operation,
                    "kind": 1, "startTimeUnixNano": str(base + offset + start * 1_000_000),
                    "endTimeUnixNano": str(base + offset + end * 1_000_000),
                    "attributes": values, "status": {"code": 2 if error else 1}}
            if parent:
                body["parentSpanId"] = parent
            if error:
                body["status"]["message"] = "Simulated payment provider timeout"
                body["events"] = [{"name": "exception", "timeUnixNano": body["endTimeUnixNano"], "attributes": [{"key": "exception.message", "value": {"stringValue": "Synthetic timeout; next attempt succeeds"}}]}]
            services[service].append(body)
            counts[trace_id] = {"name": name, "spans": count}
            return span_id
        return span

    small = trace("Small · health check", 0)
    root = small("Demo · Health check", "demo-api", 0, 24, **{"http.route": "/health"})
    small("cache.ping", "demo-api", 2, 7, root)
    small("db.ping", "demo-api", 9, 21, root, **{"db.system.name": "postgresql"})

    span = trace("Complex · checkout", 1_000_000_000)
    root = span("Demo · Checkout", "demo-api", 0, 1200, **{"http.route": "/checkout", "demo.expected_query_count": 12})
    span("auth.validate", "demo-api", 2, 35, root)
    cart = span("GET /cart", "demo-api", 45, 440, root)
    cart_server = span("cart.load", "demo-cart", 50, 425, cart)
    span("cache.get · miss", "demo-cart", 55, 67, cart_server, **{"cache.hit": False})
    items = span("cart.load_items", "demo-cart", 75, 400, cart_server)
    for i in range(12):
        span("SELECT catalog item", "demo-cart", 80 + i * 25, 96 + i * 25, items,
             **{"db.system.name": "postgresql", "db.query.text": "SELECT name, price FROM items WHERE id = ?", "demo.item_index": i})
    stock = span("POST /inventory/check", "demo-api", 45, 340, root)
    stock_server = span("inventory.check", "demo-inventory", 50, 330, stock)
    span("SELECT stock batch", "demo-inventory", 70, 240, stock_server, **{"db.system.name": "postgresql"})
    span("shipping.quote", "demo-api", 45, 200, root)
    payment = span("payment.authorize", "demo-api", 450, 1020, root)
    first = span("POST /charge · attempt 1", "demo-api", 460, 630, payment, True, **{"retry.attempt": 1, "http.response.status_code": 504})
    span("charge · timeout", "demo-payments", 465, 625, first, True)
    second = span("POST /charge · attempt 2", "demo-api", 730, 1010, payment, **{"retry.attempt": 2, "http.response.status_code": 200})
    charge = span("charge · approved", "demo-payments", 735, 1000, second)
    span("db.insert_payment", "demo-payments", 900, 970, charge)
    span("db.insert_order", "demo-api", 1030, 1090, root)
    span("notification.publish", "demo-api", 1100, 1170, root)
    body = {"resourceSpans": [{"resource": {"attributes": [{"key": "service.name", "value": {"stringValue": service}}]},
             "scopeSpans": [{"scope": {"name": "ltrace.synthetic-demo", "version": "1"}, "spans": spans}]} for service, spans in services.items()]}
    return body, counts


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--port", type=int, default=4318)
    args = parser.parse_args()
    if not 1 <= args.port <= 65535:
        parser.error("port must be between 1 and 65535")
    body, counts = examples()
    request = urllib.request.Request(f"http://127.0.0.1:{args.port}/v1/traces", data=json.dumps(body).encode(), headers={"Content-Type": "application/json"})
    opener = urllib.request.build_opener(urllib.request.ProxyHandler({}))
    with opener.open(request, timeout=10) as response:
        result = json.load(response)
    rejected = result.get("partialSuccess", {}).get("rejectedSpans", "0")
    if int(rejected):
        raise RuntimeError(f"Receiver rejected demo spans: {result}")
    print(json.dumps({"synthetic": True, "traces": counts}, indent=2))


if __name__ == "__main__":
    main()
