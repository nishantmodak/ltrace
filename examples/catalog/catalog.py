"""A deliberately repeated-query implementation for the first capture."""
import sqlite3
from opentelemetry import trace

tracer = trace.get_tracer("ltrace.catalog")


def load_items(connection: sqlite3.Connection, item_ids: list[int]) -> list[dict]:
    with tracer.start_as_current_span("catalog.load_items") as root:
        root.set_attribute("catalog.item_count", len(item_ids))
        items = []
        for item_id in item_ids:
            with tracer.start_as_current_span("db.lookup") as query:
                query.set_attribute("db.system.name", "sqlite")
                query.set_attribute("db.operation.name", "SELECT")
                query.set_attribute("code.function.name", "load_items")
                query.set_attribute("code.file.path", __file__)
                row = connection.execute(
                    "SELECT id, name FROM items WHERE id = ?", (item_id,)
                ).fetchone()
                if row:
                    items.append({"id": row[0], "name": row[1]})
        return items
