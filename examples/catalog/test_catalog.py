"""Real SQLite queries, real SDK exports, and independent output assertions."""
import os
import sqlite3
import unittest

from opentelemetry import trace
from opentelemetry.sdk.resources import Resource
from opentelemetry.sdk.trace import TracerProvider
from opentelemetry.sdk.trace.export import SimpleSpanProcessor
from opentelemetry.exporter.otlp.proto.http.trace_exporter import OTLPSpanExporter
from catalog import load_items

provider = None


def setUpModule():
    global provider
    # Test-local opt-in. Running normal tests never creates a network exporter.
    if os.environ.get("LTRACE_RUN_ID"):
        provider = TracerProvider(resource=Resource.create({"service.name": "catalog-example"}))
        provider.add_span_processor(SimpleSpanProcessor(OTLPSpanExporter(timeout=5)))
        trace.set_tracer_provider(provider)


def tearDownModule():
    if provider:
        provider.force_flush(timeout_millis=5000)
        provider.shutdown()


class CatalogTest(unittest.TestCase):
    def setUp(self):
        self.connection = sqlite3.connect(":memory:")
        self.addCleanup(self.connection.close)
        self.connection.execute("CREATE TABLE items (id INTEGER PRIMARY KEY, name TEXT)")
        self.connection.executemany("INSERT INTO items VALUES (?, ?)", [(i, f"Item {i}") for i in range(1, 13)])

    def test_load_twelve_items(self):
        actual = load_items(self.connection, list(range(1, 13)))
        self.assertEqual(actual, [{"id": i, "name": f"Item {i}"} for i in range(1, 13)])

    def test_order_duplicates_and_missing_items(self):
        self.assertEqual(load_items(self.connection, [3, 1, 3, 99]), [
            {"id": 3, "name": "Item 3"}, {"id": 1, "name": "Item 1"}, {"id": 3, "name": "Item 3"}
        ])

    def test_empty_input(self):
        self.assertEqual(load_items(self.connection, []), [])


if __name__ == "__main__":
    unittest.main()
