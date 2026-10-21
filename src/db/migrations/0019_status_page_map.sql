-- Status pages can embed the live map of their servers (opt-in).
ALTER TABLE status_pages ADD COLUMN show_map INTEGER NOT NULL DEFAULT 0;
