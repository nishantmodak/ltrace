"""Batch item lookups while preserving the caller's order and duplicates."""
import sqlite3
from opentelemetry import trace

tracer = trace.get_tracer("ltrace.catalog")


def load_items(connection: sqlite3.Connection, item_ids: list[int]) -> list[dict]:
    with tracer.start_as_current_span("catalog.load_items") as root:
        root.set_attribute("catalog.item_count", len(item_ids))
        if not item_ids:
            return []
        unique_ids = list(dict.fromkeys(item_ids))
        with tracer.start_as_current_span("db.lookup") as query:
            query.set_attribute("db.system.name", "sqlite")
            query.set_attribute("db.operation.name", "SELECT")
            query.set_attribute("code.function.name", "load_items")
            query.set_attribute("code.file.path", __file__)
            placeholders = ",".join("?" for _ in unique_ids)
            rows = connection.execute(
                f"SELECT id, name FROM items WHERE id IN ({placeholders})", unique_ids
            ).fetchall()
        names = dict(rows)
        return [{"id": item_id, "name": names[item_id]} for item_id in item_ids if item_id in names]
